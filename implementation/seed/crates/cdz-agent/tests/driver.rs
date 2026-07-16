//! Integration test for the `cdz-agent` inbox-loop BINARY: it reads fleet inbox JSON messages and drives
//! each through the authorized Cadenza agent loop. Invokes the built binary as a subprocess (via
//! `CARGO_BIN_EXE_cdz-agent`) against a temp inbox + the authz-model-consumer fixture, with the
//! permit-all default policy + the mock model — no AWS creds/network. Skips if the value-heap runtime
//! store isn't present (the driver needs it to run the consumer; a runtime bump can stale the fixture).
//!
//! MOCK-ONLY: these assertions pin the mock backend's DETERMINISTIC output (uppercase completions, exact
//! byte-lengths). Under `--features bedrock` the built binary is the REAL Bedrock backend, whose output
//! is a live model answer (creds present) or an error marker (no creds) — neither matches these fixed
//! expectations. So the whole file is compiled out of the bedrock build; the `bedrock` feature's own
//! coverage is the lib's response-decode unit tests (no network), which `cargo test --features bedrock`
//! still runs. (Without this gate, CI's `cargo test --features bedrock` reran these subprocess tests
//! against the bedrock binary and they could never pass — a latent job failure.)
#![cfg(not(feature = "bedrock"))]

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

