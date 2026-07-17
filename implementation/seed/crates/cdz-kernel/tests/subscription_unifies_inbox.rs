//! L3d — the unification dogfood: the agent loop IS a subscription (vision §8/§15 rung 3).
//!
//! The L3 thesis is that subscriptions are *the one reactive primitive* — the agent loop ("wake me on
//! messages addressed to me"), reporters, compaction, routing all collapse onto it. This test proves the
//! foundational case end-to-end over a real `FileLog`, tying L2 (messaging + the [`inbox_for`] fold) to L3
//! (the [`Subscription`] + [`dispatch`]): a `Predicate::MessageTo(me)` subscription, dispatched as each event
//! lands, fires on EXACTLY the message events [`inbox_for`] surfaces. I.e. "the agent loop" and "a
//! MessageTo(me) subscription" are the same thing — the loop is not special machinery, just this primitive.
//!
//! It also pins the one semantic seam between the two views: `dispatch` fires WHEN a message lands (a
//! scheduling decision at landing time), while `inbox_for` is the UNREAD projection (it additionally drops
//! messages a later `ack` marked processed). So the messages a MessageTo(me) subscription fired on ⊇ the
//! current inbox, and they coincide exactly when nothing is acked — which the test asserts both ways.

use cdz_kernel::msg::{inbox_for, reply_then_ack, Message, MESSAGE};
use cdz_kernel::sub::{dispatch, Predicate, Subscription, SUBSCRIBE};
use cdz_kernel::{Event, FileLog, Log, Seq};

fn temp_log() -> (std::path::PathBuf, FileLog) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let n = N.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("cdz-kernel-l3d-{}-{n}.log", std::process::id()));
    let _ = std::fs::remove_file(&p);
    (p.clone(), FileLog::open(&p).unwrap())
}

fn msg(from: &str, to: &str) -> Message {
    Message {
        from: from.into(),
        to: to.into(),
        kind: "note".into(),
        subject: "s".into(),
        refs: vec![],
        body: b"b".to_vec(),
    }
}

/// Replay the log the way the fold owner would — feed each event to `dispatch` as it "lands" (the prefix up
/// to and including it) — and collect the seqs on which the `MessageTo(me)` subscription fired.
fn seqs_fired_for(events: &[Event]) -> Vec<Seq> {
    let mut fired = Vec::new();
    for i in 0..events.len() {
        let prefix = &events[..=i];
        let landed = &events[i];
        for (_, sub) in dispatch(prefix, landed) {
            if sub.predicate == Predicate::MessageTo("me".into()) {
                fired.push(landed.seq);
            }
        }
    }
    fired
}

#[test]
fn a_message_to_me_subscription_fires_on_exactly_the_inbox_messages() {
    let (path, mut log) = temp_log();

    // The agent registers "wake me on messages addressed to me" — a MessageTo(me) subscription.
    let sub = Subscription {
        id: "agent-loop".into(),
        predicate: Predicate::MessageTo("me".into()),
        program_ref: "agent-loop.wasm".into(),
        capability: "model".into(),
    };
    log.append(SUBSCRIBE, &sub.encode()).unwrap();

    // A mix of events lands: messages to me, a message to someone else, and a non-message event.
    log.append(MESSAGE, &msg("v-peer", "me").encode()).unwrap(); // seq 1  → fires
    log.append(MESSAGE, &msg("v-peer", "other").encode())
        .unwrap(); // seq 2  → no (not to me)
    log.append("model-response", b"result").unwrap(); // seq 3  → no (not a message)
    log.append(MESSAGE, &msg("v-two", "me").encode()).unwrap(); // seq 4  → fires

    let events = log.tail(0).unwrap();

    // THE UNIFICATION: the seqs a MessageTo(me) subscription fires on == the seqs inbox_for(me) surfaces.
    let fired = seqs_fired_for(&events);
    let inbox_seqs: Vec<Seq> = inbox_for(&events, "me").iter().map(|(s, _)| *s).collect();
    assert_eq!(
        fired, inbox_seqs,
        "the agent loop IS a MessageTo(me) subscription: dispatch fires on exactly the inbox messages"
    );
    assert_eq!(fired, vec![1, 4], "only the two messages addressed to me");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn dispatch_fires_at_landing_time_inbox_is_the_unread_subset_after_acks() {
    // The one seam: dispatch is a landing-time scheduling decision; inbox_for is the UNREAD projection. A
    // message that fired the subscription when it landed, then got acked, is no longer in the inbox — but it
    // DID fire. So fired ⊇ inbox, coinciding exactly when nothing is acked.
    let (path, mut log) = temp_log();
    let sub = Subscription {
        id: "agent-loop".into(),
        predicate: Predicate::MessageTo("me".into()),
        program_ref: "p.wasm".into(),
        capability: "model".into(),
    };
    log.append(SUBSCRIBE, &sub.encode()).unwrap();
    let m1 = log.append(MESSAGE, &msg("v-peer", "me").encode()).unwrap(); // seq 1
    let _m2 = log.append(MESSAGE, &msg("v-two", "me").encode()).unwrap(); // seq 2

    // Both fired at landing time.
    let events = log.tail(0).unwrap();
    assert_eq!(
        seqs_fired_for(&events),
        vec![1, 2],
        "both messages fired when they landed"
    );

    // The agent processes m1: reply-then-ack. Now inbox drops m1, but m1 still fired.
    reply_then_ack(&mut log, m1, &msg("me", "v-peer")).unwrap();
    let events = log.tail(0).unwrap();
    let inbox_seqs: Vec<Seq> = inbox_for(&events, "me").iter().map(|(s, _)| *s).collect();
    assert_eq!(inbox_seqs, vec![2], "m1 is acked → only m2 remains unread");

    // The reply is itself a message addressed to v-peer, not me — so it does NOT fire the MessageTo(me) sub.
    let fired = seqs_fired_for(&events);
    assert!(
        fired.contains(&1) && fired.contains(&2),
        "the acked m1 still fired at landing time (fired ⊇ inbox)"
    );
    assert!(
        inbox_seqs.iter().all(|s| fired.contains(s)),
        "every current-inbox message is a subset of what fired"
    );

    let _ = std::fs::remove_file(&path);
}
