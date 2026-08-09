//! The `Now` executor — read the system wall clock (§9c).
//!
//! A reducer never reads the clock directly (that would be non-deterministic and unreplayable). It
//! emits a `Now` effect; the kernel authorizes + dispatches it, THIS executor reads the real clock, and
//! the kernel folds the instant back as an `EffectResult` — which is RECORDED in the log. Replay then
//! reuses that recorded instant, so the fold stays a pure function of the log (§16c-S3 determinism lives
//! in the log, not the executor). This is the "reads-are-effects" pattern the design leans on for the
//! clock: the executor is non-deterministic, the *recorded outcome* makes the session replayable.
//!
//! This executor EMITS the raw reading as nanoseconds since the Unix epoch, a **u64 little-endian 8-byte
//! integer** (`ns.to_le_bytes()`) — the operator's binary-ns directive, and the exact shape the kernel's
//! Now-clamp reads. **Monotonicity is the KERNEL's job, not this executor's:** the kernel's
//! `clamp_now_outcome` clamps a Now result to `max(raw, last_now+1ns)` and records the clamped value, so
//! the RECORDED `Now` sequence is strictly increasing and replay-deterministic. This executor just hands
//! over the raw clock read in the clamp-compatible 8-byte LE shape; two successive raw reads here may be
//! equal or (under an NTP step) even go backwards — that's fine, the kernel clamp is what makes the
//! recorded sequence monotonic. A reducer decodes the recorded value with
//! `u64::from_le_bytes(bytes.try_into()?)`. (`Now`'s target is empty; a capability gates it by kind, e.g.
//! `Capability { kind: Now, predicate: Any }`.)

use cdz_kernel::effect::{effect_ct, EffectId, EffectRequest, Payload};
use cdz_kernel::event::EffectOutcome;
use cdz_kernel::executor::Executor;
use cdz_kernel::hash::Hash;
use std::time::{SystemTime, UNIX_EPOCH};

/// Performs `Now` effects by reading the system wall clock. Hermetic (no network, no credentials), so
/// it needs no feature gate. Non-`Now` kinds are an observable `Err` — this is a single-KIND executor;
/// route multiple kinds by registering it under `Now` in a
/// [`cdz_kernel::executor::CompositeExecutor`].
///
/// **Idempotency (§16c-S1):** reading the clock is a pure read with no external side effect, so the
/// `idempotency_key` is ignored — a re-driven `Now` after a crash simply reads the (later) clock again,
/// and only the recorded result matters for replay.
#[derive(Default)]
pub struct ClockExecutor;

impl ClockExecutor {
    pub fn new() -> Self {
        ClockExecutor
    }
}

