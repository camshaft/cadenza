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
//! the same encoding every value on the wire uses; there is no bespoke format. Crucially the value has the
//! shape the schema declares — a `Deliver-Envelope.Deliver` of a record whose `event` is one of the `Event`
//! variants — so it type-ascribes against the schema (the source's conformance values prove this at codegen
//! time). The user's own payload rides inside the event as opaque [`Bytes`]. Decoding is total:
//! [`Deliver::decode`] returns `None` on any input that is not a well-formed deliver value, so a bad
//! envelope is a rejected deliver, not a panic.

use crate::{
    Bytes, Contract, ContractId, Error, Hash, HostId, Message, Notification, Origin, ReducerId,
    Request, Response, Str,
};
use cadenza_ast::ast::{Builder, Leaf, Struct, StructId};
use cadenza_ast::codec;
use std::sync::{Arc, OnceLock};

/// The single built-in contract the kernel recognizes: a [`Request`](crate::Request) against it is a deliver
/// (§4). It is a real contract whose id is the hash of its declared schema — the compiler-checked
/// `deliver_schema` module generated from `contracts/deliver.sexp`; the contract value is built once and
/// cached, so the id is derived only once.
#[must_use]
pub fn deliver_contract() -> ContractId {
    static DELIVER: OnceLock<Contract> = OnceLock::new();
    DELIVER
        .get_or_init(|| {
            Contract::new(
                Str::from_static("cdz-platform.deliver"),
                crate::deliver_schema::schema,
                "Deliver-Envelope",
                "Deliver-Outcome",
            )
        })
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

    /// Build the envelope value into `b`, returning its root — the value form the schema declares, so it
    /// type-ascribes against `Deliver-Envelope`:
    /// `(Deliver-Envelope.Deliver (record (target <bytes>) (event <event>)))`, where `<event>` is one of
    /// the `Event` variants (`Event.Message` / `Event.Response` / `Event.Notification`) applied to its
    /// record. Each hash and opaque payload is a `Bytes` leaf.
    fn build(&self, b: &mut Builder) -> StructId {
        let target = bytes_leaf(b, self.target.hash().as_bytes());
        let event = match &self.event {
            Delivered::Message(m) => {
                let id = bytes_leaf(b, m.id.hash().as_bytes());
                let reducer = bytes_leaf(b, m.from.reducer.hash().as_bytes());
                let host = bytes_leaf(b, m.from.host.hash().as_bytes());
                let payload = bytes_leaf(b, &m.payload);
                let token = bytes_leaf(b, &m.continuation_token);
                let rec = record(
                    b,
                    &[
                        ("id", id),
                        ("reducer", reducer),
                        ("host", host),
                        ("payload", payload),
                        ("token", token),
                    ],
                );
                ctor(b, "Event.Message", rec)
            }
            Delivered::Response(r) => {
                let id = bytes_leaf(b, r.id.hash().as_bytes());
                let token = bytes_leaf(b, &r.continuation_token);
                let result = match &r.payload {
                    Ok(value) => {
                        let v = bytes_leaf(b, value);
                        ctor(b, "Result.Ok", v)
                    }
                    Err(error) => {
                        let e = ctor_nullary(b, error_ctor(*error));
                        ctor(b, "Result.Err", e)
                    }
                };
                let rec = record(b, &[("id", id), ("token", token), ("result", result)]);
                ctor(b, "Event.Response", rec)
            }
            Delivered::Notification(n) => {
                let id = bytes_leaf(b, n.id.hash().as_bytes());
                let payload = bytes_leaf(b, &n.payload);
                let rec = record(b, &[("id", id), ("payload", payload)]);
                ctor(b, "Event.Notification", rec)
            }
        };
        let rec = record(b, &[("target", target), ("event", event)]);
        ctor(b, "Deliver-Envelope.Deliver", rec)
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
        let arenas = codec::decode(bytes)?;
        let [rec] = as_form(&arenas, arenas.root, "Deliver-Envelope.Deliver")?;
        let target =
            ReducerId::from_hash(read_hash(&arenas, record_field(&arenas, rec, "target")?)?);
        let event = decode_event(&arenas, record_field(&arenas, rec, "event")?)?;
        Some(Self { target, event })
    }
}

