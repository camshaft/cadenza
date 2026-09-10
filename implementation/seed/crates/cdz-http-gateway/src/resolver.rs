//! The gateway effect resolver (`DESIGN-http-outpost-drive-contract.md` §2) — the concrete `carry` the
//! per-session [`drive`](crate::drive::drive) loop hands each emitted request.
//!
//! The gateway is a pure router of effects: it switches on the request's contract-id (the CANONICAL
//! computed ids of `cdz_platform::contracts::*`, injected at construction — never hard-coded markers) and
//! never inspects a payload beyond the envelope a given effect needs.
//!
//! Effects handled: **`http.dispatch`** — decode `Dispatch { subprogram, input }`, fetch + spawn the
//! subprogram from the store by its `ProgramHash`, RECURSIVELY drive it (a handler may itself dispatch,
//! bounded by a recursion budget), and fold its terminal `Break` reason back as the dispatch answer.
//! **`control.send`** (§3) — forward the opaque payload UP the control link and register the emitter so the
//! `ControlDown` response folds back. A **per-request `deadline`** — arm a `Timeout` if no answer arrives in
//! time. Any other contract-id is answered `Err(MissingHandler)`; `ws.send` layers on once its sink exists.
//!
//! `http.response` / `http.deny` are NOT effects here — a program emits them as its terminal `Break`
//! (schema = that contract-id), which the edge decodes into an HTTP response; the resolver never sees them.

use crate::cancel::CancelScope;
use crate::drive::drive;
use crate::session::{ControlSink, Sessions};
use bytes::Bytes;
use cdz_http_protocol::{ControlUp, RequestContext, value};
use cdz_platform::{
    ContractId, Delivered, Error, HostId, Message, Origin, ProgramHash, ProgramStore, ReducerId,
    ReducerKind, Request, Response, Runtime, SpawnContext,
};
use std::sync::Arc;
use std::time::Duration;

/// The control-link back-channel a `control.send` effect uses (design §3), threaded through the drive tree.
/// Cloned per dispatched subprogram with only `program` updated (the emitting handler's hash), so a
/// `ControlUp` is stamped with the correct provenance at every depth. Cheap to clone (an mpsc sender, two
/// `Arc`s, small fields).
#[derive(Clone)]
pub struct ControlCtx {
    /// The write half of the control link — a `control.send` is forwarded UP through here as a `ControlUp`.
    pub sink: ControlSink,
    /// The pending-`control.send` registry — the emitting reducer is registered here so the control server's
    /// `ControlDown` response folds back into it (correlated by `continuation_token`).
    pub sessions: Sessions,
    /// The `control.send` effect's canonical contract-id — a request on this id is forwarded, not dispatched.
    pub control_send: ContractId,
    /// This request's session id, stamped on each `ControlUp` so the control server can address responses.
    pub session: Bytes,
    /// The originating HTTP request context, so the control server can route without re-parsing the payload.
    pub request: Arc<RequestContext>,
    /// The `ProgramHash` of the reducer currently being driven — the `ControlUp`'s `program` provenance.
    pub program: ProgramHash,
}

/// Routes the effects a driven program emits to their gateway actions, by canonical computed contract-id.
/// Held behind an [`Arc`] so a dispatched subprogram is driven with a cheap child clone (recursion) and the
/// carry futures stay `'static` + `Send`.
pub struct GatewayResolver {
    /// The content-addressed store the gateway fetches subprograms from (the HTTP CAS in production, a
    /// native store in tests).
    store: Arc<dyn ProgramStore>,
    /// The `http.dispatch` effect's contract-id — `cdz_platform::contracts::http_dispatch::contract().id()`
    /// in production; a test id in unit tests. A request on this id is a dispatch.
    dispatch_id: ContractId,
    /// The contract-id the dispatched subprogram's opening `Message` carries (the `http-request` id in
    /// production), so the handler folds its input through `on_message` typed as the request it handles.
    request_id: ContractId,
    /// Remaining dispatch-recursion budget: a chain router → handler → sub-handler → … is bounded so a
    /// cyclic or runaway dispatch cannot recurse forever. `0` ⇒ no further dispatch (answered as missing).
    depth: usize,
    /// The control-link back-channel for `control.send` (design §3), or `None` if the gateway drives without
    /// a control link (unit tests, or a `control.send` with nowhere to go answers `MissingHandler`).
    control: Option<ControlCtx>,
}

