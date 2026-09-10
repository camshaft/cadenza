//! The gateway's effect resolver (`DESIGN-http-outpost-drive-contract.md` §2, redirect inc-3b): routes an
//! effect a looping program emits (a `Request`, resolved by [`drive_loop`](crate::loop_driver::drive_loop))
//! to the gateway action for its contract-id, and produces the response folded back. The gateway is a pure
//! router of effects — it switches on `req.id` and treats each payload as OPAQUE beyond the envelope a given
//! effect needs.
//!
//! This slice implements the **`control.send`** effect (the 4th directive): a program emits it to send an
//! opaque payload to the control server; the gateway wraps it in a [`ControlUp`](crate::codec::ControlUp)
//! envelope stamped with the emitting program's provenance and hands it to a [`ControlSink`], then acks so
//! the program's loop continues. Other effect classes — `dispatch` (fetch + drive a subprogram from the CAS)
//! and timers — are later slices; an unrecognized effect answers `Err(MissingHandler)` so the program can
//! react rather than the gateway guessing. (`http.response`/`deny` are terminal `Break` outcomes the edge
//! reads off [`drive_loop`], not effects resolved here.)

use crate::codec::ControlUp;
use crate::loop_driver::EffectResolver;
use async_trait::async_trait;
use bytes::Bytes;
use cdz_platform::{ContractId, Error, Request, Response};
use std::sync::Arc;

/// The 33-byte marker for the `control.send` effect contract-id (v0; a real derived contract-id later — see
/// the schema-id fix). A program emits a `Request` on this contract to send its payload to the control server.
const CONTROL_SEND_MARKER: &[u8; cdz_platform::Hash::LEN] = b"cdz.control.send.................";

/// The contract-id a program emits to send a message up to the control server.
#[must_use]
pub fn control_send_contract() -> ContractId {
    ContractId::try_from(&CONTROL_SEND_MARKER[..])
        .expect("the control-send marker is Hash::LEN bytes")
}

/// Where the gateway forwards a program's `control.send` message (a [`ControlUp`] envelope) — up the control
/// link. The gateway owns the wire; the sink is the seam the edge wires to [`crate::control_link`]. A test
/// captures the envelopes.
#[async_trait]
pub trait ControlSink: Send + Sync {
    /// Forward one enveloped handler→control message. Best-effort: the program's ack does not depend on the
    /// control server's reply (a reply, if any, arrives later as a `ControlDown` push → an `on_notification`).
    async fn send(&self, msg: ControlUp);
}

/// The gateway's [`EffectResolver`]: routes an emitted effect to its gateway action. Holds the PROVENANCE of
/// the program being driven (its `ProgramHash` bytes + the connection/session id) so a `control.send` is
/// stamped with where it came from, and the [`ControlSink`] the message is forwarded to.
pub struct GatewayResolver {
    /// The driven program's `ProgramHash` bytes — stamped as `ControlUp.program`.
    program: Bytes,
    /// The connection/session id — stamped as `ControlUp.session`.
    session: Bytes,
    /// The up-link sink for `control.send` messages.
    control: Arc<dyn ControlSink>,
}

impl GatewayResolver {
    /// A resolver for a program identified by `program` running for `session`, forwarding `control.send`
    /// messages to `control`.
    #[must_use]
    pub fn new(program: Bytes, session: Bytes, control: Arc<dyn ControlSink>) -> Self {
        Self {
            program,
            session,
            control,
        }
    }

    /// The ack folded back for an effect the gateway performed with no meaningful return value (e.g.
    /// `control.send` — fire-and-forward): an `Ok` empty payload correlated to the request.
    fn ack(req: &Request) -> Response {
        Response {
            id: req.id,
            continuation_token: req.continuation_token.clone(),
            payload: Ok(Bytes::new()),
        }
    }

    /// The answer for an effect the gateway does not handle: `Err(MissingHandler)`, correlated — the program
    /// decides what to do (retry, close, …) rather than the gateway silently dropping it.
    fn unhandled(req: &Request) -> Response {
        Response {
            id: req.id,
            continuation_token: req.continuation_token.clone(),
            payload: Err(Error::MissingHandler),
        }
    }
}

