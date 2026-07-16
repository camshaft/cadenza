//! Integration test for the `cdz-agent` inbox-loop BINARY: it reads fleet inbox JSON messages and drives
//! each through the authorized Cadenza agent loop. Invokes the built binary as a subprocess (via
//! `CARGO_BIN_EXE_cdz-agent`) against a temp inbox + the authz-model-consumer fixture, with the
//! permit-all default policy + the mock model — no AWS creds/network. Skips if the value-heap runtime
//! store isn't present (the driver needs it to run the consumer; a runtime bump can stale the fixture).

use std::path::PathBuf;

fn find_runtime_present(hash: &str) -> bool {
    let mut dir: PathBuf = env!("CARGO_MANIFEST_DIR").into();
    loop {
        if dir
            .join("target/cadenza-store")
            .join(format!("{hash}.wasm"))
            .exists()
        {
            return true;
        }
        if !dir.pop() {
            return false;
        }
    }
}

#[test]
fn the_driver_processes_inbox_messages_through_the_authorized_loop() {
    // The 3-effect consumer: Inbox.next -> Model.converse (on the MESSAGE BODY) -> Cedar-gated. So the
    // returned byte-len reflects the message body's length, proving the body reaches the model.
    let fixture: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "tests/fixtures/inbox-model-consumer.wasm",
    ]
    .iter()
    .collect();
    let consumer = std::fs::read(&fixture).expect("read the consumer fixture");
    let req = cdz_run::required_runtime(&consumer)
        .expect("read fixture runtime requirement")
        .expect("the fixture imports the value-heap runtime");
    if !find_runtime_present(&req.hash) {
        eprintln!(
            "[cdz-agent driver] runtime {} not in any ancestor store (run `cargo xtask build`) or stale fixture; skipping",
            req.hash
        );
        return;
    }

    // A temp inbox with two JSON messages in the fleet's format (unique per process to avoid collisions).
    let dir = std::env::temp_dir().join(format!("cdz-agent-driver-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir temp inbox");
    // Two messages with DISTINCT body byte-lengths, so the reported value proves the body reached the
    // model (a fixed prompt would give the same value for both). "do the task" = 11, "another" = 7.
    std::fs::write(
        dir.join("001-msg.json"),
        r#"{"from":"tester","to":"cdz-agent","kind":"note","subject":"hi","body":"do the task"}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("002-msg.json"),
        r#"{"from":"tester","to":"cdz-agent","kind":"note","subject":"two","body":"another"}"#,
    )
    .unwrap();

    // Invoke the built binary: permit-all default policy, mock model. The loop reads each body via
    // Inbox.next, converses (mock = uppercase, same byte-len), and returns its byte-len — so message 1
    // ("do the task", 11 bytes) → value 11 and message 2 ("another", 7 bytes) → value 7. Distinct values
    // prove the ACTUAL body drove each model call.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_cdz-agent"))
        .args([
            "--consumer",
            fixture.to_str().unwrap(),
            "--inbox",
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("spawn cdz-agent");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        out.status.success(),
        "the driver must exit 0: stdout=<{stdout}> stderr=<{stderr}>"
    );
    assert!(
        stdout.contains("001-msg.json -> value 11") && stdout.contains("002-msg.json -> value 7"),
        "each message's OWN body drives its model call (11 and 7 bytes) — the body reaches the model: {stdout}"
    );
    assert!(
        stdout.contains("processed 2 message(s)"),
        "the driver reports processing both messages: {stdout}"
    );
}