fn decode_event(arenas: &cadenza_ast::ast::Arenas, id: StructId) -> Option<Delivered> {
    if let Some([rec]) = as_form(arenas, id, "Event.Message") {
        return Some(Delivered::Message(Message {
            id: ContractId::from_hash(read_hash(arenas, record_field(arenas, rec, "id")?)?),
            from: Origin {
                reducer: ReducerId::from_hash(read_hash(
                    arenas,
                    record_field(arenas, rec, "reducer")?,
                )?),
                host: HostId::from_hash(read_hash(arenas, record_field(arenas, rec, "host")?)?),
            },
            payload: read_bytes(arenas, record_field(arenas, rec, "payload")?)?,
            continuation_token: read_bytes(arenas, record_field(arenas, rec, "token")?)?,
        }));
    }
    if let Some([rec]) = as_form(arenas, id, "Event.Response") {
        return Some(Delivered::Response(Response {
            id: ContractId::from_hash(read_hash(arenas, record_field(arenas, rec, "id")?)?),
            continuation_token: read_bytes(arenas, record_field(arenas, rec, "token")?)?,
            payload: decode_result(arenas, record_field(arenas, rec, "result")?)?,
        }));
    }
    if let Some([rec]) = as_form(arenas, id, "Event.Notification") {
        return Some(Delivered::Notification(Notification {
            id: ContractId::from_hash(read_hash(arenas, record_field(arenas, rec, "id")?)?),
            payload: read_bytes(arenas, record_field(arenas, rec, "payload")?)?,
        }));
    }
    None
}

fn decode_result(arenas: &cadenza_ast::ast::Arenas, id: StructId) -> Option<Result<Bytes, Error>> {
    if let Some([value]) = as_form(arenas, id, "Result.Ok") {
        return Some(Ok(read_bytes(arenas, value)?));
    }
    if let Some([err]) = as_form(arenas, id, "Result.Err") {
        // The error is a nullary constructor of the `Error` type: `(Error.Timeout)` / `(Error.MissingHandler)`.
        return Some(Err(match arenas.head_name(err)? {
            "Error.Timeout" => Error::Timeout,
            "Error.MissingHandler" => Error::MissingHandler,
            _ => return None,
        }));
    }
    None
}

/// The constructor spelling of an [`Error`] as the schema declares it (the `Error` type's variant names).
fn error_ctor(error: Error) -> &'static str {
    match error {
        Error::Timeout => "Error.Timeout",
        Error::MissingHandler => "Error.MissingHandler",
    }
}

// --- value builder helpers ---

/// A constructor application `(<ctor> <payload>)` — a sum value: the dotted constructor name applied to its
/// single payload (e.g. `Event.Message` applied to its record).
fn ctor(b: &mut Builder, name: &str, payload: StructId) -> StructId {
    let head = b.name(name);
    b.list(vec![head, payload])
}

/// A nullary constructor value `(<ctor>)` — a sum variant with no payload (e.g. `Error.Timeout`).
fn ctor_nullary(b: &mut Builder, name: &str) -> StructId {
    let head = b.name(name);
    b.list(vec![head])
}

/// A record value `(record (<field> <value>)…)` — the head `record`, then one `(name value)` form per
/// field, in the given order (fields are read back by name, so the order is not load-bearing).
fn record(b: &mut Builder, fields: &[(&str, StructId)]) -> StructId {
    let head = b.name("record");
    let mut children = Vec::with_capacity(1 + fields.len());
    children.push(head);
    for &(name, value) in fields {
        let name = b.name(name);
        children.push(b.list(vec![name, value]));
    }
    b.list(children)
}

fn bytes_leaf(b: &mut Builder, bytes: &[u8]) -> StructId {
    b.atom_leaf(Leaf::Bytes(Arc::from(bytes)))
}

// --- value reader helpers ---

