use anyhow::{Result, bail, ensure};
use ro_camel_iwf::{
    charging::Event,
    runtime::Runtime,
    store::{Input, Store},
};
use std::{
    io::{self, BufRead},
    sync::Arc,
    time::Instant,
};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("lab") => {
            let dir = args
                .get(2)
                .ok_or_else(|| anyhow::anyhow!("lab requires a data directory"))?;
            let rt = Runtime::open(dir, 4, 1024, 100_000)?;
            // Local normalized-event harness; deliberately not exposed as a network protocol.
            let mut input = io::stdin().lock();
            loop {
                let mut line = Vec::new();
                use std::io::Read;
                let n = input.by_ref().take(4097).read_until(b'\n', &mut line)?;
                if n == 0 {
                    break;
                }
                ensure!(n <= 4096, "input exceeds 4096 bytes");
                let result = serde_json::from_slice::<Input>(&line)
                    .map_err(anyhow::Error::from)
                    .and_then(|event| rt.apply(event));
                match result {
                    Ok(actions) => println!("{}", serde_json::json!({"actions":actions})),
                    Err(e) => println!("{}", serde_json::json!({"error":e.to_string()})),
                }
            }
        }
        Some("outbox") => {
            let path = args.get(2).ok_or_else(|| {
                anyhow::anyhow!("outbox requires a shard file; stop runtime first")
            })?;
            let store = Store::open(path, 60, 100_000)?;
            println!("{}", serde_json::to_string_pretty(&store.pending(1000)?)?);
        }
        Some("bench") => {
            let dir = args
                .get(2)
                .ok_or_else(|| anyhow::anyhow!("bench requires a fresh data directory"))?;
            ensure!(
                !std::path::Path::new(dir).exists(),
                "benchmark directory must not exist"
            );
            let calls: usize = args.get(3).map(|s| s.parse()).transpose()?.unwrap_or(1000);
            let shards: usize = args.get(4).map(|s| s.parse()).transpose()?.unwrap_or(4);
            ensure!((1..=100_000).contains(&calls), "invalid call count");
            let rt = Arc::new(Runtime::open(dir, shards, 1024, calls as u64 + 10)?);
            let start = Instant::now();
            let mut workers = Vec::new();
            for worker in 0..4 {
                let rt = Arc::clone(&rt);
                workers.push(std::thread::spawn(move || -> Result<Vec<u128>> {
                    let mut latencies = Vec::new();
                    for call in (worker..calls).step_by(4) {
                        let id = format!("bench.local;{call}");
                        let events = [
                            Event::InitialDp {
                                subscriber: "1000000000".into(),
                            },
                            Event::Answer {
                                number: 0,
                                success: true,
                                granted: 60,
                            },
                            Event::Usage { seconds: 60 },
                            Event::Answer {
                                number: 1,
                                success: true,
                                granted: 60,
                            },
                            Event::Disconnect { seconds: 12 },
                            Event::Answer {
                                number: 2,
                                success: true,
                                granted: 0,
                            },
                        ];
                        for (n, event) in events.into_iter().enumerate() {
                            let t = Instant::now();
                            let actions = rt.apply(Input {
                                session_id: id.clone(),
                                event_id: n.to_string(),
                                event,
                            })?;
                            for action in actions {
                                rt.ack(&action)?;
                            }
                            latencies.push(t.elapsed().as_micros());
                        }
                    }
                    Ok(latencies)
                }));
            }
            let mut latencies = Vec::new();
            for worker in workers {
                latencies.extend(
                    worker
                        .join()
                        .map_err(|_| anyhow::anyhow!("worker panic"))??,
                );
            }
            let elapsed = start.elapsed().as_secs_f64();
            latencies.sort_unstable();
            println!(
                "{}",
                serde_json::json!({"mode":"durable-core-with-ack", "calls":calls,"workers":4,"shards":shards,
                "seconds":elapsed,"calls_per_second":calls as f64 / elapsed,"events_per_second":latencies.len() as f64 / elapsed,
                "event_p50_us":latencies[latencies.len()/2],"event_p99_us":latencies[latencies.len()*99/100],
                "network_included":false,"ss7_included":false})
            );
        }
        _ => bail!(
            "usage: ro-camel-iwf lab DATA_DIR | outbox SHARD_FILE | bench NEW_DIR [CALLS] [SHARDS]"
        ),
    }
    Ok(())
}
