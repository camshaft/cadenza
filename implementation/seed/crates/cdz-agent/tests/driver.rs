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
    let fixture: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "tests/fixtures/authz-model-consumer.wasm",
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

    // Invoke the built binary: permit-all default policy, mock model — it should process BOTH messages
    // and, since the consumer authorizes "tool:chat" (permitted) then converses, print `value 2` for each.
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
        stdout.contains("001-msg.json -> value 2") && stdout.contains("002-msg.json -> value 2"),
        "both messages drive the authorized loop to `value 2` (permit-all → allow → mock converse): {stdout}"
    );
    assert!(
        stdout.contains("processed 2 message(s)"),
        "the driver reports processing both messages: {stdout}"
    );
}
