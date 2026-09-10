//! The looping-reducer drive loop (`DESIGN-http-outpost-drive-contract.md` §1, redirect inc-3).
//!
//! The operator redirect replaces the one-shot handler (fold one `on_message`, expect a `Break`) with a REAL
//! LOOPING REDUCER: a program runs across turns, emits EFFECTS (the `Vec<Request>` every fold returns), and
//! folds each effect's answer back via `on_response` — until it `Break`s (its terminal result) or goes
//! quiescent. [`drive_loop`] is that driver, generic over any [`Reducer`] and an [`EffectResolver`] that
//! performs an emitted effect and produces the response folded back. The router program and the subprograms
//! it dispatches to are all driven this way; the real gateway's resolver routes an effect by its contract-id
//! (the effect vocabulary, drive-contract §2) — tests supply a stub. Timers are effects with a deadline; the
//! resolver owns that (a later slice), so the loop itself stays a pure fold-and-feed.
//!
//! Socket/CAS-independent: no wasmtime, no HTTP — the drive is the same for a wasmtime reducer and a native
//! one, so it lives outside the `host` feature and is exercised with native reducers.

use async_trait::async_trait;
use cdz_platform::{Bytes, ContractId, Message, Outcome, Reducer, Request, Response};
use std::collections::VecDeque;

/// Performs one effect a reducer emitted and produces the [`Response`] folded back into the reducer. The
/// gateway's real resolver routes by `req.id` (drive-contract §2: dispatch a subprogram, send to the control
/// server, arm a timer, …) and correlates the answer by the request's `continuation_token`; a test supplies
/// a stub. `Send + Sync` so the driver can hold it behind a shared reference across `.await`s.
#[async_trait]
pub trait EffectResolver: Send + Sync {
    /// Resolve `req` into the response to fold back (its `id`/`continuation_token` must echo `req`'s so the
    /// reducer correlates it). A domain failure rides as `Response.payload = Err(..)`, still an answer.
    async fn resolve(&self, req: Request) -> Response;
}

/// Why a drive loop ended, other than a clean `Break`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveEnd {
    /// The reducer went quiescent — a `Continue` with no pending effects left to resolve, so there is nothing
    /// more to do for this input and it never produced a terminal `Break`.
    Quiescent,
    /// The loop hit its fold ceiling without settling (a runaway that keeps emitting effects) — bounded so a
    /// misbehaving program cannot spin the driver forever.
    FoldLimit,
}

