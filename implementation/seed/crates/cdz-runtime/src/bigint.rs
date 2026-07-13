//! Arbitrary-precision signed integers — a small, hand-written `no_std` limb library for the runtime
//! (DESIGN-bigint-and-rational-rcdzc.md §5). Pure over `alloc::vec::Vec`, no I/O, no dependency: the
//! runtime's wasm bytes are content-hashed (`REQUIRED_RUNTIME_HASH`), so pulling `num-bigint` (+
//! `num-integer`/`num-traits`) into the frozen runtime would be a large, hard-to-audit hash-changing
//! dependency. The surface is small (add/sub/mul/divmod/cmp + from/to i64 + two byte encodings) over
//! `Vec<u32>` limbs — schoolbook algorithms (the magnitudes real programs hit are small; correctness >
//! asymptotics for the seed). Independently unit-testable natively, with a differential test against
//! `num-bigint` (a dev-dependency) as the safety net — the analogue of the CHAMP-vs-BTreeMap oracle.
//!
//! # Representation
//! `Big { neg: bool, mag: Vec<u32> }` — base-2³² limbs, LITTLE-ENDIAN (`mag[0]` is the least-significant
//! limb), with NO trailing zero limbs. Zero is the canonical `{ neg: false, mag: [] }`. Every operation
//! `normalize`s its result (strips trailing zero limbs; forces `neg = false` when the magnitude is zero),
//! so a value has exactly ONE in-memory form — required because the heap-leaf byte form (below) is what
//! `champ_hash`/`champ_eq`/`value-eq` compare, so a `BigInt` used as a map key / compared with `=` must be
//! canonical.

use alloc::vec::Vec;
use core::cmp::Ordering;

/// An arbitrary-precision signed integer. See the module doc for the canonical-form invariant.
#[derive(Clone, PartialEq, Eq, Debug)]
#[allow(dead_code)] // wired to runtime ops in a later increment; the module is DCE'd from the wasm until then
pub struct Big {
    /// Sign: `true` = negative. Always `false` when `mag` is empty (zero is non-negative, canonical).
    pub neg: bool,
    /// Magnitude limbs, base 2³², little-endian, no trailing zero limbs (empty = zero).
    pub mag: Vec<u32>,
}

#[allow(dead_code)]
impl Big {
    /// The canonical zero.
    pub fn zero() -> Big {
        Big { neg: false, mag: Vec::new() }
    }

    /// Whether this is zero (canonical: empty magnitude).
    pub fn is_zero(&self) -> bool {
        self.mag.is_empty()
    }

    /// Strip trailing zero limbs and force a zero magnitude to non-negative — re-establishes the
    /// canonical form after an operation that may have produced trailing zeros or a signed zero.
    fn normalize(&mut self) {
        while self.mag.last() == Some(&0) {
            self.mag.pop();
        }
        if self.mag.is_empty() {
            self.neg = false;
        }
    }

    // ─── magnitude helpers (unsigned, operate on limb slices) ─────────────────────────────────

    /// Compare two magnitudes (limb slices, little-endian, normalized) by value.
    fn cmp_mag(a: &[u32], b: &[u32]) -> Ordering {
        if a.len() != b.len() {
            return a.len().cmp(&b.len());
        }
        // Equal limb count → compare from the most-significant limb down.
        for i in (0..a.len()).rev() {
            match a[i].cmp(&b[i]) {
                Ordering::Equal => {}
                ord => return ord,
            }
        }
        Ordering::Equal
    }

