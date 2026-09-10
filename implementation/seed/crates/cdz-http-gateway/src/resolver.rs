//! The gateway effect resolver (`DESIGN-http-outpost-drive-contract.md` §2) — the concrete `carry` the
//! per-session [`drive`](crate::drive::drive) loop hands each emitted request.
//!
//! The gateway is a pure router of effects: it switches on the request's contract-id (the CANONICAL
//! computed ids of `cdz_platform::contracts::*`, injected at construction — never hard-coded markers) and
//! never inspects a payload beyond the envelope a given effect needs.
//!
//! This slice implements the **`http.dispatch`** effect — how a root router hands a matched request to a
//! subprogram: decode the `Dispatch { subprogram, input }` payload, fetch + spawn the subprogram from the
//! store by its `ProgramHash`, RECURSIVELY drive it (so a handler may itself dispatch, bounded by a
//! recursion budget), and fold its terminal `Break` reason back to the emitter as the dispatch answer. Any
//! other contract-id is answered `Err(MissingHandler)` for now; `control.send` (forward up the control
//! link), `ws.send`, and timers (a request with a `deadline`) layer on once their sinks exist.
//!
//! `http.response` / `http.deny` are NOT effects here — a program emits them as its terminal `Break`
//! (schema = that contract-id), which the edge decodes into an HTTP response; the resolver never sees them.

use crate::drive::drive;
use bytes::Bytes;
use cdz_http_protocol::value;
use cdz_platform::{
    ContractId, Delivered, Error, HostId, Message, Origin, ProgramHash, ProgramStore, ReducerId,
    ReducerKind, Request, Response, Runtime, SpawnContext,
};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

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
}

/// A boxed, `Send`, `'static` carry future — boxing makes the dispatch → drive → carry recursion DYNAMIC
/// rather than an infinitely-monomorphized type.
type CarryFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

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
        })
    }

    /// Carry out one emitted [`Request`] fire-and-forget, injecting its answer back into `mailbox` as a
    /// `Delivered::Response` correlated by `continuation_token`. This is the `carry` [`drive`] calls; pass
    /// it as `move |req, tx| resolver.clone().carry::<R>(req, tx)`.
    pub fn carry<R: Runtime>(self: Arc<Self>, request: Request, mailbox: R::Sender) -> CarryFuture {
        Box::pin(async move {
            if request.id == self.dispatch_id && self.depth > 0 {
                let answer = self.run_dispatch::<R>(&request.payload).await;
                let payload = answer.ok_or(Error::MissingHandler);
                reply::<R>(&mailbox, request.id, request.continuation_token, payload);
            } else {
                // Unknown effect (or dispatch budget exhausted): answer a runtime failure so the emitter's
                // `on_response` folds it rather than blocking. control.send / ws.send / timers land here
                // until their sinks exist.
                reply::<R>(
                    &mailbox,
                    request.id,
                    request.continuation_token,
                    Err(Error::MissingHandler),
                );
            }
        })
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
        // The child drives the subprogram's own effects with one less dispatch budget.
        let child = Arc::new(Self {
            store: Arc::clone(&self.store),
            dispatch_id: self.dispatch_id,
            request_id: self.request_id,
            depth: self.depth - 1,
        });
        let (_schema, reason) = drive::<R>(reducer, first, move |req, tx| {
            Arc::clone(&child).carry::<R>(req, tx)
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

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use cdz_platform::{Notification, Outcome, Reducer, TokioRuntime};
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

        let out = drive::<TokioRuntime>(router, opening(), move |req, tx| {
            Arc::clone(&resolver).carry::<TokioRuntime>(req, tx)
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
        let out = drive::<TokioRuntime>(router, opening(), move |req, tx| {
            Arc::clone(&resolver).carry::<TokioRuntime>(req, tx)
        })
        .await;
        // The dispatch answer was Err → the router folded an empty reason (unwrap_or_default) and broke.
        assert_eq!(out, Some((resp_id(), Bytes::new())));
    }
}
