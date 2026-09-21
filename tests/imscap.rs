use ro_camel_iwf::{
    charging::{Ccr, RequestType},
    diameter::{Message, credit_request},
    imscap::{Action, Event, transition},
};
fn request(number: u32, kind: RequestType, used: u32) -> Ccr {
    Ccr {
        session_id: "tas;1".into(),
        subscriber: "12345".into(),
        number,
        kind,
        used_seconds: used,
        requested_seconds: if kind == RequestType::Termination {
            0
        } else {
            60
        },
    }
}
#[test]
fn initial_requires_both_continue_and_grant_in_either_order() {
    for events in [
        [Event::Continue, Event::ApplyCharging { seconds: 60 }],
        [Event::ApplyCharging { seconds: 60 }, Event::Continue],
    ] {
        let (s, a) =
            transition(None, &Event::Request(request(0, RequestType::Initial, 0))).unwrap();
        assert!(matches!(a[0], Action::InitialDp { .. }));
        let (s, a) = transition(Some(&s), &events[0]).unwrap();
        assert!(a.is_empty());
        let (s, a) = transition(Some(&s), &events[1]).unwrap();
        assert!(s.pending.is_none());
        assert!(matches!(
            a[0],
            Action::Answer {
                result: 2001,
                seconds: 60,
                ..
            }
        ));
    }
}
#[test]
fn lifecycle_and_invalid_updates() {
    let (s, _) = transition(None, &Event::Request(request(0, RequestType::Initial, 0))).unwrap();
    let (s, _) = transition(Some(&s), &Event::Continue).unwrap();
    let (s, _) = transition(Some(&s), &Event::ApplyCharging { seconds: 60 }).unwrap();
    for (n, used) in [(2, 60), (1, 61), (1, 30)] {
        assert!(
            transition(
                Some(&s),
                &Event::Request(request(n, RequestType::Update, used))
            )
            .is_err()
        );
    }
    let (s, a) = transition(
        Some(&s),
        &Event::Request(request(1, RequestType::Update, 60)),
    )
    .unwrap();
    assert_eq!(
        a,
        vec![Action::ApplyChargingReport {
            seconds: 60,
            call_active: true
        }]
    );
    assert!(transition(Some(&s), &Event::ApplyCharging { seconds: 61 }).is_err());
    let (s, _) = transition(Some(&s), &Event::ApplyCharging { seconds: 60 }).unwrap();
    let (s, a) = transition(
        Some(&s),
        &Event::Request(request(2, RequestType::Termination, 12)),
    )
    .unwrap();
    assert_eq!(a.len(), 2);
    assert!(transition(Some(&s), &Event::ReleaseCall).is_err());
    let (s, a) = transition(Some(&s), &Event::DialogueClosed).unwrap();
    assert!(s.closed);
    assert!(matches!(
        a[0],
        Action::Answer {
            result: 2001,
            seconds: 0,
            ..
        }
    ));
}
#[test]
fn denial_and_wire_roundtrip() {
    let ccr = request(0, RequestType::Initial, 0);
    let msg = credit_request(&ccr, "tas.test", "test", "test", "imscap-sim@test", 42, 99).unwrap();
    let msg = Message::decode(&msg.encode().unwrap()).unwrap();
    assert_eq!(msg.imscap_request().unwrap(), ccr);
    let (s, _) = transition(None, &Event::Request(ccr.clone())).unwrap();
    let (_, a) = transition(Some(&s), &Event::ReleaseCall).unwrap();
    let cca = msg.imscap_answer(&a[0], "imscap.test", "test").unwrap();
    assert_eq!(
        cca.credit_answer(&ccr, 42, 99).unwrap(),
        ro_camel_iwf::charging::Event::Answer {
            number: 0,
            success: false,
            granted: 0
        }
    );
    let mut bad = msg.clone();
    bad.avps.push(bad.avps[0].clone());
    assert!(bad.imscap_request().is_err());
    let mut bad = msg;
    bad.avps[0].vendor = Some(123);
    assert!(bad.imscap_request().is_err());
}
