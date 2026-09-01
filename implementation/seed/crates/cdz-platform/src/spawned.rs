//! The birth notification (`design/cadenza-platform.md` §7).
//!
//! When the system spawns a reducer, it delivers a [`Spawned`] event as the reducer's very first event —
//! ahead of any message — so the reducer knows who it is before it folds anything: its own id and the id of
//! the reducer that spawned it. It arrives as a control-plane [`Notification`] on the [`spawned_contract`],
//! folded through `on_notification` like any other; a reducer that does not care simply ignores it.
//!
//! Like every value on the wire, the event is a Cadenza value in the one canonical binary codec, its schema
//! generated from `contracts/spawned.cdz` ([`crate::contracts::spawned`]). Decoding is total:
//! [`Spawned::decode`] returns `None` on any input that is not a well-formed spawned value.

use crate::{Bytes, Contract, ContractId, Notification, ReducerId};
use cadenza_ast::ast::{Builder, StructId};
use cadenza_ast::codec;
use std::sync::OnceLock;

/// The contract of the birth notification (§7): a [`Notification`] whose `id` is this contract's id carries a
/// [`Spawned`] event as its payload. A real contract whose id is the hash of its declared schema — the
/// compiler-checked `spawned` module generated from `contracts/spawned.cdz` — built once and cached, so the
/// id is derived only once.
#[must_use]
pub fn spawned_contract() -> ContractId {
    static SPAWNED: OnceLock<Contract> = OnceLock::new();
    SPAWNED
        .get_or_init(crate::contracts::spawned::contract)
        .id()
}

/// A reducer's birth event, delivered to it as its first event (§7): its own id and its parent's. For a
/// root, `parent` equals `id` — a root is its own parent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spawned {
    /// The reducer's own id.
    pub id: ReducerId,
    /// The reducer that spawned it — its own id again for a root.
    pub parent: ReducerId,
}

impl Spawned {
    /// Encode the event as a Cadenza value in the canonical binary form ([`cadenza_ast::codec`]) — the same
    /// encoding every value on the wire uses. The inverse of [`decode`](Self::decode).
    #[must_use]
    pub fn encode(&self) -> Bytes {
        crate::contract_value::encode_ascribed(|b| self.build(b), "Event")
    }

    /// Build the event value into `b`, returning its root — a value of the schema type `Event`, so it
    /// type-ascribes against the contract's schema. The value shape is entirely the generated builders'
    /// (`contracts::spawned::*`, generated from the same source as the schema, so they cannot drift); this
    /// only supplies the `Bytes` leaves.
    fn build(&self, b: &mut Builder) -> StructId {
        use crate::contract_value as v;
        use crate::contracts::spawned as c;
        let id = v::bytes_leaf(b, self.id.hash().as_bytes());
        let parent = v::bytes_leaf(b, self.parent.hash().as_bytes());
        c::event_spawned(b, c::EventSpawned { id, parent })
    }

    /// The control-plane [`Notification`] that carries this event: on the [`spawned_contract`], with the
    /// event as its payload. This is what the system delivers as the reducer's first event.
    #[must_use]
    pub fn into_notification(self) -> Notification {
        Notification {
            id: spawned_contract(),
            payload: self.encode(),
        }
    }

    /// Decode an event from a Cadenza value, or `None` if the bytes are not a well-formed spawned value.
    /// Total, so a malformed value is a rejected event, never a panic.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        use crate::contract_value as v;
        use crate::contracts::spawned as c;
        let arenas = codec::decode(bytes)?;
        let root = v::as_ascribed(&arenas, arenas.root)?;
        let e = c::as_event_spawned(&arenas, root)?;
        Some(Self {
            id: ReducerId::from_hash(v::read_hash(&arenas, e.id)?),
            parent: ReducerId::from_hash(v::read_hash(&arenas, e.parent)?),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Spawned, spawned_contract};
    use crate::ReducerId;

    fn rid(tag: &[u8]) -> ReducerId {
        ReducerId::of(tag)
    }

    #[test]
    fn a_spawned_event_round_trips_through_the_codec() {
        let event = Spawned {
            id: rid(b"child"),
            parent: rid(b"parent"),
        };
        assert_eq!(Spawned::decode(&event.encode()), Some(event));
    }

    #[test]
    fn a_single_constructor_record_arm_elides_its_constructor() {
        // FIX B invariant, record elision arm: `Event` is a SINGLE-constructor sum (`| Spawned(Record …)`),
        // so the canonical form the compiler's `Value.decode` reads ELIDES the constructor — the payload
        // record rides directly under the root ascription, with NO `(Spawned …)` wrapper. Builder and reader
        // elide symmetrically, so the round-trip test cannot catch a regression that re-introduces the
        // wrapper; a guest `Value.decode` would then fail. Pin the PHYSICAL shape: under the ascription is the
        // `record` itself (its `id` field reads directly), not a `Spawned` constructor list.
        use crate::contract_value as v;
        let event = Spawned {
            id: rid(b"child"),
            parent: rid(b"parent"),
        };
        let arenas = cadenza_ast::codec::decode(&event.encode()).expect("well-formed value");
        let inner = v::as_ascribed(&arenas, arenas.root).expect("root ascription");
        // The elided value IS the record: its fields read directly by name.
        assert!(
            v::record_field(&arenas, inner, "id").is_some(),
            "the elided single-ctor value is the record itself"
        );
        // And it is NOT wrapped in the `Spawned` constructor (elided).
        assert!(v::as_qctor(&arenas, inner, "Event", "Spawned").is_none());
    }

    #[test]
    fn a_root_carries_itself_as_its_parent() {
        let root = rid(b"root");
        let event = Spawned {
            id: root,
            parent: root,
        };
        let decoded = Spawned::decode(&event.encode()).expect("well-formed");
        assert_eq!(decoded.id, decoded.parent);
    }

    #[test]
    fn the_notification_carries_the_event_on_the_spawned_contract() {
        let event = Spawned {
            id: rid(b"child"),
            parent: rid(b"parent"),
        };
        let n = event.into_notification();
        assert_eq!(n.id, spawned_contract());
        assert_eq!(Spawned::decode(&n.payload), Some(event));
    }

    #[test]
    fn decode_rejects_bytes_that_are_not_a_spawned_value() {
        assert_eq!(Spawned::decode(&[0xFF, 0x00, 0x13, 0x37]), None);
        assert_eq!(Spawned::decode(&[]), None);
        let mut b = cadenza_ast::ast::Builder::new();
        let root = b.name("not-a-spawned-event");
        let wrong_shape = cadenza_ast::codec::encode(&b.finish(root));
        assert_eq!(Spawned::decode(&wrong_shape), None);
    }

    #[test]
    fn the_spawned_contract_id_is_a_real_contract_id_and_stable() {
        assert_eq!(spawned_contract(), spawned_contract());
        let rebuilt = crate::Contract::new(
            crate::Str::from_static("cdz-platform.spawned"),
            crate::contracts::spawned::schema,
            "Event",
            "Ack",
        );
        assert_eq!(spawned_contract(), rebuilt.id());
    }
}
