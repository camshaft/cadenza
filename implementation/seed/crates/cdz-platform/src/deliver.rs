//! The deliver primitive (`design/cadenza-platform.md` §4).
//!
//! Delivering an event into a reducer's log is the one privileged thing an event reducer does that an
//! ordinary reducer cannot: it is the routing act — handing a message to the next handler, folding a
//! response back to a caller. An event reducer emits it as an ordinary [`Request`] against the
//! [`deliver_contract`], whose payload is a [`Deliver`] envelope naming the target reducer and the event to
//! inject.
//!
//! The deliver contract is a real [`Contract`] like any other — built through [`Contract::new`] so its id is
//! the hash of its declared schema, not of a bare name — and cached once. Its schema is not hand-authored:
//! it lives as real Cadenza in `contracts/deliver.sexp`, which `cargo xtask codegen` typechecks with the
//! compiler and projects into the `deliver_schema` module, so the schema is provably valid Cadenza and
//! cannot silently drift into something the language would reject.
//!
//! The envelope is a Cadenza value encoded through the one canonical binary codec ([`cadenza_ast::codec`]),
//! the same encoding every value on the wire uses; there is no bespoke format. Both its schema and its
//! value builders/readers are generated from `contracts/deliver.cdz` (`crate::contracts::deliver`), so the
//! value has the shape the schema declares — an `Envelope.Deliver` of a record whose `event` is one of the
//! `Event` variants — and type-ascribes against the schema (the source's conformance tests prove this at
//! codegen time). The user's own payload rides inside the event as opaque [`Bytes`]. Decoding is total:
//! [`Deliver::decode`] returns `None` on any input that is not a well-formed deliver value, so a bad
//! envelope is a rejected deliver, not a panic.

use crate::{
    Bytes, Contract, ContractId, Error, HostId, Message, Notification, Origin, ReducerId, Request,
    Response,
};
use cadenza_ast::ast::{Builder, StructId};
use cadenza_ast::codec;
use std::sync::OnceLock;

/// The single built-in contract the kernel recognizes: a [`Request`](crate::Request) against it is a deliver
/// (§4). It is a real contract whose id is the hash of its declared schema — the compiler-checked
/// `deliver_schema` module generated from `contracts/deliver.sexp`; the contract value is built once and
/// cached, so the id is derived only once.
#[must_use]
pub fn deliver_contract() -> ContractId {
    static DELIVER: OnceLock<Contract> = OnceLock::new();
    DELIVER
        .get_or_init(crate::contracts::deliver::contract)
        .id()
}

/// An event delivered to a reducer, selecting the entry point it folds through: the three kinds an ordinary
/// [`Reducer`](crate::Reducer) receives. The runtime dispatches each to `on_message` / `on_response` /
/// `on_notification`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delivered {
    /// Deliver to `on_message` — an effect performed on the reducer.
    Message(Message),
    /// Deliver to `on_response` — a reply to a request the reducer performed.
    Response(Response),
    /// Deliver to `on_notification` — a platform control-plane event.
    Notification(Notification),
}

/// A deliver envelope: inject `event` into the log of the reducer named by `target` (§4). This is what the
/// payload of a deliver [`Request`](crate::Request) carries; the runtime decodes it and runs the target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Deliver {
    /// The reducer to deliver the event to.
    pub target: ReducerId,
    /// The event to inject into its log.
    pub event: Delivered,
}

impl Deliver {
    /// Encode the envelope as a Cadenza value in the canonical binary form (`cadenza_ast::codec`) — the same
    /// encoding every value on the wire uses. The inverse of [`decode`](Self::decode).
    #[must_use]
    pub fn encode(&self) -> Bytes {
        let mut b = Builder::new();
        let root = self.build(&mut b);
        let arenas = b.finish(root);
        Bytes::from(codec::encode(&arenas))
    }

