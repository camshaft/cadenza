//! End-to-end (hermetic): the SELF-HOSTING TICK-LOOP — an agent-as-reducer that RE-ARMS a timer each tick,
//! driven to completion by the real [`AsyncAgentHost`] loop (GAP-5, the self-hosting-harness endgame). This
//! is the host-side demonstration that the harness can run the fleet.rs re-issue shape ITSELF: today
//! fleet.rs re-issues a `/loop` tick prompt to each tmux window on a cron; here a session re-arms its own
//! `timer` effect each tick, so the loop re-drives it — the SAME "wake me again next interval" pattern, but
//! in userspace-on-the-harness (a reducer + the existing timer/loop mechanism), NO new host machinery.
//!
//! The ROLE reducer (a userspace program, exactly where the operator standing-order wants the tick POLICY):
//! - on the initial inbound (the "start your loop" kick) → arm a `timer`.
//! - on each `TimerFired` → do the tick's work (here: bump a durable tick counter in KV) + RE-ARM the timer
//!   for the next tick, UNTIL a stop condition (tick budget reached) — then stop re-arming, and the loop,
//!   with no armed timer + a closed inbox, drains to a clean shutdown.
//!
//! The host provides only MECHANISM (the timer effect + the loop's timer wheel that fires it + re-drives the
//! session); the tick CADENCE, WORK, and STOP are the reducer's policy. That's the fleet-loop, self-hosted.

use cdz_agent_host::{AgentHost, AsyncAgentHost, HostedSession, SessionId};
use cdz_kernel::authz::Authorizer;
use cdz_kernel::effect::{
    effect_ct, Capability, EffectKind, EffectRequest, Payload, ResourcePredicate, Timeliness,
};
use cdz_kernel::event::{ContentType, Event, EventBody};
use cdz_kernel::executor::CompositeExecutor;
use cdz_kernel::hash::Hash;
use cdz_kernel::kv::Kv;
use cdz_kernel::reducer::{FoldOutput, Reducer};

/// How many ticks the role runs before it stops re-arming (its self-imposed budget — a real role would loop
/// until a stop-file / operator signal; a bounded budget keeps the test deterministic + terminating).
const TICK_BUDGET: u64 = 5;
/// The tick interval the role arms (ms). Value is irrelevant to the test — `now_ms` is driven far past it so
/// each re-armed timer is immediately due, cycling the loop fast to quiescence.
const TICK_INTERVAL_MS: u64 = 1000;

/// A self-re-arming TICK-LOOP role reducer (the fleet.rs re-issue shape, in userspace). Stores its tick count
/// in KV (durable — a recovered session resumes its loop). Stops re-arming at the budget so the loop drains.
struct TickLoopRole;

impl TickLoopRole {
    fn arm_tick() -> EffectRequest {
        EffectRequest::new_with_family(
            effect_ct::TIMER,
            TICK_INTERVAL_MS.to_string(),
            None,
            Timeliness::Interactive,
        )
    }
    fn ticks(kv: &Kv) -> u64 {
        kv.get(b"ticks")
            .and_then(|b| std::str::from_utf8(b).ok())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    }
}

#[async_trait::async_trait(?Send)]
impl Reducer for TickLoopRole {
    async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
        match &event.body {
            // The kick: start the loop by arming the first tick.
            EventBody::Inbound { .. } => FoldOutput::with(vec![Self::arm_tick()]),
            // Each tick fires: do the tick's work (bump the counter) + re-arm for the next — until budget.
            EventBody::TimerFired { .. } => {
                let n = Self::ticks(kv) + 1;
                kv.put(b"ticks".to_vec(), n.to_string().into_bytes());
                if n < TICK_BUDGET {
                    // Re-arm: the loop's timer wheel will fire the next tick → this fold runs again. THE
                    // self-perpetuating loop — the reducer decides to continue (policy), the host just fires.
                    FoldOutput::with(vec![Self::arm_tick()])
                } else {
                    // Budget reached → stop re-arming. With no armed timer + a closed inbox, the loop drains.
                    kv.put(b"done".to_vec(), b"1".to_vec());
                    FoldOutput::none()
                }
            }
            _ => FoldOutput::none(),
        }
    }
}

fn kick() -> EventBody {
    EventBody::Inbound {
        content_type: ContentType {
            family: "message".into(),
            version: 1,
        },
        payload: Payload::Inline(b"start-your-loop".to_vec().into()),
    }
}

#[tokio::test]
async fn agent_reducer_runs_a_self_rearming_tick_loop_through_the_host_loop() {
    // Grant only `timer` (the role re-arms it each tick); deny-by-default otherwise.
    let authz = Authorizer::new(vec![Capability {
        kind: EffectKind::Timer,
        predicate: ResourcePredicate::Any,
    }]);
    let mut host = AgentHost::new();
    let id = SessionId::new("tick-role");
    host.spawn(
        id.clone(),
        HostedSession::genesis(
            Hash::of(b"tick-role-v1"),
            Box::new(TickLoopRole),
            Box::new(authz),
            CompositeExecutor::new(),
        ),
    );
    // Kick the loop BEFORE running so the first timer is armed at loop entry.
    host.deliver(&id, kick(), None).await;
    assert_eq!(
        TickLoopRole::ticks(host.get(&id).unwrap().session().kv()),
        0,
        "no ticks yet — the timer is armed but hasn't fired"
    );

    // Run the loop with the clock pinned FAR PAST any deadline, so each re-armed timer is immediately due:
    // the loop fires the tick → the role re-arms → fires again → … until the role stops re-arming at the
    // budget, at which point (no armed timer, inbox closed) the loop drains and `run` returns. No shutdown
    // signal needed — the self-stopping role IS the termination (the fleet-loop's stop condition).
    let async_host = AsyncAgentHost::new(host);
    let (_sd_tx, sd_rx) = tokio::sync::oneshot::channel();
    let host = async_host
        .run(sd_rx, || u64::MAX)
        .await
        .expect("the loop drains cleanly once the role stops re-arming");

    // The role ran exactly TICK_BUDGET ticks, then stopped — a self-hosted tick-loop, driven to completion by
    // the host's timer wheel with no external re-issue (the fleet.rs cron replaced by the reducer's re-arm).
    let kv = host.get(&id).unwrap();
    let kv = kv.session().kv();
    assert_eq!(
        TickLoopRole::ticks(kv),
        TICK_BUDGET,
        "the role ran exactly its tick budget, re-arming each tick through the host loop"
    );
    assert_eq!(
        kv.get(b"done").map(|v| v.to_vec()),
        Some(b"1".to_vec()),
        "the role reached its stop condition + stopped re-arming (the loop then drained)"
    );
}
