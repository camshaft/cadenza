//! The reference system reducer — the default that shepherds an effect (`design/cadenza-platform.md` §4).
//!
//! When a reducer emits an effect, the system routes it to the system reducer the event registry names for
//! that contract (§4), delivering the effect to it as a message. That system reducer is an ordinary
//! [`Reducer`] with one added power — the privileged API: it reads the routing substrate (the reducer graph)
//! and it delivers events into other reducers' logs. This is the reference implementation the platform
//! bootstraps as the default; a deployment may override it per contract in the event registry (§3), and it
//! will eventually be a wasm module like any other. Native and in-crate for now, so it can hold the
//! [`ReducerGraph`](crate::ReducerGraph) directly rather than reach it across a wasm boundary.
//!
//! What this first reference does: it learns its own id from its birth notification, and on the effect it
//! shepherds it reads the emitter's handler chain for the contract from the graph and delivers the effect to
//! the first handler; if the contract has no handler, it answers the emitter `Err(MissingHandler)` (§4). The
//! rest of dispatch — advancing the chain across generations, pending obligations keyed by the handler's
//! `from`, and forward/respond — is the same reducer's next slice; the reply a handler sends is folded by
//! [`on_response`](ReferenceSystemReducer::on_response), a no-op until then.

use crate::{
    Bytes, Deliver, Delivered, Error, Message, Notification, Origin, Outcome, Reducer,
    ReducerGraph, ReducerId, Request, Response, Spawned, spawned_contract,
};
use async_trait::async_trait;
use std::sync::Arc;

/// The reference [system reducer](self): resolves a contract's handler chain from the graph and delivers the
/// effect to the first handler, or answers `MissingHandler` when there is none. Holds the graph directly (the
/// privileged read) since it is the native in-crate reference; a wasm system reducer would reach the graph
/// through the host's privileged API instead.
pub struct ReferenceSystemReducer {
    /// The routing substrate — read to assemble the handler chain for a contract (§3/§4).
    graph: Arc<dyn ReducerGraph>,
    /// This reducer's own id, learned from its birth notification. It is the **context** id (§4): the id a
    /// handler replies to, so the system reducer stamps it as the `from` on the effect it delivers onward.
    me: Option<ReducerId>,
}

impl ReferenceSystemReducer {
    /// A reference system reducer reading the routing substrate `graph`. Its own id is not known yet; it is
    /// learned from the birth notification the system delivers as its first event (§7).
    #[must_use]
    pub fn new(graph: Arc<dyn ReducerGraph>) -> Self {
        Self { graph, me: None }
    }
}

#[async_trait]
impl Reducer for ReferenceSystemReducer {
    /// Shepherd the effect: resolve the emitter's handler chain for the contract and deliver the effect to
    /// the first handler, or — with no handler — answer the emitter `Err(MissingHandler)` (§4).
    async fn on_message(&mut self, effect: Message) -> (Vec<Request>, Outcome) {
        let emitter = effect.from.reducer;
        let contract = effect.id;
        let chain = self.graph.resolve(emitter, contract).await;
        match chain.first() {
            // A handler answers the contract: deliver the effect to the first one. Its reply comes back to
            // this context — the system reducer stamps its own id as `from`, so the handler replies here, not
            // to the leaf, and never sees the leaf's own correlation token. Advancing past the first handler
            // and folding the reply are the next slice, so this stays alive (`Continue`).
            Some(&handler) => {
                let event = Delivered::Message(Message {
                    id: contract,
                    payload: effect.payload,
                    from: Origin {
                        reducer: self
                            .me
                            .expect("birth notification precedes the first effect"),
                        host: effect.from.host,
                    },
                    // A context-chosen token, not the leaf's (§4). Correlating the reply is the next slice;
                    // until then the token the handler echoes is unused.
                    continuation_token: Bytes::new(),
                });
                (
                    vec![
                        Deliver {
                            target: handler,
                            event,
                        }
                        .into_request(),
                    ],
                    Outcome::Continue,
                )
            }
            // No handler for the contract: answer the emitter `Err(MissingHandler)`, correlated by the
            // emitter's own token, and the dispatch is complete.
            None => {
                let event = Delivered::Response(Response {
                    id: contract,
                    continuation_token: effect.continuation_token,
                    payload: Err(Error::MissingHandler),
                });
                (
                    vec![
                        Deliver {
                            target: emitter,
                            event,
                        }
                        .into_request(),
                    ],
                    Outcome::Break {
                        schema: contract,
                        reason: Bytes::from_static(b"no handler"),
                    },
                )
            }
        }
    }

    /// A handler's reply to a forwarded effect. Folding it back to the caller is the next slice (pending
    /// obligations + forward/respond); for now the reference reducer records nothing and does nothing.
    async fn on_response(&mut self, _response: Response) -> (Vec<Request>, Outcome) {
        (Vec::new(), Outcome::Continue)
    }