    /// Build the envelope value into `b`, returning its root — a value of the schema type `Envelope`, so it
    /// type-ascribes against the contract's schema. The value SHAPE is entirely the generated builders'
    /// (`contracts::deliver::*`, which the same source generates as the schema, so they cannot drift); this
    /// only supplies the `Bytes` leaves and, for a response, the prelude `Result` value (`Ok`/`Err` are
    /// prelude constructors, not in this contract's schema, so they use the generic [`v::bare_ctor`]).
    fn build(&self, b: &mut Builder) -> StructId {
        use crate::contract_value as v;
        use crate::contracts::deliver as c;

        let target = v::bytes_leaf(b, self.target.hash().as_bytes());
        let event = match &self.event {
            Delivered::Message(m) => {
                let id = v::bytes_leaf(b, m.id.hash().as_bytes());
                let reducer = v::bytes_leaf(b, m.from.reducer.hash().as_bytes());
                let host = v::bytes_leaf(b, m.from.host.hash().as_bytes());
                let payload = v::bytes_leaf(b, &m.payload);
                let token = v::bytes_leaf(b, &m.continuation_token);
                c::event_message(
                    b,
                    c::EventMessage {
                        id,
                        reducer,
                        host,
                        payload,
                        token,
                    },
                )
            }
            Delivered::Response(r) => {
                let id = v::bytes_leaf(b, r.id.hash().as_bytes());
                let token = v::bytes_leaf(b, &r.continuation_token);
                let result = match &r.payload {
                    Ok(value) => {
                        let x = v::bytes_leaf(b, value);
                        v::bare_ctor(b, "Ok", vec![x])
                    }
                    Err(error) => {
                        let e = match error {
                            Error::Timeout => c::error_timeout(b),
                            Error::MissingHandler => c::error_missing_handler(b),
                            Error::SchemaViolation => c::error_schema_violation(b),
                        };
                        v::bare_ctor(b, "Err", vec![e])
                    }
                };
                c::event_response(b, c::EventResponse { id, token, result })
            }
            Delivered::Notification(n) => {
                let id = v::bytes_leaf(b, n.id.hash().as_bytes());
                let payload = v::bytes_leaf(b, &n.payload);
                c::event_notification(b, c::EventNotification { id, payload })
            }
        };
        c::envelope_deliver(b, c::EnvelopeDeliver { target, event })
    }

    /// The deliver [`Request`](crate::Request) an event reducer emits to carry this out: against the
    /// [`deliver_contract`], with the envelope as its payload. It correlates nothing of the event reducer's
    /// own (the target's reply routes by the delivered event's token), so it carries an empty token and no
    /// deadline.
    #[must_use]
    pub fn into_request(self) -> Request {
        Request {
            id: deliver_contract(),
            payload: self.encode(),
            continuation_token: Bytes::new(),
            deadline: None,
        }
    }

    /// Decode an envelope from a Cadenza value, or `None` if the bytes are not a well-formed deliver value.
    /// Total, so a malformed envelope is a rejected deliver, never a panic.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        use crate::contract_value as v;
        use crate::contracts::deliver as c;

        let arenas = codec::decode(bytes)?;
        let env = c::as_envelope_deliver(&arenas, arenas.root)?;
        let target = ReducerId::from_hash(v::read_hash(&arenas, env.target)?);
        let event = decode_event(&arenas, env.event)?;
        Some(Self { target, event })
    }
}

/// Read a delivered event value (an `Event` variant) back into a [`Delivered`], using the generated
/// readers for the shape and [`v`](crate::contract_value) for the leaves. `None` on any mismatch.
fn decode_event(arenas: &cadenza_ast::ast::Arenas, id: StructId) -> Option<Delivered> {
    use crate::contract_value as v;
    use crate::contracts::deliver as c;

    if let Some(m) = c::as_event_message(arenas, id) {
        return Some(Delivered::Message(Message {
            id: ContractId::from_hash(v::read_hash(arenas, m.id)?),
            from: Origin {
                reducer: ReducerId::from_hash(v::read_hash(arenas, m.reducer)?),
                host: HostId::from_hash(v::read_hash(arenas, m.host)?),
            },
            payload: v::read_bytes(arenas, m.payload)?,
            continuation_token: v::read_bytes(arenas, m.token)?,
        }));
    }
    if let Some(r) = c::as_event_response(arenas, id) {
        return Some(Delivered::Response(Response {
            id: ContractId::from_hash(v::read_hash(arenas, r.id)?),
            continuation_token: v::read_bytes(arenas, r.token)?,
            payload: decode_result(arenas, r.result)?,
        }));
    }
    if let Some(n) = c::as_event_notification(arenas, id) {
        return Some(Delivered::Notification(Notification {
            id: ContractId::from_hash(v::read_hash(arenas, n.id)?),
            payload: v::read_bytes(arenas, n.payload)?,
        }));
    }
    None
}

/// Read a `Result(Bytes, Error)` value — the prelude `Ok`/`Err` constructors (via the generic bare-ctor
/// reader), the `Err` payload being a generated `Error` value.
fn decode_result(arenas: &cadenza_ast::ast::Arenas, id: StructId) -> Option<Result<Bytes, Error>> {
    use crate::contract_value as v;
    use crate::contracts::deliver as c;

    if let Some([value]) = v::as_bare_ctor(arenas, id, "Ok").and_then(one) {
        return Some(Ok(v::read_bytes(arenas, value)?));
    }
    if let Some([err]) = v::as_bare_ctor(arenas, id, "Err").and_then(one) {
        return Some(Err(if c::is_error_timeout(arenas, err) {
            Error::Timeout
        } else if c::is_error_missing_handler(arenas, err) {
            Error::MissingHandler
        } else if c::is_error_schema_violation(arenas, err) {
            Error::SchemaViolation
        } else {
            return None;
        }));
    }
    None
}

