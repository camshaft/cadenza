//! The deliver primitive (`design/cadenza-platform.md` §4).
//!
//! Delivering an event into a reducer's log is the one privileged thing an event reducer does that an
//! ordinary reducer cannot: it is the routing act — handing a message to the next handler, folding a
//! response back to a caller. An event reducer emits it as an ordinary [`Request`] against the
//! [`deliver_contract`], whose payload is a [`Deliver`] envelope naming the target reducer and the event to
//! inject.
//!
//! The deliver contract is a real [`Contract`] like any other — built through [`Contract::new`] so its id is
//! the hash of its declared schema, not of a bare name — and cached once. The envelope is a Cadenza value
//! encoded through the one canonical binary codec ([`cadenza_ast::codec`]), the same encoding every value on
//! the wire uses; there is no bespoke format. The user's own payload rides inside the event as opaque
//! [`Bytes`]. Decoding is total: [`Deliver::decode`] returns `None` on any input that is not a well-formed
//! deliver value, so a bad envelope is a rejected deliver, not a panic.

use crate::{
    Bytes, Contract, ContractId, Error, Hash, HostId, Message, Notification, Origin, ReducerId,
    Request, Response, Str,
};
use cadenza_ast::ast::{Builder, Leaf, Struct, StructId};
use cadenza_ast::codec;
use std::sync::{Arc, OnceLock};

/// The single built-in contract the kernel recognizes: a [`Request`](crate::Request) against it is a deliver
/// (§4). It is a real contract whose id is the hash of its declared schema (below); the value is built once
/// and cached, so the id is derived only once.
#[must_use]
pub fn deliver_contract() -> ContractId {
    static DELIVER: OnceLock<Contract> = OnceLock::new();
    DELIVER
        .get_or_init(|| {
            Contract::new(
                Str::from_static("cdz-platform.deliver"),
                deliver_schema,
                "deliver-envelope",
                "deliver-outcome",
            )
        })
        .id()
}

/// The deliver contract's schema: named type declarations for its input (the envelope) and output (the
/// delivery outcome). The bytes-typed leaves are the hashes and opaque payloads the envelope carries.
fn deliver_schema(b: &mut Builder) -> Vec<StructId> {
    // input: (type deliver-envelope (deliver bytes event)) — target reducer-id (bytes) + the event.
    let envelope = {
        let target_ty = b.name("bytes");
        let event_ref = b.name("event");
        let variant = form(b, "deliver", vec![target_ty, event_ref]);
        type_decl(b, "deliver-envelope", vec![variant])
    };

    // (type event (message …) (response …) (notification …)) — an event variant per entry point.
    let event = {
        let msg = {
            let fields = names(b, &["bytes", "bytes", "bytes", "bytes", "bytes"]); // id, reducer, host, payload, token
            form(b, "message", fields)
        };
        let resp = {
            let mut fields = names(b, &["bytes", "bytes"]); // id, token
            let result = b.name("result");
            fields.push(result);
            form(b, "response", fields)
        };
        let notif = {
            let fields = names(b, &["bytes", "bytes"]); // id, payload
            form(b, "notification", fields)
        };
        type_decl(b, "event", vec![msg, resp, notif])
    };

    // (type result (ok bytes) (err error)) and (type error timeout missing-handler)
    let result = {
        let ok = {
            let v = b.name("bytes");
            form(b, "ok", vec![v])
        };
        let err = {
            let e = b.name("error");
            form(b, "err", vec![e])
        };
        type_decl(b, "result", vec![ok, err])
    };
    let error = {
        let variants = names(b, &["timeout", "missing-handler"]);
        type_decl(b, "error", variants)
    };

    // output: (type deliver-outcome delivered (failed bytes)) — delivered, or failed with a reason.
    let outcome = {
        let delivered = b.name("delivered");
        let failed = {
            let reason = b.name("bytes");
            form(b, "failed", vec![reason])
        };
        type_decl(b, "deliver-outcome", vec![delivered, failed])
    };

    vec![envelope, event, result, error, outcome]
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

    /// Build the envelope value into `b`, returning its root: `(deliver <target> <event>)`.
    fn build(&self, b: &mut Builder) -> StructId {
        let target = bytes_leaf(b, self.target.hash().as_bytes());
        let event = match &self.event {
            Delivered::Message(m) => {
                let id = bytes_leaf(b, m.id.hash().as_bytes());
                let reducer = bytes_leaf(b, m.from.reducer.hash().as_bytes());
                let host = bytes_leaf(b, m.from.host.hash().as_bytes());
                let payload = bytes_leaf(b, &m.payload);
                let token = bytes_leaf(b, &m.continuation_token);
                form(b, "message", vec![id, reducer, host, payload, token])
            }
            Delivered::Response(r) => {
                let id = bytes_leaf(b, r.id.hash().as_bytes());
                let token = bytes_leaf(b, &r.continuation_token);
                let result = match &r.payload {
                    Ok(value) => {
                        let v = bytes_leaf(b, value);
                        form(b, "ok", vec![v])
                    }
                    Err(error) => {
                        let name = b.name(error_name(*error));
                        form(b, "err", vec![name])
                    }
                };
                form(b, "response", vec![id, token, result])
            }
            Delivered::Notification(n) => {
                let id = bytes_leaf(b, n.id.hash().as_bytes());
                let payload = bytes_leaf(b, &n.payload);
                form(b, "notification", vec![id, payload])
            }
        };
        form(b, "deliver", vec![target, event])
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
        let [target, event] = as_form(&arenas, arenas.root, "deliver")?;
        let target = ReducerId::from_hash(read_hash(&arenas, target)?);
        let event = decode_event(&arenas, event)?;
        Some(Self { target, event })
    }
}

