//! Typed hash identifiers (`design/cadenza-platform.md` §1/§3).
//!
//! Everything in the platform is named by a [`Hash`], but a bare hash says nothing about *what* it names —
//! a contract, a reducer, a program, a host — so passing one where another is meant is a mistake the
//! compiler cannot see. These newtypes wrap a `Hash` with the role it plays, so a contract-id can never be
//! handed to something expecting a reducer-id, and the documentation's word is enforced by the type system
//! instead of trusted. A bare `Hash` remains for raw content addressing (the blob store, [`Hash::of`]);
//! everything else is a hash *of something*, and gets a name here.
//!
//! Each is a transparent, `Copy` wrapper: build one with `from_hash` and read the underlying hash with
//! `hash`. They render (Display) as the base64url of the hash they carry, tagged in Debug with their role.

use crate::Hash;
use std::fmt;

macro_rules! hash_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(Hash);

        impl $name {
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
    ContractId
}

hash_id! {
    /// A reducer/session instance id: the hash of the reducer's genesis (§3). Names a live participant — a
    /// handler in a chain, a node in the spawn hierarchy, the source of a message, the target of a deliver.
    ReducerId
}

hash_id! {
    /// A program hash: the content hash of a program (a wasm component) a reducer is spawned *from* (§3/§8).
    /// The event registry maps a contract to the program the kernel spawns its event reducer from.
    ProgramHash
}

hash_id! {
    /// A host id: the identity of the host (node/runtime) a reducer runs on (§3/§11). Travels in an
    /// [`Origin`](crate::Origin) alongside the reducer, the hook for federated trust.
    HostId
}

#[cfg(test)]
mod tests {
    use super::{ContractId, ReducerId};
    use crate::Hash;

    #[test]
    fn wraps_and_unwraps_a_hash() {
        let h = Hash::of(b"x");
        assert_eq!(ContractId::from_hash(h).hash(), h);
    }

    #[test]
    fn equal_hashes_give_equal_ids_and_they_are_usable_as_keys() {
        use std::collections::HashSet;
        let a = ReducerId::from_hash(Hash::of(b"r"));
        let b = ReducerId::from_hash(Hash::of(b"r"));
        assert_eq!(a, b);
        let mut set = HashSet::new();
        set.insert(a);
        assert!(set.contains(&b), "same-hash ids collide as keys");
    }

    #[test]
    fn display_is_the_hash_and_debug_is_tagged() {
        let id = ContractId::from_hash(Hash::of(b"temp.celsius"));
        assert_eq!(id.to_string(), id.hash().to_string());
        assert!(format!("{id:?}").starts_with("ContractId("));
    }
}
