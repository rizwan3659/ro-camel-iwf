use std::{
    io::{BufRead, Write},
    process::{Command, Stdio},
};

#[test]
fn forced_process_exit_retains_committed_outbox() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_ro-camel-iwf"))
        .args(["lab", dir.path().to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(child.stdin.as_mut().unwrap(),r#"{{"session_id":"crash.test;1","event_id":"idp","event":{{"InitialDp":{{"subscriber":"1234567890"}}}}}}"#).unwrap();
    let mut line = String::new();
    std::io::BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert!(line.contains("SendCcr"), "{line}");
    child.kill().unwrap();
    child.wait().unwrap();
    let mut recovered = Vec::new();
    for shard in 0..4 {
        let store = ro_camel_iwf::store::Store::open(
            dir.path().join(format!("shard-{shard}.redb")),
            60,
            100_000,
        )
        .unwrap();
        recovered.extend(store.pending(10).unwrap());
    }
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].session_id, "crash.test;1");
}
