//! The reducer contract — the kernel↔reducer interface (§16c gap A, the load-bearing ABI).
//!
//! A reducer is a pure fold: given the event being applied and mutable access to the session KV, it
//! returns the effects it wants performed. It holds NO state between calls (§4) — everything it needs
//! to continue lives in KV. In v0 this is a Rust trait so the whole kernel loop is testable before
//! wasmtime lands; the trait's shape MIRRORS the wasm component-model contract in `wit/reducer.wit`
//! (the `cadenza:agent-kernel` reducer world — `fold.apply` export + `kv` import), which per operator
//! directive §19b is the REAL boundary. This Rust trait is the INTERIM until the WIT world + the
//! wasmtime component host land (the next slice); getting the shape right here is exactly §16c gap A.
//!
//! Determinism contract (§3, §16c-S3): a reducer MUST be a pure function of `(event, kv)`. It may not
//! read the clock, network, or entropy directly — those enter only as effects whose results arrive as
//! later events. The wasm sandbox will enforce this structurally; the trait documents it.
//!
//! Effect-await pattern (§16c-S4): a reducer cannot "await" — the call returns and the instance is
//! gone. To continue after an effect, it emits the effect (getting an [`crate::effect::EffectId`] back via the
//! kernel) and stores its continuation in KV keyed by that id; when the result event arrives it looks
//! the continuation up and resumes. The kernel enforces timeout-cancels semantics so a continuation is
//! resumed at most once (by a result OR a timeout, never both).

use crate::effect::EffectRequest;
use crate::event::{CloseOutcome, Event, EventBody};
use crate::kv::Kv;

/// One effect a fold emits: the request itself plus the reducer's OWN optional continuation token for it
/// (§19e). This is the reducer→kernel HANDOFF type — distinct from the [`EffectRequest`] WIRE type (which
/// the executor sees and which stays `{kind, target, payload}`, no correlation — §19e reject-C). The
/// token is the natural home HERE because this handoff is exactly where the guest's correlation must
/// reach the kernel's `Dispatched`-frame builder (drive records it in the frame so the `EffectId ↔ token`
/// map rebuilds from the log on recovery). A NAMED field (not a bare tuple) per the strong-typing rule:
/// the `Option` says what it IS (a correlation token) at every site, and the struct grows cleanly if a
/// third per-effect handoff field ever appears.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Effect {
    pub request: EffectRequest,
    /// The reducer's opaque continuation token, or `None` if it correlates by kernel `EffectId` (the
    /// in-process Rust `Reducer` trait). A WASM `ComponentReducer` sets it to the guest's `correlation`.
    pub token: Option<Vec<u8>>,
}

impl Effect {
    /// A token-free effect (the common case: a Rust reducer that correlates by `EffectId`).
    pub fn new(request: EffectRequest) -> Self {
        Effect {
            request,
            token: None,
        }
    }
}

/// What a reducer asks the kernel to do after folding one event. Effects are *requests* (§5): the
/// kernel authorizes, assigns ids, dispatches, and folds results back as later events.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct FoldOutput {
    /// Effects to perform, in emission order. The kernel assigns each an `EffectId` (in this order)
    /// and appends a durable `Dispatched` event before routing (§16c-S1). Each carries its optional
    /// continuation token (§19e — see [`Effect`]).
    pub effects: Vec<Effect>,
    /// Set when the fold FAILED rather than completing — a WASM guest trap / fuel-exhaustion /
    /// instantiate failure (§17 totality: the kernel can't let a bad reducer panic the loop, so a
    /// failure is surfaced as data, not a crash). `None` = a normal fold. When `Some(reason)`, the
    /// kernel records a first-class [`crate::event::EventBody::FoldFailed`] LOG event (the error-
    /// resilience / supervision direction — a failure is CAPTURED on the log for a supervisor to see,
    /// NOT silently swallowed into an empty fold "into the void"). A failed fold carries no effects.
    pub failure: Option<String>,
    /// Set when the reducer signals CLEAN SELF-COMPLETION (§6 supervision): the session has reached its
    /// goal and should CLOSE, carrying a structured [`CloseOutcome`] (success-with-payload vs
    /// failure-with-reason) the kernel appends as a durable [`crate::event::EventBody::Closed`] (which sets
    /// `is_closed()`). `None` = a normal ongoing fold. This is the SYMMETRIC sibling of `failure`: `failure`
    /// captures an involuntary fold error as `FoldFailed`; `close` is the reducer VOLUNTARILY ending ITSELF
    /// (self-close — distinct from `Terminated`, which is another session ending it via `lifecycle/terminate`).
    /// Terminal like `failure`: a closing fold's effects are ignored (the session is ending). The trigger for
    /// `EventBody::Closed`, which the kernel otherwise only recognizes on recovery — a session reaches it by a
    /// reducer returning `close = Some(..)`.
    pub close: Option<CloseOutcome>,
}

