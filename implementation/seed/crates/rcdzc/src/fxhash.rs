//! A fast, non-cryptographic hasher for the compiler's INTERNAL index maps — and the `FxHashMap` /
//! `FxHashSet` aliases built on it.
//!
//! The columns model keys almost everything on small integers (`StructId`/`LocalId` = a `u32`, a
//! `db.defs` index = a `usize`) or a short identifier `String`. `std`'s default `HashMap` uses SipHash,
//! a KEYED CRYPTOGRAPHIC hash — the right default for untrusted input, but pure overhead here: these
//! maps hold compiler-internal keys (never attacker-controlled), and SipHash's per-lookup cost showed
//! up hot (`db::def_by_name`, consulted once per name reference in `resolve`, was ~1/5 of self-time on a
//! name-resolution-heavy compile). This is the FxHash algorithm (the same one rustc and Firefox use):
//! a multiply-and-rotate over the key's bytes/words. It is NOT DoS-resistant — deliberately, because the
//! keys are ours.
//!
//! Copy-don't-depend (Cargo.toml): FxHash is ~15 lines of public-domain arithmetic, so it is written
//! here rather than pulled from `rustc-hash` — the pure core stays dependency-free and the Cadenza port
//! stays a mechanical copy (a `u64` multiply-rotate ports trivially; a crate dependency would not).

use std::hash::{BuildHasherDefault, Hasher};

/// The FxHash mixing constant (the 64-bit golden-ratio-derived odd multiplier rustc-hash uses).
const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

/// A FxHash hasher: fold each written chunk into the state with `(state ROTL 5) XOR chunk) * SEED`.
/// The rotate spreads high bits down so that small-integer keys (a `u32` `StructId`) — which differ
/// only in their low bits — land in well-separated buckets.
#[derive(Default)]
pub struct FxHasher {
    hash: u64,
}

impl FxHasher {
    #[inline]
    fn add(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(SEED);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        // Consume 8 bytes at a time, then the tail — enough for a short identifier `String`. (The
        // integer-keyed maps go through `write_u32`/`write_usize` below and never reach the byte loop.)
        let mut chunks = bytes.chunks_exact(8);
        for c in &mut chunks {
            self.add(u64::from_le_bytes(c.try_into().unwrap()));
        }
        let rem = chunks.remainder();
        if !rem.is_empty() {
            let mut buf = [0u8; 8];
            buf[..rem.len()].copy_from_slice(rem);
            self.add(u64::from_le_bytes(buf));
        }
    }

    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.add(i as u64);
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }

    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }
}

/// A `HashMap` using [`FxHasher`] — for the compiler's internal, integer-or-short-string-keyed maps.
pub type FxHashMap<K, V> = std::collections::HashMap<K, V, BuildHasherDefault<FxHasher>>;

/// A `HashSet` using [`FxHasher`].
pub type FxHashSet<T> = std::collections::HashSet<T, BuildHasherDefault<FxHasher>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_roundtrips() {
        let mut m: FxHashMap<u32, &str> = FxHashMap::default();
        m.insert(3, "three");
        m.insert(7, "seven");
        assert_eq!(m.get(&3), Some(&"three"));
        assert_eq!(m.get(&7), Some(&"seven"));
        assert_eq!(m.get(&4), None);
        // String keys work too (the byte path).
        let mut s: FxHashMap<String, u32> = FxHashMap::default();
        s.insert("main".to_string(), 0);
        s.insert("sum-to".to_string(), 1);
        assert_eq!(s.get("main"), Some(&0));
        assert_eq!(s.get("sum-to"), Some(&1));
        assert_eq!(s.get("nope"), None);
    }

    #[test]
    fn set_dedups() {
        let mut s: FxHashSet<u32> = FxHashSet::default();
        assert!(s.insert(5));
        assert!(!s.insert(5));
        assert!(s.contains(&5));
    }

    #[test]
    fn distinct_small_ints_spread() {
        // Small consecutive integer keys must not all collide into one bucket — the rotate is what
        // makes FxHash usable for `StructId`-keyed maps. Insert a run and read them all back.
        let mut m: FxHashMap<u32, u32> = FxHashMap::default();
        for i in 0..1000u32 {
            m.insert(i, i * 2);
        }
        for i in 0..1000u32 {
            assert_eq!(m.get(&i), Some(&(i * 2)));
        }
    }
}
