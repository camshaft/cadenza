//! The deliver primitive (`design/cadenza-platform.md` §4).
//!
//! Delivering an event into a reducer's log is the one privileged thing an event reducer does that an
//! ordinary reducer cannot: it is the routing act — handing a message to the next handler, folding a
//! response back to a caller. An event reducer emits it as an ordinary [`Request`] against the well-known
//! [`deliver_contract`] (the single built-in contract the kernel recognizes), whose payload is a [`Deliver`]
//! envelope naming the target reducer and the event to inject.
//!
//! The envelope is kernel control metadata, not a user value, so it uses a small self-contained binary
//! encoding rather than the language's value codec — the user's own payload rides inside the event as
//! opaque [`Bytes`]. The encoding is length-prefixed and total: [`Deliver::decode`] returns `None` on any
//! malformed input rather than panicking, so a bad envelope is a rejected deliver, not a crash.

use crate::{
    Bytes, ContractId, Error, Hash, HostId, Message, Notification, Origin, ReducerId, Request,
    Response,
};

/// The single built-in contract the kernel recognizes: a [`Request`](crate::Request) against it is a deliver
/// (§4). Its id is the hash of a fixed name, so every node agrees on it.
#[must_use]
pub fn deliver_contract() -> ContractId {
    ContractId::from_hash(Hash::of(b"cdz-platform:deliver"))
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

// Event-kind tags in the encoding.
const KIND_MESSAGE: u8 = 0;
const KIND_RESPONSE: u8 = 1;
const KIND_NOTIFICATION: u8 = 2;
// Result tags for a Response payload.
const RESULT_OK: u8 = 0;
const RESULT_ERR: u8 = 1;
// Error tags.
const ERR_TIMEOUT: u8 = 0;
const ERR_MISSING_HANDLER: u8 = 1;

impl Deliver {
    /// Encode the envelope to bytes: the target hash, a kind tag, then the event's fields, hashes fixed at
    /// 32 bytes and byte strings length-prefixed. The inverse of [`decode`](Self::decode).
    #[must_use]
    pub fn encode(&self) -> Bytes {
        let mut out = Vec::new();
        push_hash(&mut out, self.target.hash());
        match &self.event {
            Delivered::Message(m) => {
                out.push(KIND_MESSAGE);
                push_hash(&mut out, m.id.hash());
                push_hash(&mut out, m.from.reducer.hash());
                push_hash(&mut out, m.from.host.hash());
                push_bytes(&mut out, &m.payload);
                push_bytes(&mut out, &m.continuation_token);
            }
            Delivered::Response(r) => {
                out.push(KIND_RESPONSE);
                push_hash(&mut out, r.id.hash());
                push_bytes(&mut out, &r.continuation_token);
                match &r.payload {
                    Ok(value) => {
                        out.push(RESULT_OK);
                        push_bytes(&mut out, value);
                    }
                    Err(error) => {
                        out.push(RESULT_ERR);
                        out.push(match error {
                            Error::Timeout => ERR_TIMEOUT,
                            Error::MissingHandler => ERR_MISSING_HANDLER,
                        });
                    }
                }
            }
            Delivered::Notification(n) => {
                out.push(KIND_NOTIFICATION);
                push_hash(&mut out, n.id.hash());
                push_bytes(&mut out, &n.payload);
            }
        }
        Bytes::from(out)
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

    /// Decode an envelope from bytes, or `None` if the input is malformed (too short, an unknown tag, or
    /// trailing bytes). Total, so a bad envelope is a rejected deliver, never a panic.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let mut cursor = Cursor::new(bytes);
        let target = ReducerId::from_hash(cursor.read_hash()?);
        let event = match cursor.read_u8()? {
            KIND_MESSAGE => Delivered::Message(Message {
                id: ContractId::from_hash(cursor.read_hash()?),
                from: Origin {
                    reducer: ReducerId::from_hash(cursor.read_hash()?),
                    host: HostId::from_hash(cursor.read_hash()?),
                },
                payload: cursor.read_bytes()?,
                continuation_token: cursor.read_bytes()?,
            }),
            KIND_RESPONSE => {
                let id = ContractId::from_hash(cursor.read_hash()?);
                let continuation_token = cursor.read_bytes()?;
                let payload = match cursor.read_u8()? {
                    RESULT_OK => Ok(cursor.read_bytes()?),
                    RESULT_ERR => Err(match cursor.read_u8()? {
                        ERR_TIMEOUT => Error::Timeout,
                        ERR_MISSING_HANDLER => Error::MissingHandler,
                        _ => return None,
                    }),
                    _ => return None,
                };
                Delivered::Response(Response {
                    id,
                    continuation_token,
                    payload,
                })
            }
            KIND_NOTIFICATION => Delivered::Notification(Notification {
                id: ContractId::from_hash(cursor.read_hash()?),
                payload: cursor.read_bytes()?,
            }),
            _ => return None,
        };
        // No trailing bytes: a well-formed envelope consumes its whole input.
        cursor.finished().then_some(Self { target, event })
    }
}

fn push_hash(out: &mut Vec<u8>, hash: Hash) {
    out.extend_from_slice(hash.as_bytes());
}

fn push_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    // A byte string is a u32 little-endian length followed by the bytes.
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
}

/// A reading cursor over the encoded bytes; every read is bounds-checked and yields `None` past the end.
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let slice = self.bytes.get(self.pos..self.pos.checked_add(n)?)?;
        self.pos += n;
        Some(slice)
    }

    fn read_u8(&mut self) -> Option<u8> {
        self.take(1).map(|s| s[0])
    }

    fn read_hash(&mut self) -> Option<Hash> {
        let slice = self.take(Hash::LEN)?;
        Some(Hash::from_bytes(
            slice.try_into().expect("took Hash::LEN bytes"),
        ))
    }

    fn read_bytes(&mut self) -> Option<Bytes> {
        let len = u32::from_le_bytes(self.take(4)?.try_into().expect("took 4 bytes")) as usize;
        Some(Bytes::copy_from_slice(self.take(len)?))
    }

    fn finished(&self) -> bool {
        self.pos == self.bytes.len()
    }
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
    fn a_message_deliver_round_trips() {
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
    fn decode_rejects_malformed_input() {
        // Truncated (nothing at all, and a lone target with no kind).
        assert_eq!(Deliver::decode(&[]), None);
        assert_eq!(Deliver::decode(rid(b"t").hash().as_bytes()), None);
        // Trailing garbage after a valid envelope is rejected.
        let good = Deliver {
            target: rid(b"child"),
            event: Delivered::Notification(Notification {
                id: cid(b"n"),
                payload: Bytes::from_static(b"x"),
            }),
        }
        .encode();
        let mut with_trailer = good.to_vec();
        with_trailer.push(0xFF);
        assert_eq!(Deliver::decode(&with_trailer), None);
        // An unknown kind tag.
        let mut bad_kind = rid(b"t").hash().as_bytes().to_vec();
        bad_kind.push(9);
        assert_eq!(Deliver::decode(&bad_kind), None);
    }

    #[test]
    fn the_deliver_contract_id_is_stable() {
        assert_eq!(deliver_contract(), deliver_contract());
    }
}