/// A list `(head child…)` headed by the exact name `head`, returned as its fixed-arity children, or `None`
/// if the shape or arity does not match. `N` is the number of children expected after the head.
fn as_form<const N: usize>(
    arenas: &cadenza_ast::ast::Arenas,
    id: StructId,
    head: &str,
) -> Option<[StructId; N]> {
    let tail = arenas.as_form(id, head)?;
    <[StructId; N]>::try_from(tail).ok()
}

/// The value of a record's field named `name` — the `<value>` of the `(name value)` form inside a
/// `(record …)` value. `None` if `id` is not a record or has no such field. Reads by name, so a value
/// built with fields in any order decodes the same.
fn record_field(arenas: &cadenza_ast::ast::Arenas, id: StructId, name: &str) -> Option<StructId> {
    let fields = arenas.as_form(id, "record")?;
    fields.iter().find_map(|&f| {
        let [value] = as_form(arenas, f, name)?;
        Some(value)
    })
}

fn read_bytes(arenas: &cadenza_ast::ast::Arenas, id: StructId) -> Option<Bytes> {
    match arenas.get(id) {
        Struct::Atom(leaf) => match arenas.leaf(*leaf) {
            Leaf::Bytes(bytes) => Some(Bytes::copy_from_slice(bytes)),
            _ => None,
        },
        Struct::List(_) => None,
    }
}

fn read_hash(arenas: &cadenza_ast::ast::Arenas, id: StructId) -> Option<Hash> {
    let bytes = read_bytes(arenas, id)?;
    Some(Hash::from_bytes(
        <[u8; Hash::LEN]>::try_from(bytes.as_ref()).ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::{Deliver, Delivered, deliver_contract};
    use crate::{
        Bytes, ContractId, Error, Hash, HostId, Message, Notification, Origin, ReducerId, Response,
    };

    fn cid(tag: &[u8]) -> ContractId {
        ContractId::from_hash(Hash::of(tag))
    }
    fn rid(tag: &[u8]) -> ReducerId {
        ReducerId::from_hash(Hash::of(tag))
    }
    fn origin() -> Origin {
        Origin {
            reducer: rid(b"peer"),
            host: HostId::from_hash(Hash::of(b"host-a")),
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
    fn a_response_deliver_round_trips_for_ok_and_both_errors() {
        for payload in [
            Ok(Bytes::from_static(b"answer")),
            Err(Error::Timeout),
            Err(Error::MissingHandler),
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
        // The value must be a `Deliver-Envelope.Deliver` of a record whose `event` is an `Event` variant —
        // the shape `contracts/deliver.sexp` declares and its conformance defs prove type-ascribes. This
        // pins the form so a regression to a bespoke shape (which would NOT ascribe) fails here.
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
        // root = (Deliver-Envelope.Deliver <record>)
        let env = arenas
            .as_form(arenas.root, "Deliver-Envelope.Deliver")
            .expect("a Deliver-Envelope.Deliver constructor");
        assert_eq!(
            env.len(),
            1,
            "the constructor carries one payload (its record)"
        );
        // its record carries `target` and `event` fields, and `event` is an `Event.Message` value.
        let rec = env[0];
        assert_eq!(arenas.head_name(rec), Some("record"));
        let event = super::record_field(&arenas, rec, "event").expect("an `event` field");
        assert_eq!(arenas.head_name(event), Some("Event.Message"));
        assert!(
            super::record_field(&arenas, rec, "target").is_some(),
            "a `target` field"
        );
    }

    #[test]
    fn the_deliver_contract_id_is_a_real_contract_id_and_stable() {
        // Stable across calls (cached), and equal to the id a fresh Contract with the same schema derives —
        // i.e. it is the hash of the declared schema, not of a bare name.
        assert_eq!(deliver_contract(), deliver_contract());
        let rebuilt = crate::Contract::new(
            crate::Str::from_static("cdz-platform.deliver"),
            crate::deliver_schema::schema,
            "Deliver-Envelope",
            "Deliver-Outcome",
        );
        assert_eq!(deliver_contract(), rebuilt.id());
    }
}
