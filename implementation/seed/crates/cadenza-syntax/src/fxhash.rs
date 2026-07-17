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

    fn h(f: impl FnOnce(&mut FxHasher)) -> u64 {
        let mut hasher = FxHasher::default();
        f(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn known_answer_vectors_pin_the_exact_fxhash_algorithm() {
        // This hasher must stay BYTE-IDENTICAL to rcdzc's copied-in `fxhash` (this crate's AST + codec
        // is the copy SOURCE), so the round-trip depends on the EXACT algorithm, not merely "some hash
        // that works". `map_roundtrips_*` passes for ANY functioning hasher and so guards nothing here.
        // These are independently hand-computed known-answer vectors — a regression to `SEED`, the
        // `rotate_left(5)` amount, the little-endian word packing, or the `write` remainder padding
        // changes at least one of them (each constant was derived from a separate reference impl, NOT
        // from the code under test).

        // Fresh state finishes at 0; hashing a zero word keeps it 0 (rotl(0,5) ^ 0 == 0, * SEED == 0).
        assert_eq!(h(|_| {}), 0, "empty state");
        assert_eq!(h(|s| s.write_u64(0)), 0, "u64(0) stays 0");

        // A single `add(1)` from zero state is exactly `(rotl(0,5) ^ 1) * SEED == SEED` — pins the
        // multiplier constant directly.
        assert_eq!(
            h(|s| s.write_u64(1)),
            0x517c_c1b7_2722_0a95,
            "u64(1) == SEED"
        );
        // write_u32 / write_usize also route through the same single `add`, so all three agree on 1.
        assert_eq!(
            h(|s| s.write_u32(1)),
            0x517c_c1b7_2722_0a95,
            "u32(1) == SEED"
        );
        assert_eq!(
            h(|s| s.write_usize(1)),
            0x517c_c1b7_2722_0a95,
            "usize(1) == SEED"
        );

        // Order sensitivity pins `rotate_left(5)`: (1 then 0) folds a rotated nonzero state, so it is
        // NOT 0 and NOT the same as (0 then 1). A missing/incorrect rotate would collapse these.
        assert_eq!(
            h(|s| {
                s.write_u64(1);
                s.write_u64(0);
            }),
            0x0d45_69ee_47d3_c0f2,
            "u64(1),u64(0) is rotate-sensitive"
        );
        assert_eq!(
            h(|s| {
                s.write_u64(1);
                s.write_u64(1);
            }),
            0x5ec2_2ba5_6ef5_cb87,
            "u64(1),u64(1)"
        );
        assert_ne!(
            h(|s| {
                s.write_u64(1);
                s.write_u64(0);
            }),
            h(|s| {
                s.write_u64(0);
                s.write_u64(1);
            }),
            "order matters (rotate_left)"
        );

        // The `write(&[u8])` path: a single byte 0x01 must zero-pad the 8-byte remainder buffer
        // little-endian, giving the SAME state as `write_u64(1)`. This pins the padding + endianness.
        assert_eq!(
            h(|s| s.write(&[1u8])),
            0x517c_c1b7_2722_0a95,
            "one byte pads to u64(1)"
        );
        assert_eq!(
            h(|s| s.write(&1u64.to_le_bytes())),
            0x517c_c1b7_2722_0a95,
            "8 LE bytes of 1 == u64(1)"
        );

        // A 4-byte string (pure remainder path, no full chunk) and a 9-byte slice (one exact 8-byte
        // chunk + a 1-byte remainder) exercise both branches of `write`.
        assert_eq!(
            h(|s| s.write(b"main")),
            0x1e74_8e51_ec9d_f671,
            "\"main\" remainder path"
        );
        assert_eq!(
            h(|s| s.write(&[1, 2, 3, 4, 5, 6, 7, 8, 9])),
            0xcd94_3b11_5336_6ac4,
            "9 bytes = chunk + remainder"
        );
    }
}
