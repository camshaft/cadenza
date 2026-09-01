//! The lifecycle notification (`design/cadenza-platform.md` §7).
//!
//! A supervision link is a one-way subscription: it asks the system to deliver a watched reducer's
//! lifecycle events into the watcher's mailbox. When a watched reducer terminates, the system delivers a
//! [`Lifecycle`] event to each of its watchers as a control-plane [`Notification`], on the
//! [`lifecycle_contract`]. The watcher folds it through `on_notification` and decides for itself what a
//! peer's exit means for it — the system enacts no reaction of its own, so a parent that wants to tear down
//! when a child crashes does so by returning its own `Break`, which in turn reaches *its* watchers.
//!
//! Because the two directions of a link are independent, a lifecycle event must name its subject: a reducer
//! watching several peers folds every peer's exit through the one `on_notification`, and reads `reducer` to
//! know which peer ended. Like every value on the wire, the event is a Cadenza value in the one canonical
//! binary codec, its schema generated from `contracts/lifecycle.cdz`
//! ([`crate::contracts::lifecycle`]). Decoding is total: [`Lifecycle::decode`] returns `None` on any input
//! that is not a well-formed lifecycle value, so a bad payload is a rejected event, not a panic.

use crate::{Bytes, Contract, ContractId, Notification, ReducerId};
use cadenza_ast::ast::{Builder, StructId};
use cadenza_ast::codec;
use std::sync::OnceLock;

/// The contract of the lifecycle notification (§7): a [`Notification`] whose `id` is this contract's id
/// carries a [`Lifecycle`] event as its payload. A real contract whose id is the hash of its declared
/// schema — the compiler-checked `lifecycle` module generated from `contracts/lifecycle.cdz` — built once
/// and cached, so the id is derived only once.
#[must_use]
pub fn lifecycle_contract() -> ContractId {
    static LIFECYCLE: OnceLock<Contract> = OnceLock::new();
    LIFECYCLE
        .get_or_init(crate::contracts::lifecycle::contract)
        .id()
}

/// A terminated reducer's lifecycle event, delivered to each of its watchers (§7). It names the reducer
/// that ended and says how it ended, so a watcher subscribed to several peers can tell them apart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lifecycle {
    /// The reducer closed itself with a typed reason — its [`Break`](crate::Outcome::Break). `schema` is
    /// the reason value's schema hash and `reason` its canonical bytes, the same typed reason the reducer
    /// carried, so a supervisor decodes why it ended.
    Exited {
        /// The reducer that ended.
        reducer: ReducerId,
        /// The schema hash of the exit reason value.
        schema: ContractId,
        /// The exit reason value in its canonical encoding.
        reason: Bytes,
    },
    /// The reducer's task ended without a typed reason — a panic that unwound past its fold, or its mailbox
    /// closing — so only the reducer that ended is reported; there is no reason to carry.
    Crashed {
        /// The reducer that ended.
        reducer: ReducerId,
    },
}

impl Lifecycle {
    /// Encode the event as a Cadenza value in the canonical binary form ([`cadenza_ast::codec`]) — the same
    /// encoding every value on the wire uses. The inverse of [`decode`](Self::decode).
    #[must_use]
    pub fn encode(&self) -> Bytes {
        crate::contract_value::encode_ascribed(|b| self.build(b), "Event")
    }

    /// Build the event value into `b`, returning its root — a value of the schema type `Event`, so it
    /// type-ascribes against the contract's schema. The value shape is entirely the generated builders'
    /// (`contracts::lifecycle::*`, generated from the same source as the schema, so they cannot drift);
    /// this only supplies the `Bytes` leaves.
    fn build(&self, b: &mut Builder) -> StructId {
        use crate::contract_value as v;
        use crate::contracts::lifecycle as c;
        match self {
            Lifecycle::Exited {
                reducer,
                schema,
                reason,
            } => {
                let reducer = v::bytes_leaf(b, reducer.hash().as_bytes());
                let schema = v::bytes_leaf(b, schema.hash().as_bytes());
                let reason = v::bytes_leaf(b, reason);
                c::event_exited(
                    b,
                    c::EventExited {
                        reducer,
                        schema,
                        reason,
                    },
                )
            }
            Lifecycle::Crashed { reducer } => {
                let reducer = v::bytes_leaf(b, reducer.hash().as_bytes());
                c::event_crashed(b, c::EventCrashed { reducer })
            }
        }
    }

