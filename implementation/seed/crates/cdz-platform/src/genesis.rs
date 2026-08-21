//! A reducer's genesis — what it is spawned from, and the identity that follows from it
//! (`design/cadenza-platform.md` §2/§3).
//!
//! A reducer's id is not chosen; it is *derived* from its **genesis**: the program it runs, a spawn nonce
//! supplied by whatever spawned it, and its parent. Hashing the genesis makes the id both reproducible (a
//! replay that spawns the same thing gets the same id) and lineage-bearing (the parent is in the hash). The
//! nonce is the parent's to choose — a parent spawning several children picks a distinct nonce for each, so
//! their ids differ deterministically.
//!
//! The id is built with the incremental hasher ([`Hash::hasher`]) so no combined buffer is allocated: the
//! two fixed-size fields (program hash, parent id) are fed first and the variable-length nonce last, which
//! keeps the concatenation unambiguous without separators.

use crate::{Bytes, Hash, HashTag, ProgramHash, ReducerId};

/// What a reducer is spawned from: the program it runs, the spawn `nonce` its parent chose, and its parent.
/// Its [`id`](Genesis::id) is the hash of these — the reducer's identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Genesis {
    /// The program the reducer runs (by content hash).
    pub program: ProgramHash,
    /// The spawn nonce, chosen by the parent so sibling spawns get distinct, reproducible ids.
    pub nonce: Bytes,
    /// The reducer that spawned this one. (The root reducer is its own parent.)
    pub parent: ReducerId,
}

impl Genesis {
    /// The reducer's id — the hash of its genesis. Reproducible from the genesis alone, and it carries the
    /// parent (so lineage is in the identity). Fixed-size fields first, variable-length nonce last, so the
    /// hashed concatenation is unambiguous.
    #[must_use]
    pub fn id(&self) -> ReducerId {
        let mut hasher = Hash::hasher(HashTag::Reducer);
        hasher
            .update(self.program.hash().as_bytes())
            .update(self.parent.hash().as_bytes())
            .update(&self.nonce);
        ReducerId::from_hash(hasher.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::Genesis;
    use crate::{Bytes, ProgramHash, ReducerId};

    fn prog(tag: &[u8]) -> ProgramHash {
        ProgramHash::of(tag)
    }
    fn rid(tag: &[u8]) -> ReducerId {
        ReducerId::of(tag)
    }

    fn genesis(program: &[u8], nonce: &'static [u8], parent: &[u8]) -> Genesis {
        Genesis {
            program: prog(program),
            nonce: Bytes::from_static(nonce),
            parent: rid(parent),
        }
    }

    #[test]
    fn id_is_reproducible_from_the_genesis() {
        // Same genesis → same id (a pure function of program + nonce + parent).
        assert_eq!(
            genesis(b"agent", b"1", b"root").id(),
            genesis(b"agent", b"1", b"root").id()
        );
    }

    #[test]
    fn each_field_distinguishes_the_id() {
        let base = genesis(b"agent", b"1", b"root").id();
        // A different program, nonce, or parent each yields a different id.
        assert_ne!(base, genesis(b"other", b"1", b"root").id());
        assert_ne!(base, genesis(b"agent", b"2", b"root").id());
        assert_ne!(base, genesis(b"agent", b"1", b"parent").id());
    }

    #[test]
    fn the_nonce_gives_a_parent_distinct_children() {
        // A parent spawning the same program twice picks distinct nonces to get distinct, reproducible ids.
        let a = genesis(b"child", b"0", b"parent").id();
        let b = genesis(b"child", b"1", b"parent").id();
        assert_ne!(a, b);
    }
}
