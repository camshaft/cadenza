//! A fast, non-cryptographic hasher for this crate's INTERNAL dedup maps — and the `FxHashMap` alias.
//!
//! The AST builder interns every leaf through a `HashMap<Leaf, LeafId>` dedup map, and `canon`
//! remaps leaf ids through another. `std`'s default `HashMap` uses SipHash — a keyed CRYPTOGRAPHIC
//! hash, the right default for untrusted input but pure overhead here: these maps hold the program's
//! own leaves (identifiers, literals), keyed on short strings and small integers, and `hash_one` +
//! SipHash's `write` were ~27% of front-end parse time. This is the FxHash algorithm (the same one
//! rustc and Firefox use): a multiply-and-rotate over the key's bytes/words. Not DoS-resistant —
//! deliberately, because the keys are the program's own.
//!
//! It is written inline (not pulled from `rustc-hash`) to match the compiler crate's copied-in
//! `fxhash` verbatim — this AST + codec is the copy SOURCE for `rcdzc`, so keeping the hasher a
//! byte-identical local module preserves that round-trip.

use std::hash::{BuildHasherDefault, Hasher};

/// The FxHash mixing constant (the 64-bit golden-ratio-derived odd multiplier rustc-hash uses).
const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

/// A FxHash hasher: fold each written chunk into the state with `((state ROTL 5) XOR chunk) * SEED`.
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

/// A `HashMap` using [`FxHasher`] — for this crate's internal, leaf/id-keyed dedup maps.
pub type FxHashMap<K, V> = std::collections::HashMap<K, V, BuildHasherDefault<FxHasher>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_roundtrips_strings_and_ints() {
        let mut m: FxHashMap<String, u32> = FxHashMap::default();
        m.insert("main".to_string(), 0);
        m.insert("+".to_string(), 1);
        assert_eq!(m.get("main"), Some(&0));
        assert_eq!(m.get("+"), Some(&1));
        assert_eq!(m.get("nope"), None);
        let mut n: FxHashMap<u32, u32> = FxHashMap::default();
        for i in 0..1000u32 {
            n.insert(i, i * 3);
        }
        for i in 0..1000u32 {
            assert_eq!(n.get(&i), Some(&(i * 3)));
        }
    }
}