    /// The control-plane [`Notification`] that carries this event: on the [`lifecycle_contract`], with the
    /// event as its payload. This is what the system delivers into a watcher's mailbox.
    #[must_use]
    pub fn into_notification(self) -> Notification {
        Notification {
            id: lifecycle_contract(),
            payload: self.encode(),
        }
    }

    /// Decode an event from a Cadenza value, or `None` if the bytes are not a well-formed lifecycle value.
    /// Total, so a malformed value is a rejected event, never a panic.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        use crate::contract_value as v;
        use crate::contracts::lifecycle as c;
        let arenas = codec::decode(bytes)?;
        let root = v::as_ascribed(&arenas, arenas.root)?;
        if let Some(e) = c::as_event_exited(&arenas, root) {
            return Some(Lifecycle::Exited {
                reducer: ReducerId::from_hash(v::read_hash(&arenas, e.reducer)?),
                schema: ContractId::from_hash(v::read_hash(&arenas, e.schema)?),
                reason: v::read_bytes(&arenas, e.reason)?,
            });
        }
        if let Some(e) = c::as_event_crashed(&arenas, root) {
            return Some(Lifecycle::Crashed {
                reducer: ReducerId::from_hash(v::read_hash(&arenas, e.reducer)?),
            });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{Lifecycle, lifecycle_contract};
    use crate::{Bytes, ContractId, ReducerId};

    fn cid(tag: &[u8]) -> ContractId {
        ContractId::of(tag)
    }
    fn rid(tag: &[u8]) -> ReducerId {
        ReducerId::of(tag)
    }

    fn round_trips(e: &Lifecycle) {
        assert_eq!(Lifecycle::decode(&e.encode()), Some(e.clone()));
    }

    #[test]
    fn an_exited_event_round_trips_through_the_codec() {
        round_trips(&Lifecycle::Exited {
            reducer: rid(b"child"),
            schema: cid(b"finished"),
            reason: Bytes::from_static(b"done"),
        });
    }

    #[test]
    fn a_crashed_event_round_trips_through_the_codec() {
        round_trips(&Lifecycle::Crashed {
            reducer: rid(b"child"),
        });
    }

    #[test]
    fn the_notification_carries_the_event_on_the_lifecycle_contract() {
        let event = Lifecycle::Crashed {
            reducer: rid(b"child"),
        };
        let n = event.clone().into_notification();
        assert_eq!(n.id, lifecycle_contract());
        assert_eq!(Lifecycle::decode(&n.payload), Some(event));
    }

    #[test]
    fn decode_rejects_bytes_that_are_not_a_lifecycle_value() {
        // Not a valid encoding at all.
        assert_eq!(Lifecycle::decode(&[0xFF, 0x00, 0x13, 0x37]), None);
        assert_eq!(Lifecycle::decode(&[]), None);
        // A well-formed Cadenza value of the wrong shape decodes cleanly but is not a lifecycle event.
        let mut b = cadenza_ast::ast::Builder::new();
        let root = b.name("not-a-lifecycle-event");
        let wrong_shape = cadenza_ast::codec::encode(&b.finish(root));
        assert_eq!(Lifecycle::decode(&wrong_shape), None);
    }

    #[test]
    fn the_lifecycle_contract_id_is_a_real_contract_id_and_stable() {
        // Stable across calls (cached), and equal to the id a fresh Contract with the same schema derives —
        // i.e. it is the hash of the declared schema, not of a bare name.
        assert_eq!(lifecycle_contract(), lifecycle_contract());
        let rebuilt = crate::Contract::new(
            crate::Str::from_static("cdz-platform.lifecycle"),
            crate::contracts::lifecycle::schema,
            "Event",
            "Ack",
        );
        assert_eq!(lifecycle_contract(), rebuilt.id());
    }
}