impl GatewayResolver {
    /// A resolver over `store`, routing dispatch on `dispatch_id`, delivering a subprogram's input under
    /// `request_id`, with a `depth`-deep dispatch-recursion budget.
    #[must_use]
    pub fn new(
        store: Arc<dyn ProgramStore>,
        dispatch_id: ContractId,
        request_id: ContractId,
        depth: usize,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            dispatch_id,
            request_id,
            depth,
            control: None,
        })
    }

    /// A resolver as [`new`](Self::new) plus the control-link back-channel (design §3), so a `control.send`
    /// effect is forwarded UP and its response folds back rather than answered `MissingHandler`.
    #[must_use]
    pub fn new_with_control(
        store: Arc<dyn ProgramStore>,
        dispatch_id: ContractId,
        request_id: ContractId,
        depth: usize,
        control: ControlCtx,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            dispatch_id,
            request_id,
            depth,
            control: Some(control),
        })
    }

    /// Carry out one emitted [`Request`] FIRE-AND-FORGET, then return immediately — never blocking [`drive`]'s
    /// loop (§1). Any answer is injected back into `mailbox` as a `Delivered::Response` correlated by
    /// `continuation_token`, on a later turn. Spawned work (a dispatched child drive, a deadline timer) is
    /// wrapped through `scope`, so [`drive`] aborts it if this session ends before it settles. Pass it as
    /// `move |req, tx, scope| resolver.clone().carry::<R>(req, tx, scope)`.
    pub fn carry<R: Runtime>(
        self: Arc<Self>,
        request: Request,
        mailbox: R::Sender,
        scope: &CancelScope,
    ) {
        // Arm the per-request deadline (§1/§2): if no answer arrives within `d`, inject `Err(Timeout)`
        // correlated by `continuation_token`. Whichever of the answer or the timeout folds first resolves the
        // token; a late loser is a no-op (the reducer already resolved it, or the mailbox closed on `Break`).
        if let Some(deadline) = request.deadline {
            arm_deadline::<R>(
                scope,
                deadline,
                request.id,
                request.continuation_token.clone(),
                &mailbox,
            );
        }
        if request.id == self.dispatch_id && self.depth > 0 {
            // http.dispatch (§1): SPAWN the child drive (wrapped through `scope`) and inject its terminal
            // `Break` reason back into the emitter's mailbox as the dispatch answer — fire-and-forget, so the
            // parent loop keeps running (other effects in flight, the deadline able to fire). If this session
            // ends first, `scope` aborts the in-flight child drive, so no orphan handler keeps running.
            let this = Arc::clone(&self);
            let reply_to = mailbox.clone();
            let payload = request.payload.clone();
            let id = request.id;
            let token = request.continuation_token.clone();
            R::spawn(scope.wrap(async move {
                let answer = this.run_dispatch::<R>(&payload).await;
                reply::<R>(&reply_to, id, token, answer.ok_or(Error::MissingHandler));
            }));
        } else if let Some(ctx) = self
            .control
            .as_ref()
            .filter(|c| request.id == c.control_send)
        {
            // control.send (§3): forward the OPAQUE payload UP the control link as a `ControlUp`, and register
            // the emitting reducer's mailbox so the control server's `ControlDown` response folds back as its
            // `on_response` (correlated by `continuation_token`). No reply here; the answer arrives later via
            // the session registry. If the sink is closed (link down) the send is dropped and the reducer
            // relies on its deadline (if any) rather than hanging.
            self.forward_control_send::<R>(ctx, &request, &mailbox);
        } else {
            // Unknown effect (or dispatch budget exhausted): answer a runtime failure so the emitter's
            // `on_response` folds it rather than blocking. `ws.send` lands here until its sink exists; a
            // `control.send` with no control link also lands here.
            reply::<R>(
                &mailbox,
                request.id,
                request.continuation_token,
                Err(Error::MissingHandler),
            );
        }
    }

    /// Forward a `control.send` UP the control link (§3): register the emitting reducer's mailbox under the
    /// effect's `continuation_token` (so the control server's `ControlDown` response folds back as its
    /// `on_response`), then send the opaque payload UP as a [`ControlUp`] stamped with the emitting program's
    /// hash, this request's session id, and the originating HTTP request context. Fire-and-forget: the reducer
    /// keeps looping (`Continue`) and hears back only when the response arrives — never blocked here.
    fn forward_control_send<R: Runtime>(
        &self,
        ctx: &ControlCtx,
        request: &Request,
        mailbox: &R::Sender,
    ) {
        let correlation = request.continuation_token.clone();
        let mailbox = mailbox.clone();
        ctx.sessions
            .register(correlation.clone(), ctx.control_send, move |event| {
                R::send(&mailbox, event);
            });
        let _ = ctx.sink.send(ControlUp {
            program: Bytes::copy_from_slice(ctx.program.hash().as_bytes()),
            session: ctx.session.clone(),
            correlation,
            payload: request.payload.clone(),
            request: (*ctx.request).clone(),
        });
    }

    /// Decode a `Dispatch { subprogram, input }` effect, spawn the subprogram from the store, drive it (with
    /// a depth-decremented child resolver so it may dispatch in turn), and return its terminal `Break`
    /// reason — the value that folds back as the dispatch answer. `None` on any failure (undecodable
    /// effect, bad hash, unspawnable program, no terminal break).
    async fn run_dispatch<R: Runtime>(self: &Arc<Self>, effect: &[u8]) -> Option<Bytes> {
        let arenas = value::decode(effect)?;
        let rec = value::unascribe(&arenas, arenas.root);
        let subprogram =
            value::read_bytes(&arenas, value::record_field(&arenas, rec, "subprogram")?)?;
        let input = value::read_bytes(&arenas, value::record_field(&arenas, rec, "input")?)?;

        let hash = ProgramHash::try_from(subprogram.as_ref()).ok()?;
        let reducer = self
            .store
            .spawn(
                hash,
                SpawnContext {
                    id: ReducerId::of(&subprogram),
                    kind: ReducerKind::Ordinary,
                    limits: None,
                },
            )
            .await?;

        // Deliver the input as the subprogram's opening message, typed as the request it handles.
        let first = Delivered::Message(Message {
            id: self.request_id,
            payload: input,
            from: Origin {
                reducer: ReducerId::of(b"cdz-http-gateway.dispatch"),
                host: HostId::of(b"cdz-http-gateway"),
            },
            continuation_token: Bytes::new(),
        });
        // The child drives the subprogram's own effects with one less dispatch budget, and inherits the
        // control back-channel with `program` updated to the subprogram's hash (so its `control.send`s are
        // stamped with the right provenance).
        let child = Arc::new(Self {
            store: Arc::clone(&self.store),
            dispatch_id: self.dispatch_id,
            request_id: self.request_id,
            depth: self.depth - 1,
            control: self.control.clone().map(|mut c| {
                c.program = hash;
                c
            }),
        });
        let (_schema, reason) = drive::<R>(reducer, first, move |req, tx, scope| {
            Arc::clone(&child).carry::<R>(req, tx, scope)
        })
        .await?;
        Some(reason)
    }
}