impl FoldOutput {
    pub fn none() -> Self {
        FoldOutput::default()
    }

    /// Build from token-free effect requests (the ergonomic path for a Rust reducer that correlates by
    /// `EffectId`): each request becomes an [`Effect`] with `token: None`.
    pub fn with(effects: Vec<EffectRequest>) -> Self {
        FoldOutput {
            effects: effects.into_iter().map(Effect::new).collect(),
            failure: None,
            close: None,
        }
    }

    /// Build from fully-specified effects (each with its own token) — the path a WASM `ComponentReducer`
    /// adapter uses to carry the guest's per-effect correlation token.
    pub fn with_effects(effects: Vec<Effect>) -> Self {
        FoldOutput {
            effects,
            failure: None,
            close: None,
        }
    }

    /// A FAILED fold (§17 / error-resilience): no effects, carrying the reason the fold couldn't run
    /// (guest trap / fuel-exhaustion / instantiate failure). The kernel records it as a `FoldFailed`
    /// log event so a supervisor can observe the failure rather than it vanishing into a silent empty
    /// fold.
    pub fn failed(reason: impl Into<String>) -> Self {
        FoldOutput {
            effects: Vec::new(),
            failure: Some(reason.into()),
            close: None,
        }
    }

    /// A CLEAN SELF-COMPLETION fold (§6 supervision): no effects, signaling the session should CLOSE with the
    /// given [`CloseOutcome`]. The kernel appends a durable `Closed{outcome}` (setting `is_closed()`) — the
    /// symmetric sibling of [`FoldOutput::failed`]. This is how a reducer VOLUNTARILY ends its own session
    /// (self-close), distinct from being `Terminated` by another via `lifecycle/terminate`.
    pub fn close(outcome: CloseOutcome) -> Self {
        FoldOutput {
            effects: Vec::new(),
            failure: None,
            close: Some(outcome),
        }
    }
}

/// The reducer interface — a pure ASYNC fold (see module docs). There is ONE reducer trait, and it is
/// async (operator ruling: "one async trait only; no reason to use sync at all"). A reducer that does no
/// I/O simply writes an `async fn fold` with no `.await` (the empty-await is accepted); a wasm
/// reducer ([`crate::wasm_host::ComponentReducer`]/[`crate::wasm_host::AsyncComponentReducer`]) awaits its
/// fuel-yielding component call so a long fold cooperatively YIELDS (wasmtime `fuel_async_yield_interval`)
/// instead of blocking the single-threaded host loop.
///
/// **Object-safe via `async-trait`.** The kernel takes `&dyn Reducer` and the host holds
/// `Box<dyn Reducer>`, so the trait MUST stay dyn-compatible — native `async fn` in a trait is not,
/// so this uses `#[async_trait(?Send)]` (`Pin<Box<dyn Future>>` desugaring). `?Send` because the kernel is
/// single-threaded by design (§15b determinism is a sequential per-session fold) and a wasm reducer's fold
/// holds a non-`Send` wasmtime store — requiring `Send` futures would exclude exactly that reducer.
#[async_trait::async_trait(?Send)]
pub trait Reducer {
    /// Fold one event into the KV, returning requested effects. Called once per event, in log order.
    ///
    /// `&mut self` is the NORM (operator ruling 2026-08-08): a host-native reducer — the shape an
    /// [`crate::executor::Executor`] collapses into — holds LIVE, non-replayable Rust capabilities (open
    /// sockets, SDK clients, a ws frame-sink map) directly in its struct fields and mutates them here. The
    /// mutable trait is the default so those impls don't have to wrap every capability in interior mutability.
    ///
    /// The IMMUTABLE, log-based determinism contract is the EXCEPTION, enforced specifically on the
    /// REPLAYABLE reducer path (the wasm-component reducers, [`crate::wasm_host::ComponentReducer`] /
    /// `AsyncComponentReducer`), NOT structurally on the whole trait: a replayable reducer's fold MUST be a
    /// PURE FUNCTION of `(event, kv)` — its ONLY durable state is `kv`, it stashes NO cross-call state in
    /// `self` — so replay (which re-runs `fold` and reads logged `EffectResult`s, never re-performing) yields
    /// live-kv == replayed-kv (§17 / §9d). The wasm reducers satisfy this by simply NOT mutating `self` (a
    /// guest's state IS the kv-backed store); a host-native reducer's live `self` state is fine BECAUSE it
    /// only influences the LOGGED OUTCOME (which replay reads back) and is re-acquired at host restart, never
    /// derived from kv, never expected to survive replay.
    ///
    /// Totality (§17 "can't-brick"): this must not panic for any input — a reducer that sees an event it
    /// doesn't understand returns [`FoldOutput::none`], not a crash. A long-running implementation (a wasm
    /// fold) may `.await` internally so the host loop can interleave other sessions while it yields on fuel.
    async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput;
}