// Native `Executor` (not driven via the `SyncExecutorAsAsync` blanket): the async kernel loop calls
// `perform`. Reading the clock is synchronous (no `.await` point) — an executor that touches real
// I/O (Model/Http) awaits its transport, but the clock read is instantaneous — so `perform` here
// has no await. It impls `Executor` directly (dropping the old sync `Executor` impl) so it can sit in
// an `CompositeExecutor` and not overlap the blanket (§ all-async, step-5).
#[async_trait::async_trait(?Send)]
impl Executor for ClockExecutor {
    async fn perform(
        &mut self,
        _id: EffectId,
        req: &EffectRequest,
        _idempotency_key: Hash,
    ) -> EffectOutcome {
        // Key the guard on the effect FAMILY STRING (seq-39 / effect-schema slice 2), not the EffectKind
        // enum — the same decision the router and authz make, and the same shape the Model/Http executors
        // already use. Decouples this executor from the enum ahead of its retirement; matches_family is
        // the one-source-of-truth family compare.
        if !req.content_type.matches_family(effect_ct::NOW) {
            // A wrong-family request is structural — PERMANENT, a supervisor must not retry it (§17: an
            // observable Err, never a panic).
            return EffectOutcome::err(format!(
                "ClockExecutor only handles the {} family, got {}",
                effect_ct::NOW,
                req.content_type.family
            ));
        }
        // Nanoseconds since the Unix epoch as a u64 LITTLE-ENDIAN 8-byte integer (operator binary-ns
        // directive + the kernel monotonic-clamp payload spec): the kernel's clamp reads exactly this
        // 8-byte LE u64, clamps to max(raw, last_now+1ns), and records the clamped value (so the recorded
        // Now sequence is strictly increasing + replay-deterministic). A clock set before the epoch is a
        // host misconfiguration, not a transient blip — surfaced as a PERMANENT Err (never a panic, §17).
        //
        // `as u64` truncates ns from the `u128` duration; u64 ns overflows only past ~year 2554, far
        // beyond any real deadline, so the cast is safe for every meaningful clock reading.
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(dur) => {
                let ns = dur.as_nanos() as u64;
                EffectOutcome::Ok(Some(Payload::Inline(ns.to_le_bytes().to_vec().into())))
            }
            Err(e) => EffectOutcome::err(format!("system clock is before the Unix epoch: {e}")),
        }
    }

    /// This single-kind executor serves exactly the `Now` family — the capability-manifest mechanism
    /// dimension when it's used bare as a `dyn Executor` (in a `CompositeExecutor` the composite's own
    /// `by_family` override answers instead). Overrides the trait's fail-safe `false` default.
    fn handles_family(&self, family: &str) -> bool {
        family == effect_ct::NOW
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_kernel::effect::{EffectKind, Timeliness};
    use cdz_kernel::event::Retryability;

    fn req(kind: EffectKind, target: &str) -> EffectRequest {
        EffectRequest::new(kind, target, None, Timeliness::Interactive)
    }

    /// Decode a `Now` Ok payload as the u64 LE nanos the executor emits (the spec the kernel clamp reads).
    fn decode_ns(outcome: EffectOutcome) -> u64 {
        match outcome {
            EffectOutcome::Ok(Some(Payload::Inline(bytes))) => {
                let arr: [u8; 8] = bytes
                    .as_ref()
                    .try_into()
                    .expect("Now payload is exactly 8 bytes");
                u64::from_le_bytes(arr)
            }
            other => panic!("expected Ok(Inline(8-byte LE u64 nanos)), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn now_returns_an_8_byte_le_u64_nanos_timestamp() {
        let mut exec = ClockExecutor::new();
        let ns = decode_ns(
            exec.perform(EffectId(0), &req(EffectKind::Now, ""), Hash::of(b"k"))
                .await,
        );
        // A sane lower bound: well after 2020 (1_577_836_800_000_000_000 ns = 2020-01-01) — proves it's a
        // real epoch timestamp in NANOSECONDS, not ms/zero/garbage.
        assert!(
            ns > 1_577_836_800_000_000_000,
            "clock reads a real epoch time in nanos: {ns}"
        );
    }

    #[tokio::test]
    async fn now_payload_is_exactly_8_bytes() {
        // The kernel clamp only clamps an 8-byte LE u64 (anything else passes through un-clamped), so the
        // width is load-bearing for the monotonic guarantee.
        let mut exec = ClockExecutor::new();
        match exec
            .perform(EffectId(0), &req(EffectKind::Now, ""), Hash::of(b"k"))
            .await
        {
            EffectOutcome::Ok(Some(Payload::Inline(bytes))) => {
                assert_eq!(
                    bytes.len(),
                    8,
                    "Now payload must be exactly 8 bytes (u64 LE nanos)"
                )
            }
            other => panic!("expected Ok(Inline), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn two_reads_both_return_sane_epoch_nanos() {
        // This executor emits the RAW wall-clock read — it does NOT enforce monotonicity (that's the
        // kernel clamp's job). So two successive reads must NOT be asserted b >= a: the wall clock isn't
        // monotonic (an NTP step can move it backwards mid-test), which would be a latent CI flake. Assert
        // only what this executor guarantees — each read is a sane epoch-nanos value.
        let mut exec = ClockExecutor::new();
        let a = decode_ns(
            exec.perform(EffectId(0), &req(EffectKind::Now, ""), Hash::of(b"k"))
                .await,
        );
        let b = decode_ns(
            exec.perform(EffectId(0), &req(EffectKind::Now, ""), Hash::of(b"k"))
                .await,
        );
        assert!(
            a > 1_577_836_800_000_000_000,
            "first read is a real epoch nanos: {a}"
        );
        assert!(
            b > 1_577_836_800_000_000_000,
            "second read is a real epoch nanos: {b}"
        );
    }

    #[tokio::test]
    async fn a_non_now_family_is_an_observable_err_not_a_panic() {
        // This is a single-family executor; a wrong family is an observable Err (§9d), never a panic.
        let mut exec = ClockExecutor::new();
        match exec
            .perform(
                EffectId(0),
                &req(EffectKind::Http, "https://x/"),
                Hash::of(b"k"),
            )
            .await
        {
            EffectOutcome::Err {
                message,
                retryability,
            } => {
                assert!(
                    message.contains(effect_ct::NOW),
                    "err names the handled family: {message}"
                );
                assert_eq!(retryability, Retryability::Permanent, "{message}");
            }
            other => panic!("expected Err for a non-Now kind, got {other:?}"),
        }
    }

    #[test]
    fn handles_only_the_now_family() {
        // Bare-leaf mechanism dimension: serves Now, nothing else (the trait default false otherwise).
        let exec = ClockExecutor::new();
        assert!(exec.handles_family(effect_ct::NOW));
        assert!(!exec.handles_family(effect_ct::HTTP));
        assert!(!exec.handles_family(effect_ct::MODEL));
        assert!(!exec.handles_family("embedding"));
    }

    // ---- an agent RUNS end-to-end through the real ClockExecutor (converted from the deleted agent_runs_e2e
    // integration test, operator no-integration-tests mandate — hermetic: a Session + a Rust reducer + the
    // REAL ClockExecutor, no wasm/network). The milestone this crate exists for in its hermetic form: a
    // reducer drives the kernel loop (fold → authorize → dispatch → EXECUTE → fold-result) and the executor
    // doing the executing is a genuine world-touching one (reads the wall clock), NOT a test recorder. The
    // `Now` effect is the hermetic proof point; the Bedrock Model executor slots into the SAME wiring. ----
    use cdz_kernel::authz::Authorizer;
    use cdz_kernel::effect::{Capability, ResourcePredicate};
    use cdz_kernel::event::{ContentType, Event, EventBody};
    use cdz_kernel::executor::CompositeExecutor;
    use cdz_kernel::kernel::Session;
    use cdz_kernel::kv::Kv;
    use cdz_kernel::reducer::{FoldOutput, Reducer};

    /// A minimal but real agent: on an inbound "go" it asks the kernel for the current time (a `Now` effect);
    /// when the recorded instant comes back it stashes it in KV under `started_at` and marks itself `running`.
    /// This is the fold→effect→result→continuation loop every agent runs — here the effect is served by a REAL
    /// clock executor, so the loop closes against the world.
    struct ClockAgent;
    #[async_trait::async_trait(?Send)]
    impl Reducer for ClockAgent {
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
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

    fn clock_go() -> EventBody {
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
        let mut reducer = ClockAgent;
        let authz = now_cap();
        // The real executor, registered by canonical family string exactly as the Bedrock Model executor will
        // be alongside it.
        let mut exec =
            CompositeExecutor::new().with_effect(effect_ct::NOW, Box::new(ClockExecutor::new()));
        let mut session = Session::genesis(
            Hash::of(b"clock-agent-v1"),
            Hash::of(b"clock-agent-v1-nonce"),
        );

        session
            .deliver(clock_go(), None, &mut reducer, &authz, &mut exec)
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
        // §9c/§16c-S3: the clock read is non-deterministic, but its OUTCOME is recorded — so replaying the log
        // reconstructs the identical KV without ever touching the clock again. This is why a world-touching
        // executor doesn't break event-sourcing's replay-equivalence.
        let mut reducer = ClockAgent;
        let mut exec =
            CompositeExecutor::new().with_effect(effect_ct::NOW, Box::new(ClockExecutor::new()));
        let mut session = Session::genesis(
            Hash::of(b"clock-agent-v1"),
            Hash::of(b"clock-agent-v1-nonce"),
        );
        session
            .deliver(clock_go(), None, &mut ClockAgent, &now_cap(), &mut exec)
            .await
            .unwrap();

        let live_started = session.kv().get(b"started_at").unwrap().to_vec();

        // Replay the WHOLE log into a fresh session — no executor is consulted; the recorded EffectResult
        // supplies the instant. The reconstructed KV must be byte-identical to the live one.
        let replayed = Session::replay(session.log().to_vec(), &mut reducer)
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
        // Deny-by-default (SEC-F1): an agent with NO `Now` capability that asks for the time is denied at the
        // gate — the real executor is never consulted, and the denial is on the log for audit (§10).
        let mut reducer = ClockAgent;
        let deny = Authorizer::deny_all();
        let mut exec =
            CompositeExecutor::new().with_effect(effect_ct::NOW, Box::new(ClockExecutor::new()));
        let mut session = Session::genesis(
            Hash::of(b"clock-agent-v1"),
            Hash::of(b"clock-agent-v1-nonce"),
        );
        session
            .deliver(clock_go(), None, &mut reducer, &deny, &mut exec)
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
}
