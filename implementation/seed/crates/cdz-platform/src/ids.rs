//! Typed hash identifiers (`design/cadenza-platform.md` §1/§3).
//!
//! Everything in the platform is named by a [`Hash`], but a bare hash says nothing about *what* it names —
//! a contract, a reducer, a program, a host — so passing one where another is meant is a mistake the
//! compiler cannot see. These newtypes wrap a `Hash` with the role it plays, so a contract-id can never be
//! handed to something expecting a reducer-id, and the documentation's word is enforced by the type system
//! instead of trusted. A bare `Hash` remains for raw content addressing (the blob store, [`Hash::of`]);
//! everything else is a hash *of something*, and gets a name here.
//!
//! Each is a transparent, `Copy` wrapper: build one from content with `of` (which stamps the matching
//! [`HashTag`] into the hash, so the role is self-describing at runtime too), or wrap an existing hash with
//! `from_hash`, and read the underlying hash with `hash`. They render (Display) as the base62 of the
//! hash they carry, tagged in Debug with their role.

use crate::{ContractKind, Hash, HashTag};
use std::fmt;

/// Deserialize a [`Hash`] from a byte string — the on-wire form of every typed id (a `Bytes` leaf of the
/// hash's 33 raw bytes, the same shape [`Hash::as_bytes`] serializes). Shared by the id types'
/// `Deserialize` impls so a `#[derive(Deserialize)]` struct/enum with an id field decodes the id from the
/// canonical value-form via `cadenza-ast-serde` (or any serde format).
fn deserialize_hash<'de, D>(deserializer: D) -> Result<Hash, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{Error, Visitor};
    struct HashVisitor;
    impl<'de> Visitor<'de> for HashVisitor {
        type Value = Hash;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a hash as a byte string")
        }
        fn visit_bytes<E: Error>(self, v: &[u8]) -> Result<Hash, E> {
            Hash::try_from(v).map_err(E::custom)
        }
        fn visit_borrowed_bytes<E: Error>(self, v: &'de [u8]) -> Result<Hash, E> {
            Hash::try_from(v).map_err(E::custom)
        }
        fn visit_byte_buf<E: Error>(self, v: Vec<u8>) -> Result<Hash, E> {
            Hash::try_from(v.as_slice()).map_err(E::custom)
        }
    }
    deserializer.deserialize_byte_buf(HashVisitor)
}

macro_rules! hash_id {
    ($(#[$doc:meta])* $name:ident, $tag:expr) => {
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(Hash);

        impl $name {
            #[doc = concat!("The [`HashTag`](crate::HashTag) that marks a hash as a `", stringify!($name), "`.")]
            pub const TAG: HashTag = $tag;

            #[doc = concat!("The `", stringify!($name), "` of `bytes` — their content hash, tagged `", stringify!($name), "`.")]
            #[must_use]
            pub fn of(bytes: &[u8]) -> Self {
                Self(Hash::of(Self::TAG, bytes))
            }

            #[doc = concat!("Wrap a raw `Hash` as a `", stringify!($name), "`.")]
            #[must_use]
            pub const fn from_hash(hash: Hash) -> Self {
                Self(hash)
            }

            /// The underlying content hash.
            #[must_use]
            pub const fn hash(self) -> Hash {
                self.0
            }
        }

        #[doc = concat!("Read a `", stringify!($name), "` back from the raw bytes of the hash it carries — how it arrives when it crosses a boundary that carries an id as a byte slice (a WIT payload, a stored key). Fails (a wrong-length slice names no hash) with the same error as [`Hash::try_from`]; the tag is not required to match this role's [`TAG`](Self::TAG), since a hash is bytes.")]
        impl TryFrom<&[u8]> for $name {
            type Error = std::array::TryFromSliceError;

            fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
                Ok(Self::from_hash(Hash::try_from(bytes)?))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }

        // serde: a typed id (de)serializes AS its hash's 33 raw bytes — the same `Bytes` value-form the
        // hand-written codecs use (bytes_leaf(id.hash().as_bytes()) / read_hash). So a
        // `#[derive(Serialize, Deserialize)]` struct/enum with an id field round-trips through the
        // canonical binary-AST via `cadenza-ast-serde` with no change to the id's bytes.
        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_bytes(self.0.as_bytes())
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                deserialize_hash(deserializer).map(Self::from_hash)
            }
        }
    };
}

