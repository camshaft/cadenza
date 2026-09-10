//! The per-session reducer drive (`DESIGN-http-outpost-drive-contract.md` §1) — the gateway side of the
//! shared mailbox loop.
//!
//! [`drive`] runs one reducer (the control-shipped root router, or a subprogram it dispatches to) to its
//! terminal `Break` over a fresh per-session mailbox, REUSING the platform's
//! [`cdz_platform::run_mailbox_loop`] rather than a divergent copy (operator directive; drive-contract §1).
//! It delivers the opening event, then hands every request the reducer emits to a pluggable effect resolver
//! FIRE-AND-FORGET: the resolver carries the effect out and injects any answer back into the mailbox as a
//! `Delivered::Response` correlated by `continuation_token`, so many effects can be in flight and the loop
//! never blocks awaiting one.
//!
//! The concrete gateway effect vocabulary (`http.dispatch` / `http.response` / `http.deny` / `control.send`
//! / `ws.send` / timer, routed by the canonical computed contract-ids of `cdz_platform::contracts::*`) is
//! layered on top of this in the effect-resolver slice; `drive` itself is effect-agnostic — it just runs the
//! loop and returns the terminal `Break`.

use crate::cancel::CancelScope;
use cdz_platform::{ContractId, Delivered, Reducer, Request, Runtime, run_mailbox_loop};

/// Drive `reducer` to its terminal `Break`: deliver `first` as its opening event, and resolve every request
/// it emits via `carry`. Returns the `Break` reason `(schema, reason)` — the closing value the gateway turns
/// into an HTTP response / deny — or `None` if the reducer closed its mailbox or a fold panicked.
///
/// `carry` is the pluggable effect resolver: for each emitted [`Request`] it receives the request plus a
/// clone of the session mailbox sender, carries the effect out, and (when an answer arrives) injects a
/// `Delivered::Response` back through that sender — fire-and-forget, never awaited by the loop, so the answer
/// folds through the reducer's `on_response` on a later turn correlated by `continuation_token`.
pub async fn drive<R: Runtime>(
    mut reducer: Box<dyn Reducer>,
    first: Delivered,
    mut carry: impl FnMut(Request, R::Sender, &CancelScope),
) -> Option<(ContractId, bytes::Bytes)> {
    let (sender, mut receiver) = R::channel();
    // The opening event: the http-request delivered as an `on_message`, a subprogram's dispatched input, …
    R::send(&sender, first);
    // The cancel scope for THIS session's spawned effect tasks (a dispatched child drive, a deadline timer):
    // `carry` wraps each spawned task through it, and when this drive ends — the reducer `Break`s, or its
    // mailbox closes — the scope drops and ABORTS any still-running task. So an abandoned/timed-out request
    // leaves no orphan handler still driving or firing `control.send` side effects (design §1/§6). Recursive:
    // a cancelled child-drive task drops its own inner scope, cancelling its grandchildren in turn.
    let scope = CancelScope::new();
    // Reuse the platform's mailbox loop; `carry` dispatches each emitted request FIRE-AND-FORGET (it spawns
    // the work under `scope` and returns immediately, injecting any answer back through `sender` later), so
    // the loop never blocks awaiting an effect. Each request gets its own clone of the mailbox sender.
    run_mailbox_loop::<R>(
        &mut reducer,
        &mut receiver,
        async move |request: Request| {
            carry(request, sender.clone(), &scope);
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use bytes::Bytes;
    use cdz_platform::{
        HostId, Message, Notification, Origin, Outcome, ReducerId, Response, TokioRuntime,
    };

    /// A `Delivered::Message` with a synthetic edge origin, as the gateway would deliver an http-request.
    fn opening(id: &[u8], payload: &[u8]) -> Delivered {
        Delivered::Message(Message {
            id: ContractId::of(id),
            payload: Bytes::copy_from_slice(payload),
            from: Origin {
                reducer: ReducerId::of(b"edge"),
                host: HostId::of(b"gateway"),
            },
            continuation_token: Bytes::new(),
        })
    }

    /// A reducer that `Break`s immediately on its opening message, echoing the payload as the reason.
    struct BreakNow;
    #[async_trait]
    impl Reducer for BreakNow {
        async fn on_message(&mut self, m: Message) -> (Vec<Request>, Outcome) {
            (
                Vec::new(),
                Outcome::Break {
                    schema: ContractId::of(b"http-response"),
                    reason: m.payload,
                },
            )
        }
        async fn on_response(&mut self, _: Response) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
        async fn on_notification(&mut self, _: Notification) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
    }

    #[tokio::test]
    async fn drives_a_reducer_to_its_immediate_break() {
        let out = drive::<TokioRuntime>(
            Box::new(BreakNow),
            opening(b"http-request", b"hello"),
            // no effects emitted, so the resolver is never invoked
            |_req, _tx, _scope| {},
        )
        .await;
        assert_eq!(
            out,
            Some((
                ContractId::of(b"http-response"),
                Bytes::from_static(b"hello")
            ))
        );
    }

    /// A reducer that emits ONE effect on its opening message (`Continue`), then `Break`s when that effect's
    /// response folds back — exercising the fire-and-forget effect → response → break path through the loop.
    struct EffectThenBreak;
    #[async_trait]
    impl Reducer for EffectThenBreak {
        async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
            (
                vec![Request {
                    id: ContractId::of(b"an-effect"),
                    payload: Bytes::from_static(b"go"),
                    continuation_token: Bytes::from_static(b"t1"),
                    deadline: None,
                }],
                Outcome::Continue,
            )
        }
        async fn on_response(&mut self, r: Response) -> (Vec<Request>, Outcome) {
            (
                Vec::new(),
                Outcome::Break {
                    schema: ContractId::of(b"http-response"),
                    reason: r.payload.unwrap_or_default(),
                },
            )
        }
        async fn on_notification(&mut self, _: Notification) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
    }

    #[tokio::test]
    async fn resolves_an_effect_then_folds_its_response_to_break() {
        // The resolver injects a canned Ok answer for the emitted effect, correlated by continuation_token —
        // the fire-and-forget shape the real gateway effect resolver uses (dispatch a subprogram, forward
        // control.send, arm a timer, …), here canned to prove the loop wiring.
        let carry = |req: Request, tx: <TokioRuntime as Runtime>::Sender, _scope: &CancelScope| {
            TokioRuntime::send(
                &tx,
                Delivered::Response(Response {
                    id: req.id,
                    continuation_token: req.continuation_token,
                    payload: Ok(Bytes::from_static(b"answered")),
                }),
            );
        };
        let out = drive::<TokioRuntime>(
            Box::new(EffectThenBreak),
            opening(b"http-request", b""),
            carry,
        )
        .await;
        assert_eq!(
            out,
            Some((
                ContractId::of(b"http-response"),
                Bytes::from_static(b"answered")
            ))
        );
    }
}