/// Inject a `Response` (the answer to an emitted effect) back into the driven program's mailbox.
fn reply<R: Runtime>(
    mailbox: &R::Sender,
    id: ContractId,
    continuation_token: Bytes,
    payload: Result<Bytes, Error>,
) {
    R::send(
        mailbox,
        Delivered::Response(Response {
            id,
            continuation_token,
            payload,
        }),
    );
}

/// Arm a per-request deadline: after `deadline`, inject `Err(Timeout)` for `id`/`continuation_token` into the
/// emitting reducer's mailbox (§1/§2). Spawned under `scope` so it is aborted if the session ends first; a
/// fire after the reducer has already resolved the token (or closed its mailbox on `Break`) is a harmless
/// no-op regardless.
fn arm_deadline<R: Runtime>(
    scope: &CancelScope,
    deadline: Duration,
    id: ContractId,
    continuation_token: Bytes,
    mailbox: &R::Sender,
) {
    let mailbox = mailbox.clone();
    R::spawn(scope.wrap(async move {
        R::sleep(deadline).await;
        reply::<R>(&mailbox, id, continuation_token, Err(Error::Timeout));
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use cdz_platform::{Notification, Outcome, Reducer, Str, TokioRuntime};
    use std::collections::HashMap;

    // --- a native ProgramStore: spawn a reducer by ProgramHash from a factory table -----------------------

    type Factory = Box<dyn Fn() -> Box<dyn Reducer> + Send + Sync>;
    struct NativeStore {
        programs: HashMap<Vec<u8>, Factory>,
    }
    impl NativeStore {
        fn with(programs: Vec<(ProgramHash, Factory)>) -> Arc<Self> {
            Arc::new(Self {
                programs: programs
                    .into_iter()
                    .map(|(h, f)| (h.hash().digest().to_vec(), f))
                    .collect(),
            })
        }
    }
    #[async_trait]
    impl ProgramStore for NativeStore {
        async fn spawn(
            &self,
            program: ProgramHash,
            _ctx: SpawnContext,
        ) -> Option<Box<dyn Reducer>> {
            self.programs
                .get(program.hash().digest().as_slice())
                .map(|f| f())
        }
        async fn contains(&self, program: ProgramHash) -> bool {
            self.programs
                .contains_key(program.hash().digest().as_slice())
        }
    }

    // --- test reducers -----------------------------------------------------------------------------------

    fn dispatch_id() -> ContractId {
        ContractId::of(b"cdz-platform.http.dispatch")
    }
    fn request_id() -> ContractId {
        ContractId::of(b"cdz-platform.http.request")
    }
    fn resp_id() -> ContractId {
        ContractId::of(b"cdz-platform.http.response")
    }

    /// Encode a `Dispatch { input, subprogram }` effect payload (name-sorted record, root-ascribed) the way
    /// a guest's `Value.encode` would — reusing the shared value toolkit.
    fn encode_dispatch(subprogram: &ProgramHash, input: &[u8]) -> Bytes {
        use cdz_http_protocol::value::{ValueBuilder, bytes_leaf, finish, record};
        let mut b = ValueBuilder::new();
        let input_leaf = bytes_leaf(&mut b, input);
        let sub_leaf = bytes_leaf(&mut b, subprogram.hash().as_bytes());
        let r = record(
            &mut b,
            vec![("input", input_leaf), ("subprogram", sub_leaf)],
        );
        finish(b, r, "Dispatch")
    }

    /// A leaf handler: `Break`s on its opening message with a fixed http-response reason.
    struct Handler(&'static [u8]);
    #[async_trait]
    impl Reducer for Handler {
        async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
            (
                Vec::new(),
                Outcome::Break {
                    schema: resp_id(),
                    reason: Bytes::from_static(self.0),
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

    /// A router: on its opening message, emit ONE dispatch effect naming `target` with the request as input;
    /// when the dispatch answer folds back, `Break` with it (the handler's response).
    struct Router {
        target: ProgramHash,
    }
    #[async_trait]
    impl Reducer for Router {
        async fn on_message(&mut self, m: Message) -> (Vec<Request>, Outcome) {
            (
                vec![Request {
                    id: dispatch_id(),
                    payload: encode_dispatch(&self.target, &m.payload),
                    continuation_token: Bytes::from_static(b"d1"),
                    deadline: None,
                }],
                Outcome::Continue,
            )
        }
        async fn on_response(&mut self, r: Response) -> (Vec<Request>, Outcome) {
            (
                Vec::new(),
                Outcome::Break {
                    schema: resp_id(),
                    reason: r.payload.unwrap_or_default(),
                },
            )
        }
        async fn on_notification(&mut self, _: Notification) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
    }

    fn opening() -> Delivered {
        Delivered::Message(Message {
            id: request_id(),
            payload: Bytes::from_static(b"GET /"),
            from: Origin {
                reducer: ReducerId::of(b"edge"),
                host: HostId::of(b"gateway"),
            },
            continuation_token: Bytes::new(),
        })
    }

    #[tokio::test]
    async fn dispatch_spawns_drives_and_folds_the_handler_response() {
        let handler_bytes = b"the-handler-program".to_vec();
        let handler_hash = ProgramHash::of(&handler_bytes);
        let store = NativeStore::with(vec![(
            handler_hash,
            Box::new(|| Box::new(Handler(b"200 hello")) as Box<dyn Reducer>),
        )]);
        let resolver = GatewayResolver::new(store.clone(), dispatch_id(), request_id(), 8);
        let router: Box<dyn Reducer> = Box::new(Router {
            target: handler_hash,
        });

        let out = drive::<TokioRuntime>(router, opening(), move |req, tx, scope| {
            Arc::clone(&resolver).carry::<TokioRuntime>(req, tx, scope)
        })
        .await;
        assert_eq!(out, Some((resp_id(), Bytes::from_static(b"200 hello"))));
    }

    #[tokio::test]
    async fn a_dispatch_to_a_missing_subprogram_folds_a_failure() {
        // The router dispatches to a hash the store does not have → run_dispatch None → Err(MissingHandler)
        // answer → the router's on_response folds an empty reason and breaks (proving no hang).
        let store = NativeStore::with(Vec::new());
        let resolver = GatewayResolver::new(store, dispatch_id(), request_id(), 8);
        let router: Box<dyn Reducer> = Box::new(Router {
            target: ProgramHash::of(b"absent"),
        });
        let out = drive::<TokioRuntime>(router, opening(), move |req, tx, scope| {
            Arc::clone(&resolver).carry::<TokioRuntime>(req, tx, scope)
        })
        .await;
        // The dispatch answer was Err → the router folded an empty reason (unwrap_or_default) and broke.
        assert_eq!(out, Some((resp_id(), Bytes::new())));
    }

    fn control_send_id() -> ContractId {
        ContractId::of(b"cdz-platform.control.send")
    }

    /// A reducer that emits ONE `control.send` with a short deadline, then breaks on whatever its answer is —
    /// echoing whether the answer was the deadline `Timeout` (the round-trip never completed) or a real reply.
    struct ControlSendThenBreak;
    #[async_trait]
    impl Reducer for ControlSendThenBreak {
        async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
            (
                vec![Request {
                    id: control_send_id(),
                    payload: Bytes::from_static(b"ping"),
                    continuation_token: Bytes::from_static(b"c1"),
                    deadline: Some(Duration::from_millis(20)),
                }],
                Outcome::Continue,
            )
        }
        async fn on_response(&mut self, r: Response) -> (Vec<Request>, Outcome) {
            let reason = if matches!(r.payload, Err(Error::Timeout)) {
                Bytes::from_static(b"timed-out")
            } else {
                Bytes::from_static(b"unexpected")
            };
            (
                Vec::new(),
                Outcome::Break {
                    schema: resp_id(),
                    reason,
                },
            )
        }
        async fn on_notification(&mut self, _: Notification) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
    }

    /// A subprogram that never terminates — loops on `Continue`, emits nothing, so its drive blocks on `recv`
    /// forever. Used to prove a dispatch to a hung handler is now time-outable.
    struct HangForever;
    #[async_trait]
    impl Reducer for HangForever {
        async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
        async fn on_response(&mut self, _: Response) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
        async fn on_notification(&mut self, _: Notification) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
    }

    /// A router that dispatches to `target` WITH a short deadline, then breaks on the answer — echoing whether
    /// the dispatch timed out or really answered.
    struct RouterWithDeadline {
        target: ProgramHash,
    }
    #[async_trait]
    impl Reducer for RouterWithDeadline {
        async fn on_message(&mut self, m: Message) -> (Vec<Request>, Outcome) {
            (
                vec![Request {
                    id: dispatch_id(),
                    payload: encode_dispatch(&self.target, &m.payload),
                    continuation_token: Bytes::from_static(b"d1"),
                    deadline: Some(Duration::from_millis(20)),
                }],
                Outcome::Continue,
            )
        }
        async fn on_response(&mut self, r: Response) -> (Vec<Request>, Outcome) {
            let reason = if matches!(r.payload, Err(Error::Timeout)) {
                Bytes::from_static(b"timed-out")
            } else {
                Bytes::from_static(b"answered")
            };
            (
                Vec::new(),
                Outcome::Break {
                    schema: resp_id(),
                    reason,
                },
            )
        }
        async fn on_notification(&mut self, _: Notification) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
    }

    #[tokio::test]
    async fn a_dispatch_to_a_hung_handler_is_timed_out() {
        // Dispatch is now FIRE-AND-FORGET: the child drive runs as a spawned task, so a hung handler does NOT
        // block the router's loop — its per-request deadline fires Err(Timeout) after 20ms and the router
        // breaks. (With the old serial dispatch this would hang forever, the loop blocked awaiting the child.)
        // When the router breaks, the drive's scope drops and aborts the still-running child drive.
        let hang = ProgramHash::of(b"hang-forever");
        let store = NativeStore::with(vec![(
            hang,
            Box::new(|| Box::new(HangForever) as Box<dyn Reducer>),
        )]);
        let resolver = GatewayResolver::new(store, dispatch_id(), request_id(), 8);
        let router: Box<dyn Reducer> = Box::new(RouterWithDeadline { target: hang });
        let out = drive::<TokioRuntime>(router, opening(), move |req, tx, scope| {
            Arc::clone(&resolver).carry::<TokioRuntime>(req, tx, scope)
        })
        .await;
        assert_eq!(out, Some((resp_id(), Bytes::from_static(b"timed-out"))));
    }

    #[tokio::test]
    async fn a_control_send_with_a_deadline_and_no_response_folds_a_timeout() {
        // A control.send is fire-and-forget: forwarded UP, then the reducer waits for a ControlDown. With no
        // control server to answer, the per-request deadline is what unwedges it — after 20ms the gateway
        // injects Err(Timeout), which the reducer folds and breaks. Proves the deadline protects a
        // fire-and-forget effect whose response never comes.
        let store = NativeStore::with(Vec::new());
        let (up_tx, _up_rx) = tokio::sync::mpsc::unbounded_channel::<ControlUp>();
        let control = ControlCtx {
            sink: up_tx,
            sessions: Sessions::new(),
            control_send: control_send_id(),
            session: Bytes::from_static(b"sess-test"),
            request: Arc::new(RequestContext {
                method: Str::from("GET"),
                path: Str::from("/"),
                headers: Vec::new(),
            }),
            program: ProgramHash::of(b"test-program"),
        };
        let resolver =
            GatewayResolver::new_with_control(store, dispatch_id(), request_id(), 8, control);
        let reducer: Box<dyn Reducer> = Box::new(ControlSendThenBreak);
        let out = drive::<TokioRuntime>(reducer, opening(), move |req, tx, scope| {
            Arc::clone(&resolver).carry::<TokioRuntime>(req, tx, scope)
        })
        .await;
        assert_eq!(out, Some((resp_id(), Bytes::from_static(b"timed-out"))));
    }
}