#[test]
fn the_driver_returns_the_actual_model_completion() {
    // Now that a multi-peer entrypoint may RETURN a String (v-peer-linking PL46), the loop can return the
    // model's actual completion — not just a scalar byte-len. This consumer's main RETURNS
    // Model.converse(Inbox.next()) directly; the mock model uppercases, so the driver reports the real
    // answer text `(: "DO THE TASK" String)`, proving the completion (not a derived scalar) flows out.
    let fixture: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "tests/fixtures/inbox-model-return-consumer.wasm",
    ]
    .iter()
    .collect();
    let consumer = std::fs::read(&fixture).expect("read the return-consumer fixture");
    let req = cdz_run::required_runtime(&consumer)
        .expect("read fixture runtime requirement")
        .expect("the fixture imports the value-heap runtime");
    if !find_runtime_present(&req.hash) {
        eprintln!(
            "[cdz-agent driver] runtime {} absent or stale fixture; skipping",
            req.hash
        );
        return;
    }

    let dir = std::env::temp_dir().join(format!("cdz-agent-return-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir temp inbox");
    std::fs::write(
        dir.join("001-msg.json"),
        r#"{"from":"tester","to":"cdz-agent","kind":"note","subject":"s","body":"do the task"}"#,
    )
    .unwrap();

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
    // The mock uppercases "do the task" → "DO THE TASK"; the loop returns that String, rendered
    // `(: "DO THE TASK" String)` — the ACTUAL model answer, not a byte-len.
    assert!(
        stdout.contains(r#"(: "DO THE TASK" String)"#),
        "the driver reports the model's actual completion text: {stdout}"
    );
}

/// A model-consumer fixture (any of the three shapes) whose runtime is present, or None to skip.
fn consumer_and_runtime_or_skip(rel: &str) -> Option<PathBuf> {
    let fixture: PathBuf = [env!("CARGO_MANIFEST_DIR"), rel].iter().collect();
    let consumer = std::fs::read(&fixture).ok()?;
    let hash = cdz_run::required_runtime(&consumer).ok()??.hash;
    find_runtime_present(&hash).then_some(fixture)
}

#[test]
fn the_driver_reports_an_empty_inbox_cleanly() {
    // A driver run over an inbox with NO messages must exit 0 with a clear "empty" report — not error,
    // not hang. (The consumer/runtime need not even be exercised; but resolve them so the run reaches the
    // inbox scan.) Robustness: a real hive inbox is often empty between messages.
    let Some(fixture) = consumer_and_runtime_or_skip("tests/fixtures/inbox-model-consumer.wasm")
    else {
        eprintln!("[cdz-agent driver] runtime absent; skipping");
        return;
    };
    let dir = std::env::temp_dir().join(format!("cdz-agent-empty-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_cdz-agent"))
        .args([
            "--consumer",
            fixture.to_str().unwrap(),
            "--inbox",
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("spawn");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.status.success(),
        "an empty inbox is not an error: {stdout}"
    );
    assert!(
        stdout.contains("is empty"),
        "the driver reports an empty inbox: {stdout}"
    );
}

#[test]
fn the_driver_tolerates_a_bodyless_message_and_ignores_non_json() {
    // Robustness: a JSON message with NO `body` field drives the loop with an empty body (no panic — the
    // reader defaults to ""), and a non-`.json` file in the inbox is IGNORED (not parsed as a message).
    // A real fleet inbox dir also holds a `processed/` subdir + assorted files; the driver must only act
    // on `*.json` message files.
    let Some(fixture) = consumer_and_runtime_or_skip("tests/fixtures/inbox-model-consumer.wasm")
    else {
        eprintln!("[cdz-agent driver] runtime absent; skipping");
        return;
    };
    let dir = std::env::temp_dir().join(format!("cdz-agent-bad-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    // A message with no `body` → empty body → the loop runs; byte-len of "" (uppercased) = 0.
    std::fs::write(dir.join("001.json"), r#"{"from":"t","subject":"no body"}"#).unwrap();
    // A non-json file that must be ignored (not parsed, not counted).
    std::fs::write(dir.join("notes.txt"), "not a message").unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_cdz-agent"))
        .args([
            "--consumer",
            fixture.to_str().unwrap(),
            "--inbox",
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("spawn");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.status.success(),
        "a bodyless message must not error: stdout=<{stdout}> stderr=<{stderr}>"
    );
    assert!(
        stdout.contains("001.json -> value 0"),
        "a no-body message drives the loop with an empty body (byte-len 0), no panic: {stdout}"
    );
    assert!(
        stdout.contains("processed 1 message(s)"),
        "only the ONE .json message is processed; the .txt is ignored: {stdout}"
    );
}

#[test]
fn the_driver_acks_processed_messages_with_the_ack_flag() {
    // With --ack, a driven message is MOVED into <inbox>/processed/ so a re-run doesn't re-process it
    // (an at-most-once drain, the fleet's ack convention). Without --ack the inbox is left untouched
    // (a read-only dry run, covered by the other tests). Pins: after --ack the message is in processed/
    // and gone from the inbox, and a second --ack run sees an empty inbox.
    let Some(fixture) = consumer_and_runtime_or_skip("tests/fixtures/inbox-model-consumer.wasm")
    else {
        eprintln!("[cdz-agent driver] runtime absent; skipping");
        return;
    };
    let dir = std::env::temp_dir().join(format!("cdz-agent-ack-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(
        dir.join("001-msg.json"),
        r#"{"from":"t","to":"a","kind":"note","subject":"s","body":"hi"}"#,
    )
    .unwrap();

    let run = |acked: bool| {
        let mut args = vec![
            "--consumer".to_string(),
            fixture.to_str().unwrap().to_string(),
            "--inbox".to_string(),
            dir.to_str().unwrap().to_string(),
        ];
        if acked {
            args.push("--ack".to_string());
        }
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_cdz-agent"))
            .args(&args)
            .output()
            .expect("spawn");
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    };

    // First run WITH --ack: processes 1, moves it to processed/.
    let (ok, stdout) = run(true);
    assert!(ok, "ack run must exit 0: {stdout}");
    assert!(
        stdout.contains("processed 1 message(s)"),
        "one message driven: {stdout}"
    );
    assert!(
        dir.join("processed/001-msg.json").is_file() && !dir.join("001-msg.json").exists(),
        "the message is MOVED to processed/ (acked), gone from the inbox"
    );
    // Second run: the inbox is now empty (the message was acked — not re-processed).
    let (ok2, stdout2) = run(true);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(ok2, "second run exits 0: {stdout2}");
    assert!(
        stdout2.contains("is empty"),
        "after ack, a re-run finds the inbox empty — at-most-once drain: {stdout2}"
    );
}

#[test]
fn the_driver_replies_to_the_sender_with_the_model_completion() {
    // With --reply-to, the model's answer is written back to the SENDER as a fleet-format reply message
    // (kind "answer", addressed `to` the source `from`, `body` = the completion). This closes the hive
    // loop: a peer sends a task and gets the agent's answer back. The return-consumer's main RETURNS the
    // completion string, and the mock uppercases — so a body of "do the task" yields a reply body of
    // "DO THE TASK". Pins: the reply exists, is addressed to the sender, is kind "answer", and carries the
    // actual (uppercased) completion.
    let Some(fixture) =
        consumer_and_runtime_or_skip("tests/fixtures/inbox-model-return-consumer.wasm")
    else {
        eprintln!("[cdz-agent driver] runtime absent; skipping");
        return;
    };
    let dir = std::env::temp_dir().join(format!("cdz-agent-reply-{}", std::process::id()));
    let replies = std::env::temp_dir().join(format!("cdz-agent-replyout-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&replies);
    std::fs::create_dir_all(&dir).expect("mkdir inbox");
    std::fs::write(
        dir.join("001-msg.json"),
        r#"{"from":"v-peer","to":"cdz-agent","kind":"note","subject":"s","body":"do the task"}"#,
    )
    .unwrap();

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_cdz-agent"))
        .args([
            "--consumer",
            fixture.to_str().unwrap(),
            "--inbox",
            dir.to_str().unwrap(),
            "--reply-to",
            replies.to_str().unwrap(),
        ])
        .output()
        .expect("spawn");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        out.status.success(),
        "the driver must exit 0: stdout=<{stdout}> stderr=<{stderr}>"
    );
    assert!(
        stdout.contains("replied to v-peer"),
        "the driver reports replying to the sender: {stdout}"
    );
    // The reply file is `reply-<source>` in the reply dir.
    let reply_path = replies.join("reply-001-msg.json");
    let reply = std::fs::read_to_string(&reply_path).expect("the reply message was written");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&replies);

    assert!(
        reply.contains(r#""to":"v-peer""#),
        "the reply is addressed back to the sender: {reply}"
    );
    assert!(
        reply.contains(r#""kind":"answer""#),
        "the reply is an `answer` message: {reply}"
    );
    assert!(
        reply.contains(r#""from":"cdz-agent""#),
        "the reply names the driver as sender: {reply}"
    );
    assert!(
        reply.contains(r#""body":"DO THE TASK""#),
        "the reply body is the model's ACTUAL completion (mock uppercases): {reply}"
    );
    assert!(
        reply.contains(r#""in_reply_to":"001-msg.json""#),
        "the reply names the SOURCE message filename in `in_reply_to` (audit correlation): {reply}"
    );
}

#[test]
fn the_driver_reads_real_pretty_printed_fleet_messages_and_writes_a_readable_reply() {
    // REGRESSION for Copilot PR#494 — the driver against REAL fleet inboxes:
    //  (1) `cargo xtask fleet` delivers messages via serde_json::to_string_pretty, so a real message is
    //      MULTI-LINE with a SPACE after each colon (`"body": "…"`). A compact `"key":"` needle parsed
    //      body/from EMPTY → the driver sent an empty prompt and --reply-to refused (empty from). Here the
    //      message is written in the EXACT pretty shape; the reply proves body+from parsed (uppercased
    //      body reached the model, reply addressed to the sender).
    //  (2) the reply must itself be a fleet-READABLE message: the fleet `Message` struct has a REQUIRED
    //      `seq` field, so a reply lacking it fails to deserialize and sits unread. Assert the reply
    //      carries `seq`.
    let Some(fixture) =
        consumer_and_runtime_or_skip("tests/fixtures/inbox-model-return-consumer.wasm")
    else {
        eprintln!("[cdz-agent driver] runtime absent; skipping");
        return;
    };
    let dir = std::env::temp_dir().join(format!("cdz-agent-pretty-{}", std::process::id()));
    let replies = std::env::temp_dir().join(format!("cdz-agent-prettyout-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&replies);
    std::fs::create_dir_all(&dir).expect("mkdir inbox");
    // EXACTLY the shape serde_json::to_string_pretty produces (2-space indent, `": "` separators, each
    // field on its own line) — a real `cargo xtask fleet send` message, not a hand-compacted one.
    let pretty = "{\n  \"from\": \"v-peer\",\n  \"to\": \"cdz-agent\",\n  \"kind\": \"note\",\n  \"subject\": \"do it\",\n  \"body\": \"do the task\",\n  \"seq\": 1\n}";
    std::fs::write(dir.join("001-msg.json"), pretty).unwrap();

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_cdz-agent"))
        .args([
            "--consumer",
            fixture.to_str().unwrap(),
            "--inbox",
            dir.to_str().unwrap(),
            "--reply-to",
            replies.to_str().unwrap(),
        ])
        .output()
        .expect("spawn");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        out.status.success(),
        "the driver must exit 0: stdout=<{stdout}> stderr=<{stderr}>"
    );
    // `from` parsed from PRETTY json → the reply is addressed to the sender (not refused as empty-from).
    assert!(
        stdout.contains("replied to v-peer"),
        "the sender's `from` parsed from pretty json → the driver replies to it: stdout=<{stdout}> stderr=<{stderr}>"
    );
    let reply = std::fs::read_to_string(replies.join("reply-001-msg.json"))
        .expect("the reply message was written");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&replies);

    // `body` parsed from PRETTY json → the ACTUAL body ("do the task") reached the model, uppercased to
    // "DO THE TASK" (a compact-only needle would have parsed body empty → reply body "").
    assert!(
        reply.contains(r#""body":"DO THE TASK""#),
        "the body parsed from pretty json and reached the model: {reply}"
    );
    assert!(
        reply.contains(r#""to":"v-peer""#),
        "the reply is addressed to the sender parsed from pretty json: {reply}"
    );
    // The reply carries the fleet-REQUIRED `seq` field, so `cargo xtask fleet`/slack-bridge can read it.
    assert!(
        reply.contains(r#""seq":"#),
        "the reply carries the fleet-required `seq` field (else it fails to deserialize): {reply}"
    );
}

#[test]
fn the_driver_parses_a_field_whose_key_text_appears_in_an_earlier_value() {
    // REGRESSION for Copilot PR#496: json_string_field must try EVERY `"<key>"` occurrence, not just the
    // first. Here the `subject` value literally contains the word "from" (and the token `"body"`), which
    // appears in the JSON text BEFORE the real `"from"`/`"body"` fields. A first-occurrence-only reader
    // would match inside the subject value, find no `: "…"` there, and bail → empty from/body → the driver
    // refuses to reply. With the fix it scans past the non-field occurrence to the real field.
    let Some(fixture) =
        consumer_and_runtime_or_skip("tests/fixtures/inbox-model-return-consumer.wasm")
    else {
        eprintln!("[cdz-agent driver] runtime absent; skipping");
        return;
    };
    let dir = std::env::temp_dir().join(format!("cdz-agent-keyinval-{}", std::process::id()));
    let replies =
        std::env::temp_dir().join(format!("cdz-agent-keyinvalout-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&replies);
    std::fs::create_dir_all(&dir).expect("mkdir inbox");
    // `subject` (which precedes `from`/`body` in the object) mentions the words `from` and `"body"` inside
    // its VALUE, so the naive first-`"from"`/first-`"body"` match lands inside the subject string.
    let msg = r#"{"subject":"a note from ops about the \"body\" field","from":"v-peer","kind":"note","body":"do the task","seq":1}"#;
    std::fs::write(dir.join("001-msg.json"), msg).unwrap();

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_cdz-agent"))
        .args([
            "--consumer",
            fixture.to_str().unwrap(),
            "--inbox",
            dir.to_str().unwrap(),
            "--reply-to",
            replies.to_str().unwrap(),
        ])
        .output()
        .expect("spawn");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        out.status.success(),
        "the driver must exit 0: stdout=<{stdout}> stderr=<{stderr}>"
    );
    // `from` parsed the REAL field (not the mention inside subject) → the reply is addressed to v-peer.
    assert!(
        stdout.contains("replied to v-peer"),
        "the real `from` field parsed past the key-word in the subject value: stdout=<{stdout}> stderr=<{stderr}>"
    );
    let reply = std::fs::read_to_string(replies.join("reply-001-msg.json"))
        .expect("the reply message was written");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&replies);
    // `body` parsed the REAL field ("do the task") past the `"body"` mention in subject → uppercased.
    assert!(
        reply.contains(r#""body":"DO THE TASK""#),
        "the real `body` field parsed past the `\"body\"` token inside the subject value: {reply}"
    );
    assert!(
        reply.contains(r#""to":"v-peer""#),
        "the reply is addressed to the sender parsed from the real `from` field: {reply}"
    );
}

#[test]
fn the_driver_does_not_reply_when_the_model_was_denied() {
    // A Cedar-DENIED run never calls the model, so there is nothing to answer — no reply is written even
    // with --reply-to. The authz consumer returns byte-len on allow / 0 on deny; drive it with a
    // permit-NOTHING policy so authorize denies → the model is never called → no reply. (The default
    // policy is permit-all, so we must supply a deny policy explicitly.)
    let Some(fixture) = consumer_and_runtime_or_skip("tests/fixtures/authz-model-consumer.wasm")
    else {
        eprintln!("[cdz-agent driver] runtime absent; skipping");
        return;
    };
    let dir = std::env::temp_dir().join(format!("cdz-agent-noreply-{}", std::process::id()));
    let replies = std::env::temp_dir().join(format!("cdz-agent-noreplyout-{}", std::process::id()));
    let policy =
        std::env::temp_dir().join(format!("cdz-agent-denypol-{}.cedar", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&replies);
    std::fs::create_dir_all(&dir).expect("mkdir inbox");
    std::fs::write(
        dir.join("001-msg.json"),
        r#"{"from":"v-peer","to":"cdz-agent","kind":"note","subject":"s","body":"do the task"}"#,
    )
    .unwrap();
    // Permit only a DIFFERENT action, so "tool:chat" (what the consumer authorizes) has no permit → deny.
    std::fs::write(
        &policy,
        r#"permit(principal, action == Action::"tool:other", resource);"#,
    )
    .unwrap();

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_cdz-agent"))
        .args([
            "--consumer",
            fixture.to_str().unwrap(),
            "--inbox",
            dir.to_str().unwrap(),
            "--policy",
            policy.to_str().unwrap(),
            "--reply-to",
            replies.to_str().unwrap(),
        ])
        .output()
        .expect("spawn");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_file(&policy);

    assert!(
        out.status.success(),
        "the driver must exit 0 even when denied: stdout=<{stdout}> stderr=<{stderr}>"
    );
    // The reply dir exists (created up front) but holds NO reply file — the denied run had no completion.
    let any_reply = std::fs::read_dir(&replies)
        .map(|rd| rd.filter_map(|e| e.ok()).any(|e| e.path().is_file()))
        .unwrap_or(false);
    let _ = std::fs::remove_dir_all(&replies);
    assert!(
        !any_reply,
        "a denied run makes no model call → no reply is written: stderr=<{stderr}>"
    );
    assert!(
        stderr.contains("not replying"),
        "the driver explains it isn't replying (denied/failed): {stderr}"
    );
}
