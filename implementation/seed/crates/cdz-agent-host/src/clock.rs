//! The `Now` executor — read the system wall clock (§9c).
//!
//! A reducer never reads the clock directly (that would be non-deterministic and unreplayable). It
//! emits a `Now` effect; the kernel authorizes + dispatches it, THIS executor reads the real clock, and
//! the kernel folds the instant back as an `EffectResult` — which is RECORDED in the log. Replay then
//! reuses that recorded instant, so the fold stays a pure function of the log (§16c-S3 determinism lives
//! in the log, not the executor). This is the "reads-are-effects" pattern the design leans on for the
//! clock: the executor is non-deterministic, the *recorded outcome* makes the session replayable.
//!
//! The result payload is the nanoseconds since the Unix epoch as a **u64 little-endian 8-byte integer**
//! (`ns.to_le_bytes()`) — the shape the kernel's monotonic clamp reads (it clamps the raw reading to
//! `max(raw, last_now+1ns)` and records the clamped value, so the recorded `Now` sequence is strictly
//! increasing and replay-deterministic — operator binary-ns directive). A reducer decodes it with
//! `u64::from_le_bytes(bytes.try_into()?)`. (`Now`'s target is empty; a capability gates it by kind,
//! e.g. `Capability { kind: Now, predicate: Any }`.)

use crate::retry;
use cdz_kernel::effect::{EffectKind, EffectRequest, Payload};
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

impl Executor for ClockExecutor {
    fn perform(&mut self, req: &EffectRequest, _idempotency_key: Hash) -> EffectOutcome {
        if req.kind != EffectKind::Now {
            // A wrong-kind request is structural — PERMANENT, a supervisor must not retry it (§17: an
            // observable Err, never a panic).
            return EffectOutcome::Err(retry::permanent(format!(
                "ClockExecutor only handles Now effects, got {:?}",
                req.kind
            )));
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
            Err(e) => EffectOutcome::Err(retry::permanent(format!(
                "system clock is before the Unix epoch: {e}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_kernel::effect::Timeliness;

    fn req(kind: EffectKind, target: &str) -> EffectRequest {
        EffectRequest {
            kind,
            target: target.to_string(),
            payload: None,
            timeliness: Timeliness::Interactive,
        }
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

    #[test]
    fn now_returns_an_8_byte_le_u64_nanos_timestamp() {
        let mut exec = ClockExecutor::new();
        let ns = decode_ns(exec.perform(&req(EffectKind::Now, ""), Hash::of(b"k")));
        // A sane lower bound: well after 2020 (1_577_836_800_000_000_000 ns = 2020-01-01) — proves it's a
        // real epoch timestamp in NANOSECONDS, not ms/zero/garbage.
        assert!(
            ns > 1_577_836_800_000_000_000,
            "clock reads a real epoch time in nanos: {ns}"
        );
    }

    #[test]
    fn now_payload_is_exactly_8_bytes() {
        // The kernel clamp only clamps an 8-byte LE u64 (anything else passes through un-clamped), so the
        // width is load-bearing for the monotonic guarantee.
        let mut exec = ClockExecutor::new();
        match exec.perform(&req(EffectKind::Now, ""), Hash::of(b"k")) {
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

    #[test]
    fn now_is_monotonic_across_two_reads() {
        // Two successive reads must not go backwards (wall clock is non-decreasing over a test's span).
        let mut exec = ClockExecutor::new();
        let a = decode_ns(exec.perform(&req(EffectKind::Now, ""), Hash::of(b"k")));
        let b = decode_ns(exec.perform(&req(EffectKind::Now, ""), Hash::of(b"k")));
        assert!(b >= a, "second read {b} must not precede the first {a}");
    }

    #[test]
    fn non_now_kind_is_an_observable_err_not_a_panic() {
        // This is a single-kind executor; a wrong kind is an observable Err (§9d), never a panic.
        let mut exec = ClockExecutor::new();
        match exec.perform(&req(EffectKind::Http, "https://x/"), Hash::of(b"k")) {
            EffectOutcome::Err(msg) => {
                assert!(msg.contains("Now"), "err names the handled kind: {msg}");
                assert_eq!(
                    retry::classify(&msg),
                    retry::Retryability::Permanent,
                    "{msg}"
                );
            }
            other => panic!("expected Err for a non-Now kind, got {other:?}"),
        }
    }
}
