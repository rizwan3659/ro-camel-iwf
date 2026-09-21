//! TAS-facing Ro server core. CAP operations here are normalized, not wire ASN.1.
//! The caller must atomically persist state, replay receipts and actions before dispatch.
use crate::charging::{Ccr, RequestType};
use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Event {
    Request(Ccr),
    ApplyCharging { seconds: u32 },
    Continue,
    ReleaseCall,
    DialogueClosed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    InitialDp {
        subscriber: String,
    },
    ApplyChargingReport {
        seconds: u32,
        call_active: bool,
    },
    // The CAP adapter must map this using the configured originating/terminating BCSM.
    DisconnectEvent,
    Answer {
        number: u32,
        kind: RequestType,
        result: u32,
        seconds: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub subscriber: String,
    pub next_number: u32,
    pub pending: Option<Ccr>,
    pub grant: u32,
    pub continued: bool,
    pub closed: bool,
}

fn answer(request: &Ccr, result: u32, seconds: u32) -> Action {
    Action::Answer {
        number: request.number,
        kind: request.kind,
        result,
        seconds,
    }
}

/// Deterministic transition: errors leave the caller's original state untouched.
/// Duplicate CCRs must be intercepted by a durable receipt layer before this function.
/// This first profile supports whole-grant, time-based updates only.
pub fn transition(current: Option<&Session>, event: &Event) -> Result<(Session, Vec<Action>)> {
    if current.is_none() {
        let Event::Request(request) = event else {
            bail!("unknown session")
        };
        ensure!(
            request.kind == RequestType::Initial && request.number == 0,
            "expected CCR-I number zero"
        );
        ensure!(
            !request.session_id.is_empty() && request.session_id.len() <= 200,
            "invalid session id"
        );
        ensure!(
            !request.subscriber.is_empty()
                && request.subscriber.len() <= 32
                && request.subscriber.bytes().all(|b| b.is_ascii_digit()),
            "invalid subscriber"
        );
        ensure!(
            request.used_seconds == 0 && request.requested_seconds > 0,
            "invalid initial units"
        );
        return Ok((
            Session {
                id: request.session_id.clone(),
                subscriber: request.subscriber.clone(),
                next_number: 1,
                pending: Some(request.clone()),
                grant: 0,
                continued: false,
                closed: false,
            },
            vec![Action::InitialDp {
                subscriber: request.subscriber.clone(),
            }],
        ));
    }
    let mut state = current.unwrap().clone();
    ensure!(!state.closed, "session closed");
    let mut actions = Vec::new();
    match event {
        Event::Request(request) => {
            ensure!(
                request.session_id == state.id && request.subscriber == state.subscriber,
                "session identity changed"
            );
            ensure!(state.pending.is_none(), "request already pending");
            ensure!(request.number == state.next_number, "out-of-order request");
            ensure!(
                request.kind != RequestType::Initial,
                "duplicate initial request"
            );
            ensure!(request.used_seconds <= state.grant, "usage exceeds grant");
            if request.kind == RequestType::Update {
                ensure!(
                    request.used_seconds == state.grant && request.requested_seconds > 0,
                    "only whole-grant updates supported"
                );
            } else {
                ensure!(
                    request.requested_seconds == 0,
                    "termination cannot request credit"
                );
            }
            state.next_number = state
                .next_number
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("request counter exhausted"))?;
            actions.push(Action::ApplyChargingReport {
                seconds: request.used_seconds,
                call_active: request.kind != RequestType::Termination,
            });
            if request.kind == RequestType::Termination {
                actions.push(Action::DisconnectEvent);
            }
            state.grant = 0;
            state.pending = Some(request.clone());
        }
        Event::ApplyCharging { seconds } => {
            let request = state
                .pending
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("no pending CCR"))?;
            ensure!(
                request.kind != RequestType::Termination,
                "grant during termination"
            );
            ensure!(
                *seconds > 0 && state.grant == 0,
                "invalid or overlapping grant"
            );
            // Never silently truncate a CAMEL grant: that would desynchronise the SCP.
            ensure!(
                *seconds <= request.requested_seconds,
                "grant exceeds requested units: mapping policy required"
            );
            state.grant = *seconds;
        }
        Event::Continue => {
            ensure!(
                state
                    .pending
                    .as_ref()
                    .is_some_and(|r| r.kind == RequestType::Initial)
                    && !state.continued,
                "unexpected Continue"
            );
            state.continued = true;
        }
        Event::ReleaseCall => {
            let request = state.pending.take().ok_or_else(|| {
                anyhow::anyhow!("unsolicited release requires TAS reauthorization/abort support")
            })?;
            ensure!(
                request.kind != RequestType::Termination,
                "release is not settlement confirmation"
            );
            actions.push(answer(&request, 4012, 0));
            state.closed = true;
            state.grant = 0;
        }
        Event::DialogueClosed => {
            let request = state
                .pending
                .take()
                .ok_or_else(|| anyhow::anyhow!("unexpected dialogue close"))?;
            ensure!(
                request.kind == RequestType::Termination,
                "dialogue ended before authorization"
            );
            actions.push(answer(&request, 2001, 0));
            state.closed = true;
        }
    }
    if state.continued
        && state.grant > 0
        && state
            .pending
            .as_ref()
            .is_some_and(|r| r.kind != RequestType::Termination)
    {
        let request = state.pending.take().unwrap();
        actions.push(answer(&request, 2001, state.grant));
    }
    Ok((state, actions))
}
