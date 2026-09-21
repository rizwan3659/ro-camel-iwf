//! Normalized single-service voice charging. These types are NOT CAP ASN.1.
use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Event {
    InitialDp {
        subscriber: String,
    },
    Usage {
        seconds: u32,
    },
    Disconnect {
        seconds: u32,
    },
    Answer {
        number: u32,
        success: bool,
        granted: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestType {
    Initial = 1,
    Update = 2,
    Termination = 3,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ccr {
    pub session_id: String,
    pub subscriber: String,
    pub number: u32,
    pub kind: RequestType,
    pub used_seconds: u32,
    pub requested_seconds: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    SendCcr(Ccr),
    /// Adapter must request BCSM events, apply the quota, then continue the call.
    StartCall {
        seconds: u32,
    },
    ApplyCharging {
        seconds: u32,
    },
    ReleaseCall,
    EndDialogue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Awaiting(Ccr),
    Active { quota: u32 },
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub subscriber: String,
    pub next_number: u32,
    pub phase: Phase,
}

pub fn transition(
    id: &str,
    current: Option<Session>,
    event: &Event,
    quota: u32,
) -> Result<(Session, Vec<Action>)> {
    ensure!(!id.is_empty() && id.len() <= 200, "invalid session id");
    ensure!(quota > 0, "quota must be positive");
    if let Event::InitialDp { subscriber } = event {
        ensure!(current.is_none(), "session already exists");
        ensure!(
            !subscriber.is_empty()
                && subscriber.len() <= 32
                && subscriber.bytes().all(|b| b.is_ascii_digit()),
            "invalid subscriber"
        );
        let request = Ccr {
            session_id: id.into(),
            subscriber: subscriber.clone(),
            number: 0,
            kind: RequestType::Initial,
            used_seconds: 0,
            requested_seconds: quota,
        };
        return Ok((
            Session {
                subscriber: subscriber.clone(),
                next_number: 1,
                phase: Phase::Awaiting(request.clone()),
            },
            vec![Action::SendCcr(request)],
        ));
    }
    let mut session = current.ok_or_else(|| anyhow::anyhow!("unknown session"))?;
    match (&session.phase, event) {
        (
            Phase::Active { quota: remaining },
            Event::Usage { seconds } | Event::Disconnect { seconds },
        ) => {
            ensure!(*seconds <= *remaining, "usage exceeds granted quota");
            let kind = if matches!(event, Event::Disconnect { .. }) {
                RequestType::Termination
            } else {
                RequestType::Update
            };
            let request = Ccr {
                session_id: id.into(),
                subscriber: session.subscriber.clone(),
                number: session.next_number,
                kind,
                used_seconds: *seconds,
                requested_seconds: if kind == RequestType::Termination {
                    0
                } else {
                    quota
                },
            };
            session.next_number = session
                .next_number
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("request counter exhausted"))?;
            session.phase = Phase::Awaiting(request.clone());
            Ok((session, vec![Action::SendCcr(request)]))
        }
        (
            Phase::Awaiting(request),
            Event::Answer {
                number,
                success,
                granted,
            },
        ) => {
            ensure!(*number == request.number, "answer request number mismatch");
            let action = if request.kind == RequestType::Termination {
                // A failed final debit needs reconciliation, not a successful close.
                ensure!(
                    *success,
                    "termination rejected: retain request for reconciliation"
                );
                session.phase = Phase::Closed;
                Action::EndDialogue
            } else if !success || *granted == 0 {
                session.phase = Phase::Closed;
                Action::ReleaseCall
            } else {
                ensure!(
                    *granted <= request.requested_seconds,
                    "grant exceeds requested limit"
                );
                let initial = request.kind == RequestType::Initial;
                session.phase = Phase::Active { quota: *granted };
                if initial {
                    Action::StartCall { seconds: *granted }
                } else {
                    Action::ApplyCharging { seconds: *granted }
                }
            };
            Ok((session, vec![action]))
        }
        _ => bail!("event invalid in current phase"),
    }
}