    /// `a + b` over magnitudes (little-endian limbs), returning a normalized magnitude.
    fn add_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
        let mut out = Vec::with_capacity(a.len().max(b.len()) + 1);
        let mut carry = 0u64;
        for i in 0..a.len().max(b.len()) {
            let av = *a.get(i).unwrap_or(&0) as u64;
            let bv = *b.get(i).unwrap_or(&0) as u64;
            let s = av + bv + carry;
            out.push((s & 0xffff_ffff) as u32);
            carry = s >> 32;
        }
        if carry != 0 {
            out.push(carry as u32);
        }
        strip(&mut out);
        out
    }

    /// `a - b` over magnitudes, REQUIRING `a >= b` (caller ensures via `cmp_mag`). Returns normalized.
    fn sub_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
        let mut out = Vec::with_capacity(a.len());
        let mut borrow = 0i64;
        for i in 0..a.len() {
            let av = a[i] as i64;
            let bv = *b.get(i).unwrap_or(&0) as i64;
            let mut d = av - bv - borrow;
            if d < 0 {
                d += 1i64 << 32;
                borrow = 1;
            } else {
                borrow = 0;
            }
            out.push(d as u32);
        }
        strip(&mut out);
        out
    }

    /// `a * b` over magnitudes (O(n·m) schoolbook), returning a normalized magnitude.
    fn mul_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
        if a.is_empty() || b.is_empty() {
            return Vec::new();
        }
        let mut out = alloc::vec![0u32; a.len() + b.len()];
        for (i, &av) in a.iter().enumerate() {
            let mut carry = 0u64;
            for (j, &bv) in b.iter().enumerate() {
                let cur = out[i + j] as u64 + (av as u64) * (bv as u64) + carry;
                out[i + j] = (cur & 0xffff_ffff) as u32;
                carry = cur >> 32;
            }
            // Propagate the final carry into the next limb (and beyond, if it cascades).
            let mut k = i + b.len();
            while carry != 0 {
                let cur = out[k] as u64 + carry;
                out[k] = (cur & 0xffff_ffff) as u32;
                carry = cur >> 32;
                k += 1;
            }
        }
        strip(&mut out);
        out
    }

    // ─── signed arithmetic ────────────────────────────────────────────────────────────────────

    /// Signed comparison. Consistent with the value order (`-1 < 0 < 1`).
    pub fn cmp(&self, other: &Big) -> Ordering {
        match (self.neg, other.neg) {
            (false, true) => Ordering::Greater,
            (true, false) => Ordering::Less,
            (false, false) => Big::cmp_mag(&self.mag, &other.mag),
            // Both negative: the LARGER magnitude is the SMALLER value.
            (true, true) => Big::cmp_mag(&other.mag, &self.mag),
        }
    }

    /// `self + other`.
    pub fn add(&self, other: &Big) -> Big {
        let mut r = if self.neg == other.neg {
            // Same sign: add magnitudes, keep the sign.
            Big { neg: self.neg, mag: Big::add_mag(&self.mag, &other.mag) }
        } else {
            // Opposite signs: subtract the smaller magnitude from the larger; sign follows the larger.
            match Big::cmp_mag(&self.mag, &other.mag) {
                Ordering::Equal => Big::zero(),
                Ordering::Greater => Big { neg: self.neg, mag: Big::sub_mag(&self.mag, &other.mag) },
                Ordering::Less => Big { neg: other.neg, mag: Big::sub_mag(&other.mag, &self.mag) },
            }
        };
        r.normalize();
        r
    }

    /// `-self`.
    pub fn neg(&self) -> Big {
        if self.is_zero() {
            return Big::zero();
        }
        Big { neg: !self.neg, mag: self.mag.clone() }
    }

    /// `self - other`.
    pub fn sub(&self, other: &Big) -> Big {
        self.add(&other.neg())
    }

    /// `self * other`.
    pub fn mul(&self, other: &Big) -> Big {
        let mag = Big::mul_mag(&self.mag, &other.mag);
        let mut r = Big { neg: self.neg != other.neg, mag };
        r.normalize();
        r
    }

    /// Truncating division + remainder: returns `(quotient, remainder)` where
    /// `self = quotient * divisor + remainder`, the quotient truncates toward zero, and the remainder
    /// has the sign of `self` (Rust `/`/`%` semantics). `None` when `divisor` is zero.
    pub fn divmod(&self, divisor: &Big) -> Option<(Big, Big)> {
        if divisor.is_zero() {
            return None;
        }
        let (qmag, rmag) = divmod_mag(&self.mag, &divisor.mag);
        // Quotient sign = XOR of operand signs; remainder sign = dividend sign (truncated division).
        let mut q = Big { neg: self.neg != divisor.neg, mag: qmag };
        let mut r = Big { neg: self.neg, mag: rmag };
        q.normalize();
        r.normalize();
        Some((q, r))
    }

    /// The greatest common divisor of `|self|` and `|other|` — always NON-NEGATIVE (gcd is sign-agnostic:
    /// `gcd(a, b) = gcd(|a|, |b|)`). `gcd(0, 0) = 0`; `gcd(a, 0) = |a|`. Euclid over magnitudes via
    /// `divmod_mag` (the remainder shrinks each step). Needed by `Rational` normalization (DESIGN §7).
    pub fn gcd(&self, other: &Big) -> Big {
        let mut a = self.mag.clone(); // |self|
        let mut b = other.mag.clone(); // |other|
        while !b.is_empty() {
            let (_q, r) = divmod_mag(&a, &b); // r = a mod b, normalized (no trailing zeros)
            a = b;
            b = r;
        }
        // `a` is the gcd magnitude (empty iff both inputs were zero). Non-negative by construction.
        let mut g = Big { neg: false, mag: a };
        g.normalize();
        g
    }

    // ─── conversions ──────────────────────────────────────────────────────────────────────────

    /// Box a signed 64-bit int as a `Big`.
    pub fn from_i64(v: i64) -> Big {
        if v == 0 {
            return Big::zero();
        }
        let neg = v < 0;
        let m = v.unsigned_abs(); // handles i64::MIN without overflow
        let mut mag = Vec::new();
        mag.push((m & 0xffff_ffff) as u32);
        let hi = (m >> 32) as u32;
        if hi != 0 {
            mag.push(hi);
        }
        Big { neg, mag }
    }

    /// Narrow to `i64` if it fits, else `None` (the checked narrowing — the caller traps on `None`).
    pub fn to_i64_checked(&self) -> Option<i64> {
        if self.mag.len() > 2 {
            return None;
        }
        let lo = *self.mag.first().unwrap_or(&0) as u64;
        let hi = *self.mag.get(1).unwrap_or(&0) as u64;
        let m = lo | (hi << 32); // magnitude as u64
        if self.neg {
            // Negative: fits iff m <= 2^63 (that boundary is exactly i64::MIN).
            if m <= (i64::MAX as u64) + 1 {
                // `-(m as i128) as i64` is exact for m up to 2^63.
                Some((m as i128 * -1) as i64)
            } else {
                None
            }
        } else if m <= i64::MAX as u64 {
            Some(m as i64)
        } else {
            None
        }
    }

    // ─── sign-magnitude bytes (the HEAP-LEAF form) ──────────────────────────────────────────────
    // byte 0: sign (0 = non-negative, 1 = negative). bytes 1..: magnitude as LITTLE-ENDIAN bytes with
    // no trailing zero bytes. Zero is exactly `[0x00]` (sign 0, empty magnitude). Canonical.

    /// Serialize to the sign-magnitude heap-leaf bytes.
    pub fn to_sign_magnitude_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.push(self.neg as u8);
        // Limbs (LE u32) → LE bytes, then strip trailing zero bytes for canonicality.
        for &limb in &self.mag {
            out.extend_from_slice(&limb.to_le_bytes());
        }
        while out.len() > 1 && *out.last().unwrap() == 0 {
            out.pop();
        }
        out
    }

    /// Parse from the sign-magnitude heap-leaf bytes (the inverse of `to_sign_magnitude_bytes`). A
    /// malformed/empty input decodes as zero (total — the compiler only ever bakes a well-formed leaf).
    pub fn from_sign_magnitude_bytes(bytes: &[u8]) -> Big {
        let Some((&sign, mag_bytes)) = bytes.split_first() else {
            return Big::zero();
        };
        let mut mag = Vec::with_capacity(mag_bytes.len().div_ceil(4));
        let mut i = 0;
        while i < mag_bytes.len() {
            let mut limb = [0u8; 4];
            let k = (mag_bytes.len() - i).min(4);
            limb[..k].copy_from_slice(&mag_bytes[i..i + k]);
            mag.push(u32::from_le_bytes(limb));
            i += 4;
        }
        let mut b = Big { neg: sign != 0, mag };
        b.normalize();
        b
    }

    // ─── two's-complement bytes (the BOUNDARY form: `list<u8>`, little-endian) ───────────────────

    /// Serialize to little-endian two's-complement bytes — the pinned boundary encoding. The MINIMAL
    /// length that round-trips: the sign bit of the top byte must equal the value's sign, so a positive
    /// value whose top byte is ≥0x80 gets a `0x00` guard byte, and a negative one whose top byte is
    /// <0x80 gets a `0xff` guard byte. Zero is the empty slice.
    pub fn to_le_twos_complement_bytes(&self) -> Vec<u8> {
        if self.is_zero() {
            return Vec::new();
        }
        // Magnitude → LE bytes (strip trailing zeros).
        let mut mbytes = Vec::new();
        for &limb in &self.mag {
            mbytes.extend_from_slice(&limb.to_le_bytes());
        }
        while mbytes.last() == Some(&0) {
            mbytes.pop();
        }
        if !self.neg {
            // Non-negative: bytes are the magnitude; add a 0x00 guard if the top bit is set.
            if mbytes.last().map(|&b| b & 0x80 != 0).unwrap_or(false) {
                mbytes.push(0x00);
            }
            mbytes
        } else {
            // Negative: two's complement = invert all bytes + 1, over enough bytes that the sign bit is set.
            // Ensure a byte exists whose top bit will be 1 after negation: if the magnitude's top bit is
            // already set we still need a leading 0x00 in the magnitude so negation yields 0xff… — handled
            // by widening one byte, then trimming redundant 0xff at the end.
            let mut ext = mbytes;
            if ext.last().map(|&b| b & 0x80 != 0).unwrap_or(true) {
                ext.push(0x00);
            }
            // two's complement: invert + add 1
            let mut carry = 1u16;
            for byte in ext.iter_mut() {
                let v = (!*byte) as u16 + carry;
                *byte = (v & 0xff) as u8;
                carry = v >> 8;
            }
            // Trim redundant 0xff top bytes (keep one that preserves the sign bit).
            while ext.len() > 1
                && *ext.last().unwrap() == 0xff
                && ext[ext.len() - 2] & 0x80 != 0
            {
                ext.pop();
            }
            ext
        }
    }

    /// Parse little-endian two's-complement bytes (the inverse of `to_le_twos_complement_bytes`). Empty
    /// = zero. The sign is the top bit of the most-significant (last) byte.
    pub fn from_le_twos_complement_bytes(bytes: &[u8]) -> Big {
        let Some(&top) = bytes.last() else {
            return Big::zero();
        };
        let neg = top & 0x80 != 0;
        // Magnitude bytes: if negative, take the two's complement (invert + 1) to recover the magnitude.
        let mut mbytes: Vec<u8> = bytes.to_vec();
        if neg {
            let mut carry = 1u16;
            for byte in mbytes.iter_mut() {
                let v = (!*byte) as u16 + carry;
                *byte = (v & 0xff) as u8;
                carry = v >> 8;
            }
        }
        // LE bytes → LE u32 limbs.
        let mut mag = Vec::with_capacity(mbytes.len().div_ceil(4));
        let mut i = 0;
        while i < mbytes.len() {
            let mut limb = [0u8; 4];
            let k = (mbytes.len() - i).min(4);
            limb[..k].copy_from_slice(&mbytes[i..i + k]);
            mag.push(u32::from_le_bytes(limb));
            i += 4;
        }
        let mut b = Big { neg, mag };
        b.normalize();
        b
    }
}

