//! Bounded admission and independent durable writers; no thread per session.
use crate::store::{Envelope, Input, Store};
use anyhow::{Result, anyhow, ensure};
use std::{
    path::Path,
    sync::mpsc::{self, Receiver, SyncSender},
    thread::{self, JoinHandle},
};

type Reply = mpsc::Sender<Result<Vec<Envelope>>>;
enum Work {
    Apply(Input, Reply),
    Ack(String, Reply),
    Stop,
}
pub struct Runtime {
    workers: Vec<SyncSender<Work>>,
    handles: Vec<JoinHandle<()>>,
}
impl Runtime {
    pub fn open(dir: impl AsRef<Path>, shards: usize, queue: usize, capacity: u64) -> Result<Self> {
        ensure!(
            (1..=64).contains(&shards) && (1..=65536).contains(&queue),
            "invalid runtime limits"
        );
        std::fs::create_dir_all(dir.as_ref())?;
        // The stable shard count is part of the on-disk format, not a tuning knob.
        let manifest = dir.as_ref().join("shards");
        if manifest.exists() {
            ensure!(
                std::fs::read_to_string(&manifest)?.trim() == shards.to_string(),
                "shard count changed"
            );
        } else {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&manifest)?;
            write!(f, "{shards}")?;
            f.sync_all()?;
        }
        // Open all stores before spawning, so a lock failure leaves no orphan workers.
        let stores = (0..shards)
            .map(|n| Store::open(dir.as_ref().join(format!("shard-{n}.redb")), 60, capacity))
            .collect::<Result<Vec<_>>>()?;
        let mut workers = Vec::new();
        let mut handles = Vec::new();
        for store in stores {
            let (sender, receiver) = mpsc::sync_channel(queue);
            handles.push(thread::spawn(move || {
                while let Ok(work) = receiver.recv() {
                    match work {
                        Work::Apply(input, reply) => {
                            let _ = reply.send(store.apply(&input));
                        }
                        Work::Ack(id, reply) => {
                            let _ = reply.send(store.ack(&id).map(|()| vec![]));
                        }
                        Work::Stop => break,
                    }
                }
            }));
            workers.push(sender);
        }
        Ok(Self { workers, handles })
    }
    fn shard(&self, id: &str) -> usize {
        let hash = id.bytes().fold(0xcbf29ce484222325u64, |h, b| {
            (h ^ b as u64).wrapping_mul(0x100000001b3)
        });
        (hash % self.workers.len() as u64) as usize
    }
    /// Immediate overload rejection. Once accepted, an event can commit even if its caller disconnects.
    pub fn submit(&self, input: Input) -> Result<Receiver<Result<Vec<Envelope>>>> {
        ensure!(
            !input.session_id.is_empty()
                && input.session_id.len() <= 200
                && !input.event_id.is_empty()
                && input.event_id.len() <= 100,
            "invalid identifiers"
        );
        if let crate::charging::Event::InitialDp { subscriber } = &input.event {
            ensure!(subscriber.len() <= 32, "subscriber too long");
        }
        let (tx, rx) = mpsc::channel();
        self.workers[self.shard(&input.session_id)]
            .try_send(Work::Apply(input, tx))
            .map_err(|_| anyhow!("overloaded or worker unavailable"))?;
        Ok(rx)
    }
    pub fn apply(&self, input: Input) -> Result<Vec<Envelope>> {
        self.submit(input)?.recv()?
    }
    pub fn ack(&self, action: &Envelope) -> Result<()> {
        let (tx, rx) = mpsc::channel();
        self.workers[self.shard(&action.session_id)]
            .try_send(Work::Ack(action.id.clone(), tx))
            .map_err(|_| anyhow!("overloaded or worker unavailable"))?;
        rx.recv()??;
        Ok(())
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        for w in &self.workers {
            let _ = w.send(Work::Stop);
        }
        for h in self.handles.drain(..) {
            let _ = h.join();
        }
    }
}
