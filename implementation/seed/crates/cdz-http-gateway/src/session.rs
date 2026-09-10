//! The control-link back-channel (`DESIGN-http-outpost-drive-contract.md` §3) — how a handler's
//! `control.send` effect reaches the control server and how the control server's response reaches back.
//!
//! A driven program emits `control.send` (an opaque payload) as a fire-and-forget effect (§2). The gateway:
//!  1. forwards it UP the persistent control link as a [`ControlUp`] via the [`ControlSink`] (the link's
//!     write half, drained by the control-link task), and
//!  2. registers the emitting reducer's mailbox in [`Sessions`] keyed by the effect's `continuation_token`
//!     (the correlation), so when the control server answers with a `ControlDown` carrying that correlation
//!     the gateway folds the payload back into that reducer as its `on_response` — the same fire-and-forget
//!     shape every other effect uses (§1). The reducer's drive loop stays alive (`Continue`) meanwhile.
//!
//! The gateway is a pure opaque router here: it never inspects a `control.send`/`ControlDown` payload, only
//! its addressing (correlation) — the envelope is binary-AST and self-describing.

use bytes::Bytes;
use cdz_http_protocol::ControlUp;
use cdz_platform::{ContractId, Delivered, Response};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::UnboundedSender;

/// The write half of the control link: the resolver hands a handler's `control.send` UP as a [`ControlUp`];
/// the control-link task drains these and writes them to the ws. An unbounded mpsc so the resolver never
/// blocks the drive loop on the socket (fire-and-forget). A closed channel (link down) drops the send — the
/// awaiting reducer simply never gets a response, exactly as a real timeout would present.
pub type ControlSink = UnboundedSender<ControlUp>;

/// Inject a [`Delivered`] event into the reducer that emitted a `control.send` — a type-erased handle to its
/// mailbox `send`, so [`Sessions`] stays free of the resolver's `Runtime` generic (the resolver builds this
/// as `move |d| R::send(&mailbox, d)`).
type Inject = Box<dyn Fn(Delivered) + Send + Sync>;

/// What a pending `control.send` is waiting on: how to inject its response back into the emitting reducer,
/// plus the effect's contract-id so the folded-back [`Response`] carries the id its `on_response` expects.
struct Pending {
    inject: Inject,
    effect: ContractId,
}

/// The pending-`control.send` registry: correlation token → the reducer awaiting that response. A
/// `control.send` registers its correlation; a `ControlDown` with that correlation folds the response back
/// and removes the entry. Shared (an `Arc<Mutex<…>>`) between the control-link read task (which routes
/// `ControlDown`) and the effect resolver (which registers on `control.send`). Cheaply cloneable.
#[derive(Clone, Default)]
pub struct Sessions(Arc<Mutex<HashMap<Bytes, Pending>>>);

impl Sessions {
    /// A fresh, empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register the reducer awaiting the control-server response to a `control.send` with `correlation`:
    /// `inject` folds an event back into its mailbox, and `effect` (the `control.send` contract-id) is the id
    /// the folded [`Response`] carries. A duplicate correlation overwrites the older entry (last writer wins).
    pub fn register(
        &self,
        correlation: Bytes,
        effect: ContractId,
        inject: impl Fn(Delivered) + Send + Sync + 'static,
    ) {
        if let Ok(mut map) = self.0.lock() {
            map.insert(
                correlation,
                Pending {
                    inject: Box::new(inject),
                    effect,
                },
            );
        }
    }

    /// Route a control-server response `payload` correlated by `correlation` back to the reducer that emitted
    /// the `control.send`: fold it as that reducer's `on_response`, then drop the pending entry. A correlation
    /// with no pending reducer (already answered, or an unsolicited push) is dropped — never an error.
    pub fn route_response(&self, correlation: &Bytes, payload: Bytes) {
        let pending = self
            .0
            .lock()
            .ok()
            .and_then(|mut map| map.remove(correlation));
        if let Some(Pending { inject, effect }) = pending {
            inject(Delivered::Response(Response {
                id: effect,
                continuation_token: correlation.clone(),
                payload: Ok(payload),
            }));
        }
    }

    /// Drop a pending entry (e.g. its reducer finished without waiting) so the map does not leak a handle for
    /// a `control.send` that will never be answered.
    pub fn forget(&self, correlation: &Bytes) {
        if let Ok(mut map) = self.0.lock() {
            map.remove(correlation);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_response_folds_the_reply_into_the_registered_mailbox_then_drops_it() {
        let sessions = Sessions::new();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Delivered>();
        let correlation = Bytes::from_static(b"corr-1");
        let effect = ContractId::of(b"cdz-platform.control.send");
        sessions.register(correlation.clone(), effect, move |event| {
            let _ = tx.send(event);
        });

        // The control-server response folds back as a `Response` correlated by the token, carrying the
        // `control.send` effect id its `on_response` expects.
        sessions.route_response(&correlation, Bytes::from_static(b"answer"));
        match rx.try_recv() {
            Ok(Delivered::Response(r)) => {
                assert_eq!(r.continuation_token, correlation);
                assert_eq!(r.id, effect);
                assert_eq!(r.payload.unwrap(), Bytes::from_static(b"answer"));
            }
            other => panic!("expected a Response, got {other:?}"),
        }

        // The entry was removed: a second response for the same correlation delivers nothing (idempotent /
        // no double-fold), and an unregistered correlation is a silent no-op.
        sessions.route_response(&correlation, Bytes::from_static(b"again"));
        sessions.route_response(&Bytes::from_static(b"unknown"), Bytes::from_static(b"x"));
        assert!(rx.try_recv().is_err());
    }
}