    /// Control-plane events. The one this reference acts on is its own birth (§7): it records its id, which
    /// is the context id it stamps onto the effects it delivers. Every other notification is ignored.
    async fn on_notification(&mut self, notification: Notification) -> (Vec<Request>, Outcome) {
        if notification.id == spawned_contract()
            && let Some(spawned) = Spawned::decode(&notification.payload)
        {
            self.me = Some(spawned.id);
        }
        (Vec::new(), Outcome::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::ReferenceSystemReducer;
    use crate::{
        Bytes, ContractId, Deliver, Delivered, Error, HostId, InMemoryReducerGraph, Message,
        Origin, Outcome, Reducer, ReducerGraph, ReducerId, Spawned, deliver_contract,
    };
    use std::sync::Arc;

    fn cid(tag: &[u8]) -> ContractId {
        ContractId::of(tag)
    }
    fn rid(tag: &[u8]) -> ReducerId {
        ReducerId::of(tag)
    }

    // The effect as the system routes it to the system reducer: a message on the contract, from the emitter.
    fn effect(emitter: ReducerId, contract: ContractId) -> Message {
        Message {
            id: contract,
            payload: Bytes::from_static(b"the effect body"),
            from: Origin {
                reducer: emitter,
                host: HostId::of(b"node"),
            },
            continuation_token: Bytes::from_static(b"leaf-token"),
        }
    }

    // Drive the reference reducer's birth so it knows its own id (as the system does before the first effect).
    async fn born(reducer: &mut ReferenceSystemReducer, me: ReducerId, parent: ReducerId) {
        let birth = Spawned { id: me, parent }.into_notification();
        reducer.on_notification(birth).await;
    }

    // Decode the single deliver a fold emitted, returning its envelope.
    fn sole_deliver(requests: &[crate::Request]) -> Deliver {
        assert_eq!(requests.len(), 1, "exactly one deliver");
        assert_eq!(requests[0].id, deliver_contract(), "it is a deliver");
        Deliver::decode(&requests[0].payload).expect("a well-formed deliver envelope")
    }

    #[tokio::test]
    async fn delivers_the_effect_to_the_first_handler_stamped_from_the_context() {
        let graph = Arc::new(InMemoryReducerGraph::new());
        let (context, emitter, authz, edge) =
            (rid(b"ctx"), rid(b"emitter"), rid(b"authz"), rid(b"edge"));
        for id in [context, emitter, authz, edge] {
            graph.insert(id).await;
        }
        // The emitter's own segment answers http.get with [authz, edge] (model B: dispatch reads the
        // emitter's materialized chain).
        graph
            .set_chain(emitter, cid(b"http.get"), vec![authz, edge])
            .await;

        let mut reducer = ReferenceSystemReducer::new(Arc::clone(&graph) as _);
        born(&mut reducer, context, emitter).await;
        let (requests, outcome) = reducer.on_message(effect(emitter, cid(b"http.get"))).await;

        // It delivers the effect to the first handler, from the context (so the reply comes back here).
        let deliver = sole_deliver(&requests);
        assert_eq!(deliver.target, authz);
        match deliver.event {
            Delivered::Message(m) => {
                assert_eq!(m.id, cid(b"http.get"));
                assert_eq!(m.payload, Bytes::from_static(b"the effect body"));
                assert_eq!(
                    m.from.reducer, context,
                    "the handler replies to the context"
                );
                assert_ne!(
                    m.continuation_token,
                    Bytes::from_static(b"leaf-token"),
                    "the handler never sees the leaf's token"
                );
            }
            other => panic!("expected a delivered message, got {other:?}"),
        }
        // The dispatch stays open, awaiting the handler's reply (folded in a later slice).
        assert_eq!(outcome, Outcome::Continue);
    }

    #[tokio::test]
    async fn answers_missing_handler_when_the_contract_has_no_chain() {
        let graph = Arc::new(InMemoryReducerGraph::new());
        let (context, emitter) = (rid(b"ctx"), rid(b"emitter"));
        graph.insert(context).await;
        graph.insert(emitter).await;
        // No chain set for the contract.

        let mut reducer = ReferenceSystemReducer::new(Arc::clone(&graph) as _);
        born(&mut reducer, context, emitter).await;
        let (requests, outcome) = reducer
            .on_message(effect(emitter, cid(b"nobody.answers")))
            .await;

        // It answers the emitter Err(MissingHandler), correlated by the emitter's own token, and closes.
        let deliver = sole_deliver(&requests);
        assert_eq!(deliver.target, emitter);
        match deliver.event {
            Delivered::Response(r) => {
                assert_eq!(r.id, cid(b"nobody.answers"));
                assert_eq!(r.continuation_token, Bytes::from_static(b"leaf-token"));
                assert_eq!(r.payload, Err(Error::MissingHandler));
            }
            other => panic!("expected a delivered response, got {other:?}"),
        }
        assert!(matches!(outcome, Outcome::Break { .. }));
    }
}