hash_id! {
    /// A contract-id: the hash of a contract declaration, which is also the schema hash of the values it
    /// carries (§1/§3). Routes and dispatch key on this; every event carries one as its `id`.
    ///
    /// A contract-id also carries its **mutation-vs-query class** (654(a)) in its leading tag byte — see
    /// [`kind`](Self::kind) / [`of_kind`](Self::of_kind). The bare [`of`](Self::of) / [`TAG`](Self::TAG) name
    /// the [`Mutation`](ContractKind::Mutation) class ([`HashTag::Contract`]), the default that every
    /// pre-existing contract-id already carries.
    ContractId, HashTag::Contract
}

impl ContractId {
    /// The contract-id of `bytes` for a given [`ContractKind`] (654(a)): the content hash tagged with the
    /// class's [`route_tag`](ContractKind::route_tag), so the mutation-vs-query class rides in the id's
    /// leading byte and the router reads it straight off the id. [`of`](Self::of) is exactly
    /// `of_kind(bytes, ContractKind::Mutation)` — the default class — so a mutation contract-id is
    /// byte-identical to the pre-654(a) id and nothing drifts; a query contract-id has the same digest but
    /// the [`ContractQuery`](HashTag::ContractQuery) tag.
    #[must_use]
    pub fn of_kind(bytes: &[u8], kind: ContractKind) -> Self {
        Self::from_hash(Hash::of(kind.route_tag(), bytes))
    }

    /// The mutation-vs-query class this contract-id routes as (654(a)), read straight out of its leading tag
    /// byte — the router's decision needs no side table. `None` if the id's tag is not a contract tag (a
    /// raw/foreign hash wrapped as a `ContractId` via [`from_hash`](Self::from_hash) names no class), so a
    /// caller can tell a genuine contract-id's class from an untagged wrapper.
    #[must_use]
    pub fn kind(self) -> Option<ContractKind> {
        self.hash().tag().and_then(ContractKind::from_tag)
    }
}

hash_id! {
    /// A reducer/session instance id: the hash of the reducer's genesis (§3). Names a live participant — a
    /// handler in a chain, a node in the spawn hierarchy, the source of a message, the target of a deliver.
    ReducerId, HashTag::Reducer
}

hash_id! {
    /// A program hash: the content hash of a program (a wasm component) a reducer is spawned *from* (§3/§8).
    /// The event registry maps a contract to the program the kernel spawns its event reducer from.
    ProgramHash, HashTag::Program
}

hash_id! {
    /// A host id: the identity of the host (node/runtime) a reducer runs on (§3/§11). Travels in an
    /// [`Origin`](crate::Origin) alongside the reducer, the hook for federated trust.
    HostId, HashTag::Host
}

#[cfg(test)]
mod tests {
    use super::{ContractId, ProgramHash, ReducerId};
    use crate::{ContractKind, Hash, HashTag};

    #[test]
    fn of_defaults_to_the_mutation_class_and_does_not_drift() {
        // 654(a): the bare `of` mints a MUTATION contract-id — byte-identical to `of_kind(_, Mutation)` and
        // to the pre-654(a) `Hash::of(HashTag::Contract, …)`, so no existing contract-id shifts.
        let bytes = b"a-contract-declaration";
        let m = ContractId::of(bytes);
        assert_eq!(m, ContractId::of_kind(bytes, ContractKind::Mutation));
        assert_eq!(m.hash(), Hash::of(HashTag::Contract, bytes));
        assert_eq!(m.hash().tag(), Some(HashTag::Contract));
    }

