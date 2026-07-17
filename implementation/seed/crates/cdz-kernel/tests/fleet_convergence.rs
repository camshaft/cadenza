//! L2d — the merge-request/reject ROUND-TRIP end-to-end over the log (the first fleet-convergence dogfood).
//!
//! Vision §9/§13: everything the fleet does is messaging, and "the inbox is a projection, not a queue" —
//! this test re-expresses a REAL fleet exchange (a `merge-request` → its `merged`/`reject` reply) as pure
//! log events + projections, tying L2a (the Message/Ack types + codec), L2b (the inbox_for fold), and L2c
//! (reply_then_ack) together over a `FileLog`. It demonstrates the machinery that would DELETE the fleet's
//! file-inbox faking (JSON files in a hub dir + `processed/` moves): the whole exchange is appends to one
//! ordered log, and each participant's inbox is a fold over it.
//!
//! It exercises the crate's PUBLIC API exactly as a downstream (the future fold owner driving fleet roles)
//! would, over the file-backed log — no network, no store, CI-safe.

use cdz_kernel::msg::{inbox_for, is_acked, reply_then_ack, Message, MESSAGE};
use cdz_kernel::{FileLog, Log};

/// A unique temp log path per call — pid + a per-process counter, so two tests in the same process (which
/// share a pid) never collide on the same file and accumulate each other's events.
fn temp_log() -> (std::path::PathBuf, FileLog) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let n = N.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("cdz-kernel-l2d-{}-{n}.log", std::process::id()));
    let _ = std::fs::remove_file(&p);
    (p.clone(), FileLog::open(&p).unwrap())
}

#[test]
fn a_merge_request_and_its_merged_reply_round_trip_as_log_events() {
    let (path, mut log) = temp_log();

    // 1) v-agent-harness sends pr-sync a merge-request (the exchange the fleet runs constantly).
    let mr = Message {
        from: "v-agent-harness".into(),
        to: "pr-sync".into(),
        kind: "merge-request".into(),
        subject: "cdz-kernel: L2d — the round-trip".into(),
        refs: vec!["f2ee11844".into()],
        body: b"one commit; gated 23/0".to_vec(),
    };
    let mr_seq = log.append(MESSAGE, &mr.encode()).unwrap();

    // 2) pr-sync's inbox (a fold) shows the merge-request, unread.
    let pr_inbox = inbox_for(&log.tail(0).unwrap(), "pr-sync");
    assert_eq!(
        pr_inbox.len(),
        1,
        "pr-sync has one unread message: the merge-request"
    );
    assert_eq!(pr_inbox[0].0, mr_seq, "it is the message at mr_seq");
    assert_eq!(pr_inbox[0].1.kind, "merge-request");
    assert_eq!(
        pr_inbox[0].1.refs,
        vec!["f2ee11844".to_string()],
        "the ref (the commit) survives"
    );

    // 3) pr-sync integrates it and replies `merged` + acks the merge-request — reply-then-ack, one call.
    let merged = Message {
        from: "pr-sync".into(),
        to: "v-agent-harness".into(),
        kind: "merged".into(),
        subject: "merged: cdz-kernel: L2d — the round-trip".into(),
        refs: vec!["f2ee11844".into()],
        body: b"landed".to_vec(),
    };
    reply_then_ack(&mut log, mr_seq, &merged).unwrap();

    // 4) The exchange has converged: the merge-request is processed (out of pr-sync's inbox), and the
    //    `merged` reply is in v-agent-harness's inbox — the whole round-trip is folds over one log.
    let events = log.tail(0).unwrap();
    assert!(
        is_acked(&events, mr_seq),
        "the merge-request is acked (processed)"
    );
    assert!(
        inbox_for(&events, "pr-sync").is_empty(),
        "pr-sync's inbox is empty — the merge-request was processed, not re-surfaced"
    );
    let harness_inbox = inbox_for(&events, "v-agent-harness");
    assert_eq!(
        harness_inbox.len(),
        1,
        "v-agent-harness has one message: the merged reply"
    );
    assert_eq!(harness_inbox[0].1.kind, "merged");
    assert_eq!(harness_inbox[0].1.body, b"landed");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_reject_reply_also_round_trips_and_the_request_can_be_re_sent() {
    // The reject path: pr-sync rejects a merge-request; the requester sees the reject and (in the fleet's
    // real flow) re-sends. Here we prove the reject round-trips and a fresh re-sent request is a new unread
    // message — the log naturally accommodates the resend without any queue bookkeeping.
    let (path, mut log) = temp_log();
    let mr = Message {
        from: "v-agent-harness".into(),
        to: "pr-sync".into(),
        kind: "merge-request".into(),
        subject: "a change".into(),
        refs: vec!["deadbeef".into()],
        body: b"diff".to_vec(),
    };
    let mr_seq = log.append(MESSAGE, &mr.encode()).unwrap();

    // pr-sync rejects + acks the request.
    let reject = Message {
        from: "pr-sync".into(),
        to: "v-agent-harness".into(),
        kind: "reject".into(),
        subject: "reject: a change".into(),
        refs: vec!["deadbeef".into()],
        body: b"gate red on rebase; fix + resend".to_vec(),
    };
    reply_then_ack(&mut log, mr_seq, &reject).unwrap();

    let events = log.tail(0).unwrap();
    assert!(
        is_acked(&events, mr_seq),
        "the rejected request is acked (not re-surfaced)"
    );
    let harness_inbox = inbox_for(&events, "v-agent-harness");
    assert_eq!(
        harness_inbox.len(),
        1,
        "the requester sees exactly the reject"
    );
    assert_eq!(harness_inbox[0].1.kind, "reject");

    // The requester fixes + RE-SENDS: a fresh merge-request is simply a new message event (new seq),
    // unread at pr-sync — no queue state to reconcile, the log just grows.
    let resent = Message {
        from: "v-agent-harness".into(),
        to: "pr-sync".into(),
        kind: "merge-request".into(),
        subject: "a change (v2)".into(),
        refs: vec!["cafef00d".into()],
        body: b"fixed diff".to_vec(),
    };
    let resent_seq = log.append(MESSAGE, &resent.encode()).unwrap();
    assert!(resent_seq > mr_seq, "the resend is a later event");
    let pr_inbox = inbox_for(&log.tail(0).unwrap(), "pr-sync");
    assert_eq!(
        pr_inbox.len(),
        1,
        "pr-sync sees the RESENT request (the original stays acked)"
    );
    assert_eq!(pr_inbox[0].0, resent_seq);
    assert_eq!(
        pr_inbox[0].1.refs,
        vec!["cafef00d".to_string()],
        "the resend carries the new commit"
    );

    let _ = std::fs::remove_file(&path);
}
