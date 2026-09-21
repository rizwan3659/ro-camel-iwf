use ro_camel_iwf::{
    charging::{Action, Event, RequestType},
    store::{Input, Store},
};

fn input(id: &str, event: Event) -> Input {
    Input {
        session_id: "test.local;1".into(),
        event_id: id.into(),
        event,
    }
}
fn initial() -> Input {
    input(
        "idp",
        Event::InitialDp {
            subscriber: "1234567890".into(),
        },
    )
}
fn answer(n: u32, granted: u32) -> Input {
    input(
        &format!("cca-{n}"),
        Event::Answer {
            number: n,
            success: true,
            granted,
        },
    )
}

#[test]
fn voice_call_initial_update_termination() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("db"), 60, 100).unwrap();
    let actions = store.apply(&initial()).unwrap();
    assert!(
        matches!(&actions[0].action,Action::SendCcr(r) if r.number==0 && r.kind==RequestType::Initial)
    );
    assert_eq!(
        store.apply(&answer(0, 60)).unwrap()[0].action,
        Action::StartCall { seconds: 60 }
    );
    let update = store
        .apply(&input("acr", Event::Usage { seconds: 60 }))
        .unwrap();
    assert!(matches!(&update[0].action,Action::SendCcr(r) if r.number==1 && r.used_seconds==60));
    assert_eq!(
        store.apply(&answer(1, 30)).unwrap()[0].action,
        Action::ApplyCharging { seconds: 30 }
    );
    let end = store
        .apply(&input("end", Event::Disconnect { seconds: 5 }))
        .unwrap();
    assert!(
        matches!(&end[0].action,Action::SendCcr(r) if r.number==2 && r.used_seconds==5 && r.requested_seconds==0 && r.kind==RequestType::Termination)
    );
    assert_eq!(
        store.apply(&answer(2, 0)).unwrap()[0].action,
        Action::EndDialogue
    );
    assert!(
        store
            .apply(&input("late", Event::Usage { seconds: 1 }))
            .is_err()
    );
}

#[test]
fn restart_replays_same_action_and_ack_stays_acked() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let original;
    {
        let store = Store::open(&path, 60, 100).unwrap();
        original = store.apply(&initial()).unwrap();
    }
    {
        let store = Store::open(&path, 60, 100).unwrap();
        assert_eq!(store.pending(100).unwrap(), original);
        assert_eq!(store.apply(&initial()).unwrap(), original);
        store.ack(&original[0].id).unwrap();
    }
    let store = Store::open(&path, 60, 100).unwrap();
    assert!(store.pending(100).unwrap().is_empty());
    assert_eq!(store.apply(&initial()).unwrap(), original);
    assert!(store.pending(100).unwrap().is_empty());
    assert_eq!(
        store.apply(&answer(0, 60)).unwrap()[0].action,
        Action::StartCall { seconds: 60 }
    );
}

#[test]
fn conflicting_replay_and_wrong_answer_leave_state_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("db"), 60, 100).unwrap();
    store.apply(&initial()).unwrap();
    assert!(
        store
            .apply(&input(
                "idp",
                Event::InitialDp {
                    subscriber: "9999".into()
                }
            ))
            .is_err()
    );
    assert!(store.apply(&answer(1, 60)).is_err());
    assert!(store.apply(&answer(0, 61)).is_err());
    assert!(
        store
            .apply(&input("early", Event::Usage { seconds: 1 }))
            .is_err()
    );
    store.apply(&answer(0, 60)).unwrap();
    assert!(
        store
            .apply(&input("excess", Event::Usage { seconds: 61 }))
            .is_err()
    );
    assert!(
        store
            .apply(&input("valid", Event::Usage { seconds: 60 }))
            .is_ok()
    );
}

#[test]
fn denial_does_not_continue_call() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("db"), 60, 100).unwrap();
    store.apply(&initial()).unwrap();
    assert_eq!(
        store
            .apply(&input(
                "deny",
                Event::Answer {
                    number: 0,
                    success: false,
                    granted: 60
                }
            ))
            .unwrap()[0]
            .action,
        Action::ReleaseCall
    );
}

#[test]
fn outbox_pressure_rolls_back_transition() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("db"), 60, 1).unwrap();
    let original = store.apply(&initial()).unwrap();
    assert!(store.apply(&answer(0, 60)).is_err());
    store.ack(&original[0].id).unwrap();
    assert!(store.apply(&answer(0, 60)).is_ok());
}

#[test]
fn concurrent_duplicate_is_one_durable_action() {
    let dir = tempfile::tempdir().unwrap();
    let store = std::sync::Arc::new(Store::open(dir.path().join("db"), 60, 100).unwrap());
    let threads = (0..8)
        .map(|_| {
            let store = store.clone();
            std::thread::spawn(move || store.apply(&initial()).unwrap())
        })
        .collect::<Vec<_>>();
    let all = threads
        .into_iter()
        .map(|t| t.join().unwrap())
        .collect::<Vec<_>>();
    assert!(all.iter().all(|v| *v == all[0]));
    assert_eq!(store.pending(100).unwrap().len(), 1);
}

#[test]
fn failed_final_debit_remains_pending_for_reconciliation() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("db"), 60, 100).unwrap();
    store.apply(&initial()).unwrap();
    store.apply(&answer(0, 60)).unwrap();
    store
        .apply(&input("end", Event::Disconnect { seconds: 2 }))
        .unwrap();
    assert!(
        store
            .apply(&input(
                "failed",
                Event::Answer {
                    number: 1,
                    success: false,
                    granted: 0
                }
            ))
            .is_err()
    );
    assert_eq!(
        store.apply(&answer(1, 0)).unwrap()[0].action,
        Action::EndDialogue
    );
}

#[test]
fn two_process_owners_cannot_open_same_shard() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let _owner = Store::open(&path, 60, 100).unwrap();
    assert!(Store::open(&path, 60, 100).is_err());
}

#[test]
fn runtime_drains_accepted_work_and_preserves_partition_count() {
    use ro_camel_iwf::runtime::Runtime;
    let dir = tempfile::tempdir().unwrap();
    let rt = Runtime::open(dir.path(), 2, 1, 100).unwrap();
    let response = rt.submit(initial()).unwrap();
    drop(rt);
    assert!(response.recv().unwrap().is_ok());
    assert!(Runtime::open(dir.path(), 3, 1, 100).is_err());
    let restarted = Runtime::open(dir.path(), 2, 1, 100).unwrap();
    assert_eq!(restarted.apply(initial()).unwrap().len(), 1);
}