/// Strip trailing zero limbs from a magnitude (little-endian).
fn strip(v: &mut Vec<u32>) {
    while v.last() == Some(&0) {
        v.pop();
    }
}

/// Unsigned long division of magnitudes: `(quotient, remainder)` with `a = quotient * b + remainder`,
/// `0 <= remainder < b`. `b` MUST be non-empty (nonzero — the caller checks). Bit-at-a-time long
/// division (simple + obviously-correct; the magnitudes real programs hit are small, so the O(bits ·
/// limbs) cost is fine — correctness-first per the design). Both results normalized.
fn divmod_mag(a: &[u32], b: &[u32]) -> (Vec<u32>, Vec<u32>) {
    // a < b → quotient 0, remainder a.
    if Big::cmp_mag(a, b) == Ordering::Less {
        return (Vec::new(), a.to_vec());
    }
    let nbits = a.len() * 32;
    let mut q = alloc::vec![0u32; a.len()];
    let mut r: Vec<u32> = Vec::new(); // running remainder, normalized (no trailing zeros)
    // Process dividend bits from most-significant to least.
    for i in (0..nbits).rev() {
        // r <<= 1
        shl1(&mut r);
        // bring down bit i of a into r's bit 0
        let bit = (a[i / 32] >> (i % 32)) & 1;
        if bit != 0 {
            if r.is_empty() {
                r.push(1);
            } else {
                r[0] |= 1;
            }
        }
        // if r >= b { r -= b; set quotient bit i }
        if Big::cmp_mag(&r, b) != Ordering::Less {
            r = Big::sub_mag(&r, b);
            q[i / 32] |= 1 << (i % 32);
        }
    }
    strip(&mut q);
    strip(&mut r);
    (q, r)
}