#[async_trait]
impl EffectResolver for GatewayResolver {
    async fn resolve(&self, req: Request) -> Response {
        if req.id == control_send_contract() {
            // Wrap the program's opaque payload with provenance and forward it up the control link.
            self.control
                .send(ControlUp {
                    program: self.program.clone(),
                    session: self.session.clone(),
                    payload: req.payload.clone(),
                })
                .await;
            Self::ack(&req)
        } else {
            Self::unhandled(&req)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loop_driver::drive_loop;
    use cdz_platform::{
        HostId, Message, Notification, Origin, Outcome, Reducer, ReducerId, Request as PRequest,
    };
    use std::sync::Mutex;

    /// A `ControlSink` that captures every forwarded envelope for assertion.
    #[derive(Default)]
    struct CapturingSink(Mutex<Vec<ControlUp>>);
    #[async_trait]
    impl ControlSink for CapturingSink {
        async fn send(&self, msg: ControlUp) {
            self.0.lock().expect("sink lock").push(msg);
        }
    }

    fn msg() -> Message {
        Message {
            id: ContractId::of(b"cdz.http.request"),
            payload: Bytes::from_static(b"req"),
            from: Origin {
                reducer: ReducerId::of(b"edge"),
                host: HostId::of(b"host"),
            },
            continuation_token: Bytes::new(),
        }
    }

    #[tokio::test]
    async fn a_control_send_effect_is_enveloped_forwarded_and_acked() {
        // A program that on its first message emits a control.send effect, and on the ack Breaks.
        struct Sender;
        #[async_trait]
        impl Reducer for Sender {
            async fn on_message(&mut self, _m: Message) -> (Vec<PRequest>, Outcome) {
                (
                    vec![PRequest {
                        id: control_send_contract(),
                        payload: Bytes::from_static(b"hello control server"),
                        continuation_token: Bytes::from_static(b"c1"),
                        deadline: None,
                    }],
                    Outcome::Continue,
                )
            }
            async fn on_response(&mut self, r: Response) -> (Vec<PRequest>, Outcome) {
                // The ack is an Ok empty — close once the message is confirmed forwarded.
                assert!(r.payload.is_ok(), "control.send is acked Ok");
                (
                    vec![],
                    Outcome::Break {
                        schema: ContractId::of(b"cdz.http.response"),
                        reason: Bytes::from_static(b"sent"),
                    },
                )
            }
            async fn on_notification(&mut self, _n: Notification) -> (Vec<PRequest>, Outcome) {
                (vec![], Outcome::Continue)
            }
        }

        let sink = Arc::new(CapturingSink::default());
        let resolver = GatewayResolver::new(
            Bytes::from_static(b"prog-hash"),
            Bytes::from_static(b"session-7"),
            sink.clone(),
        );
        let mut reducer = Sender;
        let out = drive_loop(&mut reducer, msg(), &resolver, 100).await;
        assert_eq!(out.map(|(_, r)| r), Ok(Bytes::from_static(b"sent")));

        // The gateway wrapped the payload with provenance and forwarded exactly one envelope.
        let captured = sink.0.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(
            captured[0],
            ControlUp {
                program: Bytes::from_static(b"prog-hash"),
                session: Bytes::from_static(b"session-7"),
                payload: Bytes::from_static(b"hello control server"),
            }
        );
    }

    #[tokio::test]
    async fn an_unhandled_effect_is_answered_missing_handler() {
        let sink = Arc::new(CapturingSink::default());
        let resolver = GatewayResolver::new(Bytes::new(), Bytes::new(), sink.clone());
        let resp = resolver
            .resolve(PRequest {
                id: ContractId::of(b"some.unknown.effect"),
                payload: Bytes::from_static(b"x"),
                continuation_token: Bytes::from_static(b"c"),
                deadline: None,
            })
            .await;
        assert_eq!(resp.payload, Err(Error::MissingHandler));
        assert!(sink.0.lock().unwrap().is_empty(), "no envelope forwarded");
    }
}
