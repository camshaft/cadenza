//! The reducer contract — the kernel↔reducer interface (§16c gap A, the load-bearing ABI).
//!
//! A reducer is a pure fold: given the event being applied and mutable access to the session KV, it
//! returns the effects it wants performed. It holds NO state between calls (§4) — everything it needs
//! to continue lives in KV. In v0 this is a Rust trait so the whole kernel loop is testable before
//! wasmtime lands; the trait's shape IS the future wasm component interface (WIT), so getting it right
//! here is exactly gap A.
//!
//! Determinism contract (§3, §16c-S3): a reducer MUST be a pure function of `(event, kv)`. It may not
//! read the clock, network, or entropy directly — those enter only as effects whose results arrive as
//! later events. The wasm sandbox will enforce this structurally; the trait documents it.
//!
//! Effect-await pattern (§16c-S4): a reducer cannot "await" — the call returns and the instance is
//! gone. To continue after an effect, it emits the effect (getting an [`EffectId`] back via the
//! kernel) and stores its continuation in KV keyed by that id; when the result event arrives it looks
//! the continuation up and resumes. The kernel enforces timeout-cancels semantics so a continuation is
//! resumed at most once (by a result OR a timeout, never both).

use crate::effect::EffectRequest;
use crate::event::{Event, EventBody};
use crate::kv::Kv;

/// What a reducer asks the kernel to do after folding one event. Effects are *requests* (§5): the
/// kernel authorizes, assigns ids, dispatches, and folds results back as later events.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct FoldOutput {
    /// Effects to perform, in emission order. The kernel assigns each an `EffectId` (in this order)
    /// and appends a durable `Dispatched` event before routing (§16c-S1).
    pub effects: Vec<EffectRequest>,
}

impl FoldOutput {
    pub fn none() -> Self {
        FoldOutput::default()
    }

    pub fn with(effects: Vec<EffectRequest>) -> Self {
        FoldOutput { effects }
    }
}

/// The reducer interface. Implementors are pure folds (see module docs).
pub trait Reducer {
    /// Fold one event into the KV, returning requested effects. Called once per event, in log order,
    /// on a fresh conceptual instance (no cross-call state outside `kv`).
    ///
    /// Totality (§17 "can't-brick"): this must not panic for any input. A well-behaved reducer that
    /// sees an event it doesn't understand should ignore it (return `FoldOutput::none()`), not crash.
    /// The kernel treats a panic as a fold failure (a future ABI concern, §16c-A); v0 trait impls are
    /// expected to be total.
    fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput;
}

/// A trivial reducer used by kernel-loop tests: it ignores everything and emits nothing. Real reducers
/// (Rust-wasm first, Cadenza-native later) implement domain behavior.
pub struct InertReducer;

impl Reducer for InertReducer {
    fn fold(&self, _event: &Event, _kv: &mut Kv) -> FoldOutput {
        FoldOutput::none()
    }
}

/// Convenience: does this event body carry an effect id the reducer might have a continuation for?
/// (Used by the wasmtime dispatch loop to know when a reducer resume is relevant.) Kept here so the
/// correlation mapping lives beside the contract it serves.
pub fn resumes_effect(body: &EventBody) -> Option<crate::effect::EffectId> {
    match body {
        EventBody::EffectResult { id, .. } => Some(*id),
        EventBody::TimerFired { id, .. } => Some(*id),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect::{EffectId, Payload};
    use crate::event::EffectOutcome;

    #[test]
    fn resumes_effect_recognizes_result_and_timer() {
        let result = EventBody::EffectResult {
            id: EffectId(7),
            result: EffectOutcome::Ok(None),
        };
        let timer = EventBody::TimerFired {
            id: EffectId(9),
            fired_ms: 1,
        };
        let inbound = EventBody::Closed {
            outcome: Payload::Inline(vec![]),
        };
        assert_eq!(resumes_effect(&result), Some(EffectId(7)));
        assert_eq!(resumes_effect(&timer), Some(EffectId(9)));
        assert_eq!(resumes_effect(&inbound), None);
    }
}
