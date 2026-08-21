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
//! `from_hash`, and read the underlying hash with `hash`. They render (Display) as the base64url of the
//! hash they carry, tagged in Debug with their role.

use crate::{Hash, HashTag};
use std::fmt;

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
    };
}

hash_id! {
    /// A contract-id: the hash of a contract declaration, which is also the schema hash of the values it
    /// carries (§1/§3). Routes and dispatch key on this; every event carries one as its `id`.
    ContractId, HashTag::Contract
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
    use crate::{Hash, HashTag};

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
}