/// A slice of exactly `N` occurrences as a fixed array, or `None`.
fn one<const N: usize>(items: &[StructId]) -> Option<[StructId; N]> {
    <[StructId; N]>::try_from(items).ok()
}

#[cfg(test)]
mod tests {
    use super::{Deliver, Delivered, deliver_contract};
    use crate::{
        Bytes, ContractId, Error, HostId, Message, Notification, Origin, ReducerId, Response,
    };

    fn cid(tag: &[u8]) -> ContractId {
        ContractId::of(tag)
    }
    fn rid(tag: &[u8]) -> ReducerId {
        ReducerId::of(tag)
    }
    fn origin() -> Origin {
        Origin {
            reducer: rid(b"peer"),
            host: HostId::of(b"host-a"),
        }
    }

    fn round_trips(d: &Deliver) {
        assert_eq!(Deliver::decode(&d.encode()), Some(d.clone()));
    }

    #[test]
    fn a_message_deliver_round_trips_through_the_codec() {
        round_trips(&Deliver {
            target: rid(b"handler"),
            event: Delivered::Message(Message {
                id: cid(b"http.get"),
                payload: Bytes::from_static(b"a request body"),
                from: origin(),
                continuation_token: Bytes::from_static(b"tok"),
            }),
        });
    }

    #[test]
    fn a_response_deliver_round_trips_for_ok_and_every_error() {
        for payload in [
            Ok(Bytes::from_static(b"answer")),
            Err(Error::Timeout),
            Err(Error::MissingHandler),
            Err(Error::SchemaViolation),
        ] {
            round_trips(&Deliver {
                target: rid(b"caller"),
                event: Delivered::Response(Response {
                    id: cid(b"http.get"),
                    continuation_token: Bytes::from_static(b"tok"),
                    payload,
                }),
            });
        }
    }

    #[test]
    fn a_notification_deliver_round_trips() {
        round_trips(&Deliver {
            target: rid(b"child"),
            event: Delivered::Notification(Notification {
                id: cid(b"handler-available"),
                payload: Bytes::from_static(b""),
            }),
        });
    }

    #[test]
    fn decode_rejects_bytes_that_are_not_a_deliver_value() {
        // Not a valid encoding at all.
        assert_eq!(Deliver::decode(&[0xFF, 0x00, 0x13, 0x37]), None);
        assert_eq!(Deliver::decode(&[]), None);
        // A well-formed Cadenza value of the wrong shape decodes cleanly but is not a deliver.
        let mut b = cadenza_ast::ast::Builder::new();
        let root = b.name("not-a-deliver");
        let wrong_shape = cadenza_ast::codec::encode(&b.finish(root));
        assert_eq!(Deliver::decode(&wrong_shape), None);
    }

    #[test]
    fn the_encoded_value_has_the_shape_the_schema_declares() {
        // The value must be an `Envelope.Deliver` of a record whose `event` is an `Event` variant — the
        // shape the generated builders produce and the source's conformance tests prove type-ascribes. This
        // reads it back through the generated readers, so a regression to a bespoke shape fails here.
        use crate::contract_value as v;
        use crate::contracts::deliver as c;

        let d = Deliver {
            target: rid(b"handler"),
            event: Delivered::Message(Message {
                id: cid(b"http.get"),
                payload: Bytes::from_static(b"body"),
                from: origin(),
                continuation_token: Bytes::from_static(b"tok"),
            }),
        };
        let arenas = cadenza_ast::codec::decode(&d.encode()).expect("well-formed value");
        let env = c::as_envelope_deliver(&arenas, arenas.root).expect("an Envelope.Deliver value");
        // `event` is an `Event.Message` whose fields read back by name, the payload being the request body.
        let m = c::as_event_message(&arenas, env.event).expect("an Event.Message value");
        assert_eq!(
            v::read_bytes(&arenas, m.payload).as_deref(),
            Some(b"body".as_slice())
        );
    }

    #[test]
    fn the_deliver_contract_id_is_a_real_contract_id_and_stable() {
        // Stable across calls (cached), and equal to the id a fresh Contract with the same schema derives —
        // i.e. it is the hash of the declared schema, not of a bare name.
        assert_eq!(deliver_contract(), deliver_contract());
        let rebuilt = crate::Contract::new(
            crate::Str::from_static("cdz-platform.deliver"),
            crate::contracts::deliver::schema,
            "Envelope",
            "Outcome",
        );
        assert_eq!(deliver_contract(), rebuilt.id());
    }
}