fn decode_event(arenas: &cadenza_ast::ast::Arenas, id: StructId) -> Option<Delivered> {
    if let Some([id_, reducer, host, payload, token]) = as_form(arenas, id, "message") {
        return Some(Delivered::Message(Message {
            id: ContractId::from_hash(read_hash(arenas, id_)?),
            from: Origin {
                reducer: ReducerId::from_hash(read_hash(arenas, reducer)?),
                host: HostId::from_hash(read_hash(arenas, host)?),
            },
            payload: read_bytes(arenas, payload)?,
            continuation_token: read_bytes(arenas, token)?,
        }));
    }
    if let Some([id_, token, result]) = as_form(arenas, id, "response") {
        return Some(Delivered::Response(Response {
            id: ContractId::from_hash(read_hash(arenas, id_)?),
            continuation_token: read_bytes(arenas, token)?,
            payload: decode_result(arenas, result)?,
        }));
    }
    if let Some([id_, payload]) = as_form(arenas, id, "notification") {
        return Some(Delivered::Notification(Notification {
            id: ContractId::from_hash(read_hash(arenas, id_)?),
            payload: read_bytes(arenas, payload)?,
        }));
    }
    None
}

fn decode_result(arenas: &cadenza_ast::ast::Arenas, id: StructId) -> Option<Result<Bytes, Error>> {
    if let Some([value]) = as_form(arenas, id, "ok") {
        return Some(Ok(read_bytes(arenas, value)?));
    }
    if let Some([name]) = as_form(arenas, id, "err") {
        return Some(Err(match arenas.as_name(name)? {
            "timeout" => Error::Timeout,
            "missing-handler" => Error::MissingHandler,
            _ => return None,
        }));
    }
    None
}

fn error_name(error: Error) -> &'static str {
    match error {
        Error::Timeout => "timeout",
        Error::MissingHandler => "missing-handler",
    }
}

// --- value builder helpers ---

fn form(b: &mut Builder, head: &str, children: Vec<StructId>) -> StructId {
    let head = b.name(head);
    b.list(std::iter::once(head).chain(children).collect())
}

/// A named type declaration `(type <name> <variant>…)`.
fn type_decl(b: &mut Builder, name: &str, variants: Vec<StructId>) -> StructId {
    let head = b.name("type");
    let name = b.name(name);
    b.list(
        std::iter::once(head)
            .chain(std::iter::once(name))
            .chain(variants)
            .collect(),
    )
}

fn names(b: &mut Builder, names: &[&str]) -> Vec<StructId> {
    names.iter().map(|n| b.name(n)).collect()
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
    fn the_deliver_contract_id_is_a_real_contract_id_and_stable() {
        // Stable across calls (cached), and equal to the id a fresh Contract with the same schema derives —
        // i.e. it is the hash of the declared schema, not of a bare name.
        assert_eq!(deliver_contract(), deliver_contract());
        let rebuilt = crate::Contract::new(
            crate::Str::from_static("cdz-platform.deliver"),
            super::deliver_schema,
            "deliver-envelope",
            "deliver-outcome",
        );
        assert_eq!(deliver_contract(), rebuilt.id());
    }
}
