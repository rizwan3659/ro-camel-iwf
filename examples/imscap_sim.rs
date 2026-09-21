//! In-process TAS + normalized CAMEL SCP simulation. No SS7 networking.
use anyhow::{Result, ensure};
use ro_camel_iwf::{
    charging::{Ccr, RequestType},
    diameter::{Message, credit_request},
    imscap::{self, Action, Event, Session},
};

fn main() -> Result<()> {
    let mut state: Option<Session> = None;
    for (number, kind, used) in [
        (0, RequestType::Initial, 0),
        (1, RequestType::Update, 60),
        (2, RequestType::Termination, 12),
    ] {
        let ccr = Ccr {
            session_id: "tas.test;call-1".into(),
            subscriber: "1000000000".into(),
            number,
            kind,
            used_seconds: used,
            requested_seconds: if kind == RequestType::Termination {
                0
            } else {
                60
            },
        };
        let wire = credit_request(
            &ccr,
            "tas.test",
            "test",
            "test",
            "imscap-sim@test",
            number + 10,
            number + 100,
        )?
        .encode()?;
        let request = Message::decode(&wire)?;
        let (next, actions) =
            imscap::transition(state.as_ref(), &Event::Request(request.imscap_request()?))?;
        println!(
            "TAS CCR {kind:?} -> IMSCAP -> SCP: {}",
            serde_json::to_string(&actions)?
        );
        state = Some(next);
        let replies = match kind {
            RequestType::Initial => vec![Event::ApplyCharging { seconds: 60 }, Event::Continue],
            RequestType::Update => vec![Event::ApplyCharging { seconds: 60 }],
            RequestType::Termination => vec![Event::DialogueClosed],
        };
        let mut answered = false;
        for reply in replies {
            let (next, actions) = imscap::transition(state.as_ref(), &reply)?;
            state = Some(next);
            for action in actions {
                if matches!(action, Action::Answer { .. }) {
                    let wire = request
                        .imscap_answer(&action, "imscap.test", "test")?
                        .encode()?;
                    let answer =
                        Message::decode(&wire)?.credit_answer(&ccr, number + 10, number + 100)?;
                    println!("SCP -> IMSCAP -> TAS: {}", serde_json::to_string(&answer)?);
                    answered = true;
                }
            }
        }
        ensure!(answered, "missing CCA");
    }
    ensure!(state.is_some_and(|s| s.closed), "session not closed");
    Ok(())
}