/// A trivial reducer used by kernel-loop tests: it ignores everything and emits nothing. Real reducers
/// (Rust-wasm first, Cadenza-native later) implement domain behavior. Native async (no `.await` — a pure
/// in-process reducer never blocks).
pub struct InertReducer;

#[async_trait::async_trait(?Send)]
impl Reducer for InertReducer {
    async fn fold(&mut self, _event: &Event, _kv: &mut Kv) -> FoldOutput {
        FoldOutput::none()
    }
}

/// Convenience: does this event body carry an effect id the reducer might have a continuation for?
/// (Used by the wasmtime dispatch loop to know when a reducer resume is relevant.) Kept here so the
/// correlation mapping lives beside the contract it serves.
///
/// This is EVERY event that is the TERMINAL OUTCOME of a requested effect — the set the reducer might
/// have stored a continuation for (§16c-S4). Three, not two:
/// - `EffectResult` — the effect ran (Ok/Err/TimedOut); resume with the result.
/// - `TimerFired` — an armed timer's deadline arrived; resume the timer's continuation.
/// - `AuthzDenied` — the effect was DENIED at the gate and never ran; this is still that id's terminal
///   outcome, so a reducer that stored a continuation keyed by it MUST be resumed (with the denial) or
///   its flow strands on a denied effect — the §9d anti-stuck contract requires every requested effect
///   to produce a resuming outcome, and a denial is one (recovery feedback, §9d). Omitting it here was a
///   latent gap: the kernel already folds `AuthzDenied` observably, but the wasm dispatch loop keys its
///   guest-resume on this helper, so a denied effect wouldn't have resumed the guest.
pub fn resumes_effect(body: &EventBody) -> Option<crate::effect::EffectId> {
    match body {
        EventBody::EffectResult { id, .. } => Some(*id),
        EventBody::TimerFired { id, .. } => Some(*id),
        EventBody::AuthzDenied { id, .. } => Some(*id),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect::{EffectId, Payload};
    use crate::event::EffectOutcome;

    // The native `Reducer` (InertReducer) is drivable through `&dyn Reducer` — proving
    // object-safety holds (the whole reason for async-trait), which the kernel + host rely on.
    #[test]
    fn inert_reducer_is_object_safe_as_dyn_reducer() {
        let mut reducer = InertReducer;
        let event = Event {
            seq: 0,
            cause: None,
            body: EventBody::Genesis {
                reducer: crate::hash::Hash::of(b"r"),
                spawn_nonce: crate::hash::Hash::of(b"test-spawn-nonce"),
                parent: None,
            },
        };
        let mut kv = Kv::new();
        let dyn_reducer: &mut dyn Reducer = &mut reducer;
        let out = poll_ready(dyn_reducer.fold(&event, &mut kv));
        assert_eq!(out, FoldOutput::none());
    }

    // Poll an IMMEDIATELY-READY future once with a no-op waker. `InertReducer::fold` completes
    // synchronously — it does no real `.await` — so its future is ready on the first poll and never
    // returns Pending. Fail-fast on Pending rather than spin: this helper is only for known-ready futures,
    // so a Pending here is a bug (an unexpected await), not something to busy-wait on.
    fn poll_ready<F: std::future::Future>(fut: F) -> F::Output {
        use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
        fn noop_raw() -> RawWaker {
            fn no_op(_: *const ()) {}
            fn clone(_: *const ()) -> RawWaker {
                noop_raw()
            }
            RawWaker::new(
                std::ptr::null(),
                &RawWakerVTable::new(clone, no_op, no_op, no_op),
            )
        }
        let waker = unsafe { Waker::from_raw(noop_raw()) };
        let mut cx = Context::from_waker(&waker);
        let mut fut = std::pin::pin!(fut);
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => v,
            Poll::Pending => {
                panic!("poll_ready: future was not immediately ready (unexpected .await)")
            }
        }
    }

    #[test]
    fn resumes_effect_recognizes_every_terminal_outcome() {
        let result = EventBody::EffectResult {
            id: EffectId(7),
            result: EffectOutcome::Ok(None),
            token: None,
        };
        let timer = EventBody::TimerFired {
            id: EffectId(9),
            fired_ms: 1,
            token: None,
        };
        // A DENIAL is the terminal outcome of a requested effect too — the reducer's continuation for
        // that id must resume, or its flow strands on the denied effect (§9d anti-stuck).
        let denied = EventBody::AuthzDenied {
            id: EffectId(11),
            reason: "no capability".into(),
            token: None,
        };
        let closed = EventBody::Closed {
            outcome: crate::event::CloseOutcome::Success(Payload::Inline(vec![].into())),
        };
        assert_eq!(resumes_effect(&result), Some(EffectId(7)));
        assert_eq!(resumes_effect(&timer), Some(EffectId(9)));
        assert_eq!(resumes_effect(&denied), Some(EffectId(11)));
        // Non-outcome events carry no continuation to resume.
        assert_eq!(resumes_effect(&closed), None);
    }
}
