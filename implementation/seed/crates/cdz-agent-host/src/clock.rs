//! The `Now` executor — read the system wall clock (§9c).
//!
//! A reducer never reads the clock directly (that would be non-deterministic and unreplayable). It
//! emits a `Now` effect; the kernel authorizes + dispatches it, THIS executor reads the real clock, and
//! the kernel folds the instant back as an `EffectResult` — which is RECORDED in the log. Replay then
//! reuses that recorded instant, so the fold stays a pure function of the log (§16c-S3 determinism lives
//! in the log, not the executor). This is the "reads-are-effects" pattern the design leans on for the
//! clock: the executor is non-deterministic, the *recorded outcome* makes the session replayable.
//!
//! The result payload is the milliseconds since the Unix epoch as ASCII decimal — a stable,
//! self-describing, endianness-free representation a reducer parses with `str::parse`. (`Now`'s target
//! is empty; a capability gates it by kind, e.g. `Capability { kind: Now, predicate: Any }`.)

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
            return EffectOutcome::Err(format!(
                "ClockExecutor only handles Now effects, got {:?}",
                req.kind
            ));
        }
        // Milliseconds since the Unix epoch, as ASCII decimal. A clock set before the epoch is not a
        // panic (§17 totality) — surface it as an observable Err the reducer folds.
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(dur) => {
                let ms = dur.as_millis();
                // `.into()` builds the kernel's `Payload::Inline`: identity for a `Vec<u8>` inner and the
                // `From<Vec<u8>>` freeze once the kernel flipped `Inline` to ref-counted `bytes::Bytes`
                // (operator perf directive) — so this one call site compiles across the flip.
                EffectOutcome::Ok(Some(Payload::Inline(ms.to_string().into_bytes().into())))
            }
            Err(e) => EffectOutcome::Err(format!("system clock is before the Unix epoch: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(kind: EffectKind, target: &str) -> EffectRequest {
        EffectRequest {
            kind,
            target: target.to_string(),
            payload: None,
        }
    }

    #[test]
    fn now_returns_a_parseable_millis_timestamp() {
        let mut exec = ClockExecutor::new();
        match exec.perform(&req(EffectKind::Now, ""), Hash::of(b"k")) {
            EffectOutcome::Ok(Some(Payload::Inline(bytes))) => {
                // `.to_vec()` copies the payload's bytes into an owned `Vec<u8>` for `from_utf8` — works
                // whether `Inline` holds a `Vec<u8>` or (post-flip) `bytes::Bytes` (both deref to `&[u8]`).
                let text = String::from_utf8(bytes.to_vec()).expect("ascii decimal");
                let ms: u128 = text.parse().expect("parses as millis");
                // A sane lower bound: well after 2020 (1_577_836_800_000 ms = 2020-01-01) — proves it's a
                // real epoch timestamp, not a zero/garbage value.
                assert!(
                    ms > 1_577_836_800_000,
                    "clock reads a real epoch time: {ms}"
                );
            }
            other => panic!("expected Ok(Inline(millis)), got {other:?}"),
        }
    }

    #[test]
    fn now_is_monotonic_across_two_reads() {
        // Two successive reads must not go backwards (wall clock is non-decreasing over a test's span).
        let mut exec = ClockExecutor::new();
        let read = |e: &mut ClockExecutor| -> u128 {
            match e.perform(&req(EffectKind::Now, ""), Hash::of(b"k")) {
                EffectOutcome::Ok(Some(Payload::Inline(b))) => {
                    String::from_utf8(b.to_vec()).unwrap().parse().unwrap()
                }
                other => panic!("expected Ok millis, got {other:?}"),
            }
        };
        let a = read(&mut exec);
        let b = read(&mut exec);
        assert!(b >= a, "second read {b} must not precede the first {a}");
    }

    #[test]
    fn non_now_kind_is_an_observable_err_not_a_panic() {
        // This is a single-kind executor; a wrong kind is an observable Err (§9d), never a panic.
        let mut exec = ClockExecutor::new();
        match exec.perform(&req(EffectKind::Http, "https://x/"), Hash::of(b"k")) {
            EffectOutcome::Err(msg) => {
                assert!(msg.contains("Now"), "err names the handled kind: {msg}")
            }
            other => panic!("expected Err for a non-Now kind, got {other:?}"),
        }
    }
}