/// Drive `reducer` through its loop starting from `first`: fold it, then repeatedly resolve each emitted
/// effect (via `resolver`) and fold the response back, until the reducer `Break`s or the queue drains.
///
/// Returns `Ok((schema, reason))` — the reducer's terminal `Break` value (e.g. an `http-response`) — or
/// `Err(DriveEnd)` if it went quiescent or hit `max_folds`. Effects are resolved FIFO; each `on_response`
/// may emit further effects (a subprogram's answer prompting the next), which are appended. Effects emitted
/// ALONGSIDE the terminal `Break` are NOT resolved here (their responses never fold, per `Outcome`'s
/// contract) — dispatching those fire-and-forget is the caller's/edge's job (a later slice with the effect
/// vocabulary). `max_folds` bounds total folds (the initial `on_message` counts as one).
pub async fn drive_loop(
    reducer: &mut dyn Reducer,
    first: Message,
    resolver: &dyn EffectResolver,
    max_folds: usize,
) -> Result<(ContractId, Bytes), DriveEnd> {
    let (requests, outcome) = reducer.on_message(first).await;
    if let Outcome::Break { schema, reason } = outcome {
        return Ok((schema, reason));
    }
    let mut queue: VecDeque<Request> = requests.into();
    let mut folds = 1usize;
    while let Some(req) = queue.pop_front() {
        if folds >= max_folds {
            return Err(DriveEnd::FoldLimit);
        }
        let response = resolver.resolve(req).await;
        let (more, outcome) = reducer.on_response(response).await;
        folds += 1;
        if let Outcome::Break { schema, reason } = outcome {
            return Ok((schema, reason));
        }
        queue.extend(more);
    }
    Err(DriveEnd::Quiescent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use cdz_platform::{HostId, Notification, Origin, ReducerId};

    fn msg(payload: &'static [u8]) -> Message {
        Message {
            id: ContractId::of(b"cdz.http.request"),
            payload: Bytes::from_static(payload),
            from: Origin {
                reducer: ReducerId::of(b"edge"),
                host: HostId::of(b"host"),
            },
            continuation_token: Bytes::new(),
        }
    }

    /// A resolver that answers every effect `Ok` with a fixed payload, echoing the request's correlation.
    struct FixedResolver(Bytes);
    #[async_trait]
    impl EffectResolver for FixedResolver {
        async fn resolve(&self, req: Request) -> Response {
            Response {
                id: req.id,
                continuation_token: req.continuation_token,
                payload: Ok(self.0.clone()),
            }
        }
    }

    fn a_request(token: &'static [u8]) -> Request {
        Request {
            id: ContractId::of(b"cdz.effect"),
            payload: Bytes::new(),
            continuation_token: Bytes::from_static(token),
            deadline: None,
        }
    }
    fn brk(reason: &'static [u8]) -> Outcome {
        Outcome::Break {
            schema: ContractId::of(b"cdz.http.response"),
            reason: Bytes::from_static(reason),
        }
    }

    /// A reducer that Breaks immediately on its first message — the degenerate (one-shot) case still drives.
    struct BreaksNow;
    #[async_trait]
    impl Reducer for BreaksNow {
        async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
            (vec![], brk(b"immediate"))
        }
        async fn on_response(&mut self, _r: Response) -> (Vec<Request>, Outcome) {
            (vec![], Outcome::Continue)
        }
        async fn on_notification(&mut self, _n: Notification) -> (Vec<Request>, Outcome) {
            (vec![], Outcome::Continue)
        }
    }

    /// A reducer that emits ONE effect and stays open, then Breaks when its response folds back — the core
    /// emit-effect → get-answer → respond loop.
    struct OneEffectThenBreak;
    #[async_trait]
    impl Reducer for OneEffectThenBreak {
        async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
            (vec![a_request(b"t1")], Outcome::Continue)
        }
        async fn on_response(&mut self, r: Response) -> (Vec<Request>, Outcome) {
            // Close with the effect's answer as the terminal reason.
            let answer = r.payload.unwrap_or_default();
            (
                vec![],
                Outcome::Break {
                    schema: ContractId::of(b"cdz.http.response"),
                    reason: answer,
                },
            )
        }
        async fn on_notification(&mut self, _n: Notification) -> (Vec<Request>, Outcome) {
            (vec![], Outcome::Continue)
        }
    }

    /// A reducer that never closes and emits no effects — quiescent on the first fold.
    struct Quiescent;
    #[async_trait]
    impl Reducer for Quiescent {
        async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
            (vec![], Outcome::Continue)
        }
        async fn on_response(&mut self, _r: Response) -> (Vec<Request>, Outcome) {
            (vec![], Outcome::Continue)
        }
        async fn on_notification(&mut self, _n: Notification) -> (Vec<Request>, Outcome) {
            (vec![], Outcome::Continue)
        }
    }

    /// A reducer that keeps emitting an effect every fold and never Breaks — a runaway.
    struct Runaway;
    #[async_trait]
    impl Reducer for Runaway {
        async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
            (vec![a_request(b"loop")], Outcome::Continue)
        }
        async fn on_response(&mut self, _r: Response) -> (Vec<Request>, Outcome) {
            (vec![a_request(b"loop")], Outcome::Continue)
        }
        async fn on_notification(&mut self, _n: Notification) -> (Vec<Request>, Outcome) {
            (vec![], Outcome::Continue)
        }
    }

    #[tokio::test]
    async fn an_immediate_break_returns_its_reason() {
        let mut r = BreaksNow;
        let out = drive_loop(&mut r, msg(b"x"), &FixedResolver(Bytes::new()), 100).await;
        assert_eq!(
            out.map(|(_, reason)| reason),
            Ok(Bytes::from_static(b"immediate"))
        );
    }

    #[tokio::test]
    async fn drives_an_effect_then_a_response_then_break() {
        let mut r = OneEffectThenBreak;
        let out = drive_loop(
            &mut r,
            msg(b"x"),
            &FixedResolver(Bytes::from_static(b"the-answer")),
            100,
        )
        .await;
        // The emitted effect was resolved to "the-answer", folded back, and the reducer Broke with it.
        assert_eq!(
            out.map(|(_, reason)| reason),
            Ok(Bytes::from_static(b"the-answer")),
            "the effect's response folds back and becomes the terminal reason"
        );
    }

    #[tokio::test]
    async fn a_quiescent_reducer_ends_without_a_break() {
        let mut r = Quiescent;
        let out = drive_loop(&mut r, msg(b"x"), &FixedResolver(Bytes::new()), 100).await;
        assert_eq!(out, Err(DriveEnd::Quiescent));
    }

    #[tokio::test]
    async fn a_runaway_is_bounded_by_max_folds() {
        let mut r = Runaway;
        let out = drive_loop(&mut r, msg(b"x"), &FixedResolver(Bytes::new()), 8).await;
        assert_eq!(
            out,
            Err(DriveEnd::FoldLimit),
            "a never-Breaking loop is bounded"
        );
    }

    /// Multiple effects emitted in one fold are each resolved (FIFO); the reducer counts them and Breaks on
    /// the last — proving the queue drives every emitted effect.
    #[tokio::test]
    async fn resolves_every_emitted_effect() {
        struct TwoThenBreak {
            seen: usize,
        }
        #[async_trait]
        impl Reducer for TwoThenBreak {
            async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
                (vec![a_request(b"a"), a_request(b"b")], Outcome::Continue)
            }
            async fn on_response(&mut self, _r: Response) -> (Vec<Request>, Outcome) {
                self.seen += 1;
                if self.seen == 2 {
                    (
                        vec![],
                        Outcome::Break {
                            schema: ContractId::of(b"cdz.http.response"),
                            reason: Bytes::from_static(b"both-seen"),
                        },
                    )
                } else {
                    (vec![], Outcome::Continue)
                }
            }
            async fn on_notification(&mut self, _n: Notification) -> (Vec<Request>, Outcome) {
                (vec![], Outcome::Continue)
            }
        }
        let mut r = TwoThenBreak { seen: 0 };
        let out = drive_loop(&mut r, msg(b"x"), &FixedResolver(Bytes::new()), 100).await;
        assert_eq!(
            out.map(|(_, reason)| reason),
            Ok(Bytes::from_static(b"both-seen"))
        );
        assert_eq!(r.seen, 2, "both emitted effects were resolved + folded");
    }
}