/// `r <<= 1` over a little-endian limb magnitude (normalized in/out).
fn shl1(r: &mut Vec<u32>) {
    let mut carry = 0u32;
    for limb in r.iter_mut() {
        let hi = *limb >> 31;
        *limb = (*limb << 1) | carry;
        carry = hi;
    }
    if carry != 0 {
        r.push(carry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use num_bigint::BigInt as Ref;
    use num_traits::{Signed, Zero};

    // Convert a native `Big` to the reference `num_bigint::BigInt` for differential comparison.
    fn to_ref(b: &Big) -> Ref {
        // Build from sign + LE u32 limbs.
        let mut bytes = Vec::new();
        for &limb in &b.mag {
            bytes.extend_from_slice(&limb.to_le_bytes());
        }
        let mag = num_bigint::BigUint::from_bytes_le(&bytes);
        let sign = if b.is_zero() {
            num_bigint::Sign::NoSign
        } else if b.neg {
            num_bigint::Sign::Minus
        } else {
            num_bigint::Sign::Plus
        };
        Ref::from_biguint(sign, mag)
    }

    fn from_i128(v: i128) -> Big {
        // Build a Big from an i128 for test seeding (covers > i64 range).
        if v == 0 {
            return Big::zero();
        }
        let neg = v < 0;
        let mut m = v.unsigned_abs();
        let mut mag = Vec::new();
        while m != 0 {
            mag.push((m & 0xffff_ffff) as u32);
            m >>= 32;
        }
        let mut b = Big { neg, mag };
        b.normalize();
        b
    }

    // A small deterministic PRNG (no rand dep; Math.random is banned + we want reproducibility).
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            // xorshift64
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
        fn big(&mut self) -> Big {
            self.big_upto(5) // 0..=4 limbs
        }
        /// A random `Big` with 0..`max_limbs` limbs (wider magnitudes exercise the divmod limb-boundary
        /// carry/borrow that ≤4-limb operands never reach).
        fn big_upto(&mut self, max_limbs: u64) -> Big {
            let limbs = (self.next() % max_limbs) as usize;
            let mut mag = Vec::new();
            for _ in 0..limbs {
                mag.push(self.next() as u32);
            }
            let neg = self.next() & 1 == 1;
            let mut b = Big { neg, mag };
            b.normalize();
            b
        }
    }

    #[test]
    fn differential_arithmetic_vs_num_bigint() {
        let mut rng = Rng(0x1234_5678_9abc_def1);
        for _ in 0..5000 {
            let a = rng.big();
            let b = rng.big();
            let (ra, rb) = (to_ref(&a), to_ref(&b));

            assert_eq!(to_ref(&a.add(&b)), &ra + &rb, "add {a:?} {b:?}");
            assert_eq!(to_ref(&a.sub(&b)), &ra - &rb, "sub {a:?} {b:?}");
            assert_eq!(to_ref(&a.mul(&b)), &ra * &rb, "mul {a:?} {b:?}");
            assert_eq!(to_ref(&a.neg()), -&ra, "neg {a:?}");

            let ord = a.cmp(&b);
            assert_eq!(ord, ra.cmp(&rb), "cmp {a:?} {b:?}");

            if !b.is_zero() {
                let (q, r) = a.divmod(&b).unwrap();
                // num-bigint's / and % are truncating (toward zero), matching our divmod.
                assert_eq!(to_ref(&q), &ra / &rb, "div {a:?} {b:?}");
                assert_eq!(to_ref(&r), &ra % &rb, "rem {a:?} {b:?}");
                // The defining identity: a == q*b + r.
                assert_eq!(a, q.mul(&b).add(&r), "divmod identity {a:?} {b:?}");
            } else {
                assert!(a.divmod(&b).is_none(), "div by zero → None");
            }

            // gcd: compare against a reference Euclid over num-bigint absolutes (no extra dep). gcd is
            // sign-agnostic and non-negative; gcd(0,0)=0.
            let g = a.gcd(&b);
            assert!(!g.neg, "gcd is non-negative {a:?} {b:?}");
            assert_eq!(to_ref(&g), ref_gcd(ra.abs(), rb.abs()), "gcd {a:?} {b:?}");
            // The GCD divides both operands exactly (when nonzero) and is a common divisor.
            if !g.is_zero() {
                assert!(a.divmod(&g).unwrap().1.is_zero(), "gcd divides a exactly");
                assert!(b.divmod(&g).unwrap().1.is_zero(), "gcd divides b exactly");
            } else {
                assert!(a.is_zero() && b.is_zero(), "gcd is 0 only when both operands are 0");
            }
        }
    }

    /// Reference gcd via Euclid over non-negative num-bigint values (avoids a `num-integer` dep).
    fn ref_gcd(mut a: Ref, mut b: Ref) -> Ref {
        while !b.is_zero() {
            let r = &a % &b;
            a = b;
            b = r;
        }
        a
    }

    #[test]
    fn canonical_form_invariants() {
        // Zero is unique and non-negative.
        assert!(Big::zero().is_zero());
        assert_eq!(Big::zero(), Big { neg: false, mag: Vec::new() });
        // A "-0" or trailing-zero-limb input normalizes to canonical zero / minimal form.
        let mut z = Big { neg: true, mag: alloc::vec![0, 0] };
        z.normalize();
        assert_eq!(z, Big::zero());
        let mut t = Big { neg: false, mag: alloc::vec![5, 0, 0] };
        t.normalize();
        assert_eq!(t.mag, alloc::vec![5]);
        // Subtraction that reaches zero canonicalizes the sign.
        let five = Big::from_i64(5);
        assert_eq!(five.sub(&five), Big::zero());
        assert!(!five.sub(&five).neg);
    }

    #[test]
    fn i64_round_trip_and_bounds() {
        for &v in &[0i64, 1, -1, 42, -42, i64::MAX, i64::MIN, 1 << 40, -(1 << 40), 0xffff_ffff, -0xffff_ffff] {
            let b = Big::from_i64(v);
            assert_eq!(b.to_i64_checked(), Some(v), "i64 round-trip {v}");
            assert_eq!(to_ref(&b), Ref::from(v), "i64 vs ref {v}");
        }
        // Out-of-range narrowing → None.
        let too_big = Big::from_i64(i64::MAX).add(&Big::from_i64(1)); // 2^63
        assert_eq!(too_big.to_i64_checked(), None, "2^63 does not fit i64");
        let way_big = from_i128((i64::MAX as i128) * 1000);
        assert_eq!(way_big.to_i64_checked(), None);
        // i64::MIN (= -2^63) DOES fit.
        assert_eq!(Big::from_i64(i64::MIN).to_i64_checked(), Some(i64::MIN));
    }

    #[test]
    fn sign_magnitude_bytes_round_trip_and_canonical() {
        let mut rng = Rng(0xdead_beef_cafe_0001);
        for _ in 0..2000 {
            let b = rng.big();
            let bytes = b.to_sign_magnitude_bytes();
            assert_eq!(Big::from_sign_magnitude_bytes(&bytes), b, "sign-mag round-trip {b:?}");
            // Canonical: equal values → identical bytes (the champ-key requirement).
            assert_eq!(bytes, b.clone().to_sign_magnitude_bytes());
        }
        // Zero is exactly [0x00].
        assert_eq!(Big::zero().to_sign_magnitude_bytes(), alloc::vec![0u8]);
        assert_eq!(Big::from_sign_magnitude_bytes(&[0]), Big::zero());
    }

    #[test]
    fn twos_complement_bytes_round_trip_vs_num_bigint() {
        let mut rng = Rng(0x0badf00d_12345678);
        for _ in 0..3000 {
            let b = rng.big();
            let bytes = b.to_le_twos_complement_bytes();
            // Round-trips through our own parser.
            assert_eq!(Big::from_le_twos_complement_bytes(&bytes), b, "2c round-trip {b:?}");
            // Matches num-bigint's signed LE two's-complement encoding.
            let rbytes = to_ref(&b).to_signed_bytes_le();
            // num-bigint encodes 0 as [0]; we encode 0 as [] — normalize both to "value" via re-parse.
            assert_eq!(
                Big::from_le_twos_complement_bytes(&rbytes),
                b,
                "num-bigint 2c bytes {rbytes:?} parse to {b:?}"
            );
        }
    }

    /// divmod is the algorithm with real subtlety (bit-at-a-time long division; a limb-boundary carry/
    /// borrow bug hides only on LARGE operands the ≤4-limb random fuzzer never reaches). Two prongs:
    /// (1) a WIDE differential vs num-bigint (up to ~20 limbs = ~640-bit); (2) structural corner cases —
    /// powers of two (all-carry shifts), a single-limb divisor of a huge dividend, dividend just below /
    /// at / above the divisor, all-`0xffffffff` limbs, and an EXACT multiple (`(k*d)/d == k`, rem 0).
    #[test]
    fn divmod_edge_cases_and_wide_operands() {
        // (1) Wide differential — magnitudes up to ~20 limbs, both signs.
        let mut rng = Rng(0xf00d_1234_5678_9abc);
        for _ in 0..3000 {
            let a = rng.big_upto(20);
            let b = rng.big_upto(20);
            let (ra, rb) = (to_ref(&a), to_ref(&b));
            if b.is_zero() {
                assert!(a.divmod(&b).is_none());
                continue;
            }
            let (q, r) = a.divmod(&b).unwrap();
            assert_eq!(to_ref(&q), &ra / &rb, "wide div {a:?} {b:?}");
            assert_eq!(to_ref(&r), &ra % &rb, "wide rem {a:?} {b:?}");
            assert_eq!(a, q.mul(&b).add(&r), "wide divmod identity");
            // |remainder| < |divisor| (the division invariant).
            assert_eq!(
                Big { neg: false, mag: r.mag.clone() }.cmp(&Big { neg: false, mag: b.mag.clone() }),
                core::cmp::Ordering::Less,
                "|rem| < |divisor| {a:?} {b:?}"
            );
        }

        // (2) Structural corners.
        let pow2 = |bits: u32| -> Big {
            // 2^bits as a Big (a single set bit — exercises the shift/carry path).
            let limb = (bits / 32) as usize;
            let mut mag = alloc::vec![0u32; limb + 1];
            mag[limb] = 1 << (bits % 32);
            let mut b = Big { neg: false, mag };
            b.normalize();
            b
        };
        // 2^200 / 2^64 = 2^136, remainder 0.
        let (q, r) = pow2(200).divmod(&pow2(64)).unwrap();
        assert_eq!(q, pow2(136), "2^200 / 2^64 = 2^136");
        assert!(r.is_zero(), "2^200 % 2^64 = 0");
        // (2^200 - 1) / 2^64 → quotient 2^136 - 1, remainder 2^64 - 1 (all low bits set).
        let big = pow2(200).sub(&Big::from_i64(1));
        let (q2, r2) = big.divmod(&pow2(64)).unwrap();
        assert_eq!(to_ref(&q2), to_ref(&big) / to_ref(&pow2(64)), "(2^200-1)/2^64 vs ref");
        assert_eq!(to_ref(&r2), to_ref(&big) % to_ref(&pow2(64)), "(2^200-1)%2^64 vs ref");

        // Single-limb divisor of a huge dividend (the common `n / small` shape).
        let huge = pow2(300).add(&Big::from_i64(12345));
        let small = Big::from_i64(7);
        let (qs, rs) = huge.divmod(&small).unwrap();
        assert_eq!(to_ref(&qs), to_ref(&huge) / to_ref(&small));
        assert_eq!(to_ref(&rs), to_ref(&huge) % to_ref(&small));

        // Dividend just-below / at / just-above the divisor.
        let d = pow2(128);
        let below = d.sub(&Big::from_i64(1));
        assert_eq!(below.divmod(&d).unwrap(), (Big::zero(), below.clone()), "a<d → (0, a)");
        assert_eq!(d.divmod(&d).unwrap(), (Big::from_i64(1), Big::zero()), "a==d → (1, 0)");
        let above = d.add(&Big::from_i64(1));
        assert_eq!(above.divmod(&d).unwrap(), (Big::from_i64(1), Big::from_i64(1)), "a=d+1 → (1, 1)");

        // All-0xffffffff limbs (max limb values — carry propagation stress).
        let maxes = Big { neg: false, mag: alloc::vec![0xffff_ffff; 8] };
        let mref = to_ref(&maxes);
        for div in [Big::from_i64(3), pow2(32), pow2(100), maxes.clone()] {
            let (q, r) = maxes.divmod(&div).unwrap();
            assert_eq!(to_ref(&q), &mref / to_ref(&div), "maxes / {div:?}");
            assert_eq!(to_ref(&r), &mref % to_ref(&div), "maxes % {div:?}");
        }

        // Exact multiple: (k*d)/d == k, rem 0 — for random NON-NEGATIVE k, d (sign cleared so the
        // truncating quotient's sign can't confuse the `q == k` check).
        let abs = |mut b: Big| {
            b.neg = false;
            b
        };
        for _ in 0..500 {
            let k = abs(rng.big_upto(8));
            let dd = abs(rng.big_upto(8));
            if dd.is_zero() {
                continue;
            }
            let prod = k.mul(&dd);
            let (q, r) = prod.divmod(&dd).unwrap();
            assert_eq!(q, k, "(k*d)/d == k");
            assert!(r.is_zero(), "(k*d)%d == 0");
        }
    }
}
