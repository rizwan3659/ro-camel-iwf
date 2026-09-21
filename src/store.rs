//! A durable session, event deduplication and outbox transaction.
//! One writer per database; scale with independent, stably routed shards.
use crate::charging::{Action, Event, Session, transition};
use anyhow::{Result, ensure};
use redb::{Database, ReadableDatabase, ReadableTable, ReadableTableMetadata, TableDefinition};
use serde::{Deserialize, Serialize};
use std::path::Path;

const SESSIONS: TableDefinition<&str, &[u8]> = TableDefinition::new("sessions");
const RECEIPTS: TableDefinition<&str, &[u8]> = TableDefinition::new("receipts");
const OUTBOX: TableDefinition<&str, &[u8]> = TableDefinition::new("outbox");

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Input {
    pub session_id: String,
    pub event_id: String,
    pub event: Event,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Envelope {
    pub id: String,
    pub session_id: String,
    pub action: Action,
}
#[derive(Serialize, Deserialize)]
struct Receipt {
    event: Event,
    actions: Vec<Envelope>,
}

pub struct Store {
    db: Database,
    quota: u32,
    capacity: u64,
}
impl Store {
    pub fn open(path: impl AsRef<Path>, quota: u32, capacity: u64) -> Result<Self> {
        ensure!(
            quota > 0 && capacity > 0 && capacity <= 10_000_000,
            "invalid limits"
        );
        let db = Database::create(path)?;
        let tx = db.begin_write()?;
        tx.open_table(SESSIONS)?;
        tx.open_table(RECEIPTS)?;
        tx.open_table(OUTBOX)?;
        tx.commit()?;
        Ok(Self {
            db,
            quota,
            capacity,
        })
    }

    /// Commit before exposing actions. Replayed event IDs must have identical payloads.
    /// The outbox is at-least-once; consumers must acknowledge stable action IDs.
    pub fn apply(&self, input: &Input) -> Result<Vec<Envelope>> {
        ensure!(
            !input.event_id.is_empty() && input.event_id.len() <= 100,
            "invalid event id"
        );
        let key = serde_json::to_string(&(&input.session_id, &input.event_id))?;
        let tx = self.db.begin_write()?;
        let actions;
        {
            let mut receipts = tx.open_table(RECEIPTS)?;
            if let Some(previous) = receipts.get(key.as_str())? {
                let receipt: Receipt = serde_json::from_slice(previous.value())?;
                ensure!(
                    receipt.event == input.event,
                    "event id reused with different payload"
                );
                return Ok(receipt.actions);
            }
            let mut sessions = tx.open_table(SESSIONS)?;
            let old = sessions
                .get(input.session_id.as_str())?
                .map(|v| serde_json::from_slice::<Session>(v.value()))
                .transpose()?;
            // Includes tombstones: operators must plan retention before long-running use.
            ensure!(
                old.is_some() || sessions.len()? < self.capacity,
                "session capacity reached"
            );
            let (new, result) = transition(&input.session_id, old, &input.event, self.quota)?;
            actions = result
                .into_iter()
                .enumerate()
                .map(|(n, action)| Envelope {
                    id: format!("{key}:{n}"),
                    session_id: input.session_id.clone(),
                    action,
                })
                .collect::<Vec<_>>();
            let mut outbox = tx.open_table(OUTBOX)?;
            ensure!(
                outbox.len()? + actions.len() as u64 <= self.capacity,
                "outbox full"
            );
            // Bound receipts as well; fail closed rather than dropping deduplication history.
            ensure!(
                receipts.len()? < self.capacity * 32,
                "receipt capacity reached"
            );
            for action in &actions {
                let bytes = serde_json::to_vec(action)?;
                outbox.insert(action.id.as_str(), bytes.as_slice())?;
            }
            let bytes = serde_json::to_vec(&new)?;
            sessions.insert(input.session_id.as_str(), bytes.as_slice())?;
            let bytes = serde_json::to_vec(&Receipt {
                event: input.event.clone(),
                actions: actions.clone(),
            })?;
            receipts.insert(key.as_str(), bytes.as_slice())?;
        }
        tx.commit()?;
        Ok(actions)
    }

    pub fn pending(&self, limit: usize) -> Result<Vec<Envelope>> {
        let tx = self.db.begin_read()?;
        let table = tx.open_table(OUTBOX)?;
        table
            .iter()?
            .take(limit)
            .map(|row| {
                let (_, value) = row?;
                Ok(serde_json::from_slice(value.value())?)
            })
            .collect()
    }

    pub fn ack(&self, action_id: &str) -> Result<()> {
        let tx = self.db.begin_write()?;
        tx.open_table(OUTBOX)?.remove(action_id)?;
        tx.commit()?;
        Ok(())
    }
}
