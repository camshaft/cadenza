//! End-to-end: an agent session RUNS a real reactive loop through a REAL executor from this crate.
//!
//! This is the milestone this crate exists for, in its hermetic form: a reducer drives the kernel loop
//! (fold → authorize → durably-dispatch → EXECUTE → fold-result), and the executor doing the executing
//! is a genuine one that touches the world — [`ClockExecutor`], which reads the system wall clock — NOT
//! the kernel's tests-only `RecordingExecutor`. The `Now` effect is the hermetic proof point (no
//! network, no credentials); the Bedrock `Model` executor lands behind `live-net` next and slots into
//! the SAME `CompositeExecutor` wiring shown here.
//!
//! What it proves:
//! - the real executor's outcome folds back and drives the reducer's next step (the loop actually
//!   closes through a world-touching executor);
//! - the recorded result makes the run REPLAYABLE — replaying the log reconstructs the identical KV even
//!   though the clock read is non-deterministic (§9c: determinism lives in the recorded outcome, not the
//!   executor).

use cdz_agent_host::ClockExecutor;
use cdz_kernel::authz::Authorizer;
use cdz_kernel::effect::{
    effect_ct, Capability, EffectKind, EffectRequest, Payload, ResourcePredicate, Timeliness,
};
use cdz_kernel::event::{ContentType, EffectOutcome, Event, EventBody};
use cdz_kernel::executor::CompositeExecutor;
use cdz_kernel::hash::Hash;
use cdz_kernel::kernel::Session;
use cdz_kernel::kv::Kv;
use cdz_kernel::reducer::{FoldOutput, Reducer};

/// A minimal but real agent: on an inbound "go" it asks the kernel for the current time (a `Now`
/// effect); when the recorded instant comes back it stashes it in KV under `started_at` and marks
/// itself `running`. This is the fold→effect→result→continuation loop every agent runs — here the
/// effect is served by a REAL clock executor, so the loop closes against the world.
struct ClockAgent;

#[async_trait::async_trait(?Send)]
impl Reducer for ClockAgent {
    async fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
        match &event.body {
            EventBody::Inbound { .. } => {
                kv.put(b"phase".to_vec(), b"awaiting-time".to_vec());
                FoldOutput::with(vec![EffectRequest::new_with_family(
                    effect_ct::NOW,
                    String::new(),
                    None,
                    Timeliness::Interactive,
                )])
            }
            EventBody::EffectResult {
                result: EffectOutcome::Ok(Some(Payload::Inline(bytes))),
                ..
            } => {
                // The recorded instant arrived — record it and advance. The reducer never read the clock
                // itself; it only sees the result the kernel folded back.
                kv.put(b"started_at".to_vec(), bytes.to_vec());
                kv.put(b"phase".to_vec(), b"running".to_vec());
                FoldOutput::none()
            }
            _ => FoldOutput::none(),
        }
    }
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

/// Grant exactly the `Now` capability (deny-by-default: nothing else is permitted).
fn now_cap() -> Authorizer {
    Authorizer::new(vec![Capability {
        kind: EffectKind::Now,
        predicate: ResourcePredicate::Any,
    }])
}

#[tokio::test]
async fn agent_loop_runs_end_to_end_through_the_real_clock_executor() {
    let reducer = ClockAgent;
    let authz = now_cap();
    // The real executor, registered by canonical family string exactly as the Bedrock Model executor
    // will be alongside it.
    let mut exec =
        CompositeExecutor::new().with_effect(effect_ct::NOW, Box::new(ClockExecutor::new()));
    let mut session = Session::genesis(
        Hash::of(b"clock-agent-v1"),
        Hash::of(b"clock-agent-v1-nonce"),
    );

    session
        .deliver(inbound_go(), None, &reducer, &authz, &mut exec)
        .await
        .unwrap();

    // The loop closed: the reducer asked for the time, the REAL clock served it, the result folded back
    // and advanced the agent to `running` with a real recorded timestamp.
    assert_eq!(session.kv().get(b"phase"), Some(&b"running"[..]));
    let started = session
        .kv()
        .get(b"started_at")
        .expect("started_at recorded");
    // The Now payload is a u64 LE 8-byte nanoseconds-since-epoch integer (the ClockExecutor spec).
    let arr: [u8; 8] = started
        .try_into()
        .expect("started_at is 8 bytes (u64 LE nanos)");
    let ns = u64::from_le_bytes(arr);
    assert!(
        ns > 1_577_836_800_000_000_000,
        "a real epoch instant (nanos) was recorded: {ns}"
    );
    // Every effect settled; the agent is idle awaiting its next input (reactive, §9d).
    assert_eq!(session.open_effects(), 0);
}

#[tokio::test]
async fn the_recorded_instant_makes_the_run_replayable() {
    // §9c/§16c-S3: the clock read is non-deterministic, but its OUTCOME is recorded — so replaying the
    // log reconstructs the identical KV without ever touching the clock again. This is why a
    // world-touching executor doesn't break event-sourcing's replay-equivalence.
    // Both the live drive and the replay go through the async path: `deliver` for the run, and
    // `Session::replay` for the replay (it re-folds the recorded log through the Reducer, runs
    // no executor). The reducer is a native `Reducer` (the single reducer trait, ruling (b)).
    let reducer = ClockAgent;
    let mut exec =
        CompositeExecutor::new().with_effect(effect_ct::NOW, Box::new(ClockExecutor::new()));
    let mut session = Session::genesis(
        Hash::of(b"clock-agent-v1"),
        Hash::of(b"clock-agent-v1-nonce"),
    );
    session
        .deliver(inbound_go(), None, &ClockAgent, &now_cap(), &mut exec)
        .await
        .unwrap();

    let live_started = session.kv().get(b"started_at").unwrap().to_vec();

    // Replay the WHOLE log into a fresh session — no executor is consulted; the recorded EffectResult
    // supplies the instant. The reconstructed KV must be byte-identical to the live one.
    let replayed = Session::replay(session.log().to_vec(), &reducer)
        .await
        .unwrap();
    assert_eq!(replayed.kv().get(b"phase"), Some(&b"running"[..]));
    assert_eq!(
        replayed.kv().get(b"started_at").map(|b| b.to_vec()),
        Some(live_started),
        "replay reuses the recorded instant, reconstructing the identical KV"
    );
    assert_eq!(replayed.snapshot().kv_root, session.snapshot().kv_root);
}

#[tokio::test]
async fn a_now_effect_outside_the_grant_is_denied_never_reaching_the_clock() {
    // Deny-by-default (SEC-F1): an agent with NO `Now` capability that asks for the time is denied at
    // the gate — the real executor is never consulted, and the denial is on the log for audit (§10).
    let reducer = ClockAgent;
    let deny = Authorizer::deny_all();
    let mut exec =
        CompositeExecutor::new().with_effect(effect_ct::NOW, Box::new(ClockExecutor::new()));
    let mut session = Session::genesis(
        Hash::of(b"clock-agent-v1"),
        Hash::of(b"clock-agent-v1-nonce"),
    );
    session
        .deliver(inbound_go(), None, &reducer, &deny, &mut exec)
        .await
        .unwrap();

    // Never advanced to running (the time never came back), and the denial is logged.
    assert_ne!(session.kv().get(b"phase"), Some(&b"running"[..]));
    assert!(session
        .log()
        .iter()
        .any(|e| matches!(e.body, EventBody::AuthzDenied { .. })));
    assert_eq!(session.open_effects(), 0);
}