    #[test]
    fn the_class_rides_in_the_tag_byte_and_reads_back_from_the_id_alone() {
        // A mutation and a query over the SAME declaration bytes share a digest but differ only in the
        // leading tag byte — so the router reads the class straight off the id, no side table.
        let bytes = b"temp.celsius";
        let mutation = ContractId::of_kind(bytes, ContractKind::Mutation);
        let query = ContractId::of_kind(bytes, ContractKind::Query);
        assert_eq!(mutation.kind(), Some(ContractKind::Mutation));
        assert_eq!(query.kind(), Some(ContractKind::Query));
        assert_eq!(query.hash().tag(), Some(HashTag::ContractQuery));
        // Same digest (the class is the tag, not hashed content here), distinct ids (the tag is identity).
        assert_eq!(mutation.hash().digest(), query.hash().digest());
        assert_ne!(mutation, query);
    }

    #[test]
    fn a_non_contract_tagged_wrapper_names_no_class() {
        // Wrapping a raw/foreign hash as a ContractId via `from_hash` names no mutation-vs-query class — its
        // tag is not a contract tag — so `kind()` is None rather than misreporting a class.
        let reducerish = ContractId::from_hash(Hash::of(HashTag::Reducer, b"x"));
        assert_eq!(reducerish.kind(), None);
        let mut raw = *Hash::of(HashTag::Contract, b"y").as_bytes();
        raw[0] = 0xFF; // an unknown tag
        assert_eq!(ContractId::from_hash(Hash::from_bytes(raw)).kind(), None);
    }

    #[test]
    fn wraps_and_unwraps_a_hash() {
        let h = Hash::of(HashTag::Contract, b"x");
        assert_eq!(ContractId::from_hash(h).hash(), h);
    }

    #[test]
    fn of_stamps_the_matching_tag_into_the_hash() {
        // `of` mints a typed id whose underlying hash carries the newtype's tag — the runtime counterpart
        // of the compile-time newtype.
        assert_eq!(ContractId::of(b"c").hash().tag(), Some(HashTag::Contract));
        assert_eq!(ReducerId::of(b"r").hash().tag(), Some(HashTag::Reducer));
        assert_eq!(ProgramHash::of(b"p").hash().tag(), Some(HashTag::Program));
        assert_eq!(ContractId::TAG, HashTag::Contract);
        // Same bytes under different id types produce different hashes (the tag is part of the identity),
        // so a contract-id and a reducer-id of "x" never collide even though the digest matches.
        assert_ne!(ContractId::of(b"x").hash(), ReducerId::of(b"x").hash());
        assert_eq!(
            ContractId::of(b"x").hash().digest(),
            ReducerId::of(b"x").hash().digest()
        );
    }

    #[test]
    fn equal_hashes_give_equal_ids_and_they_are_usable_as_keys() {
        use std::collections::HashSet;
        let a = ReducerId::of(b"r");
        let b = ReducerId::of(b"r");
        assert_eq!(a, b);
        let mut set = HashSet::new();
        set.insert(a);
        assert!(set.contains(&b), "same-hash ids collide as keys");
    }

    #[test]
    fn display_is_the_hash_and_debug_is_tagged() {
        let id = ContractId::of(b"temp.celsius");
        assert_eq!(id.to_string(), id.hash().to_string());
        assert!(format!("{id:?}").starts_with("ContractId("));
    }

    #[test]
    fn try_from_slice_reads_an_id_back_from_its_bytes_and_rejects_wrong_length() {
        // The bytes an id crosses a boundary as (a WIT payload, a stored key) reconstruct it exactly.
        let id = ReducerId::of(b"a-reducer");
        assert_eq!(
            ReducerId::try_from(id.hash().as_bytes().as_slice()).unwrap(),
            id
        );
        // A wrong-length slice names no id.
        assert!(ReducerId::try_from(b"short".as_slice()).is_err());
        // The tag is not required to match the role — a hash is bytes, so a `ContractId`'s bytes read back
        // as a `ReducerId` (the caller vouches for what the bytes name; the length is all that is checked).
        let from_contract = ContractId::of(b"x").hash();
        assert_eq!(
            ReducerId::try_from(from_contract.as_bytes().as_slice())
                .unwrap()
                .hash(),
            from_contract
        );
    }
}
