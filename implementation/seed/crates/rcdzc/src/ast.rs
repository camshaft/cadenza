//! The AST: two flat arenas — the interface between syntax and compiler.
//!
//! The tree is NOT nested and NOT one arena. It splits **leaf values** from **structure**:
//!
//! - The **leaf pool** holds the distinct primitive values, DEDUPLICATED. A name or literal used
//!   500 times is one entry. Leaves carry no source spans. `LeafId` indexes it.
//! - The **structure arena** holds one entry per SYNTACTIC OCCURRENCE, NOT deduplicated. An entry
//!   is an `Atom(LeafId)` or a `List` of child `StructId`s. `StructId` indexes it; `root` is the
//!   top occurrence.
//!
//! Why the split: it dissolves the occurrence/span problem. A shared node in a nested tree would
//! have many source positions (its span depends on the path taken to reach it). Here the only
//! deduplicated things are leaves, and leaves have no spans; every syntactic occurrence is its own
//! `StructId`, so a span table is a trivial total map `StructId -> range` (see `spans.rs`).
//!
//! A construct is a `List` whose first child is an `Atom` of a `Name` — e.g. `(if c t e)`. There
//! is no dedicated variant per construct: keywords are data, so a new construct is a new head
//! *name*, never a change to this frozen shape. This is what keeps the AST stable and macro
//! pre-expansion (rewriting uniform `(head child…)` structure) easy.

use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;
// `String`/`ToString`/`format!` from `alloc` (not std's prelude) so this file compiles under the
// `#![no_std]` `cdz-runtime` that `include!`s it as well as under std `rcdzc`; `alloc::string::String`
// == `std::string::String`.
use alloc::format;
use alloc::string::{String, ToString};

/// A leaf primitive value. The value kinds plus one MARKER (`BadEscape`) the reader emits for a
/// lexically-malformed literal it cannot itself report.
///
/// `Int` is arbitrary-precision and `Float` is an exact width-free decimal: a literal's magnitude
/// or precision is never a well-formedness ceiling, and the concrete machine width (`Int64`,
/// `(Int N)`, `f32`, `f64`, …) is a *type* decision made downstream, not a representation choice
/// made here. `Float` always holds a FINITE decimal; a non-finite float VALUE (NaN, ±∞) — e.g. the
/// result of `Ast.encode` of a computed float — is a dedicated payloadless leaf ([`Leaf::FloatNan`] /
/// [`Leaf::FloatInf`]), since a decimal cannot represent it.
#[derive(Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum Leaf {
    /// An integer literal: its exact value plus the base its text used. The base is display-only
    /// (`42`, `0x2A`, `0b101010` are the same value) but is recorded so the printed form re-reads to
    /// the same leaf — a faithful text round-trip. Digit-separator (`_`) positions are NOT recorded.
    Int {
        value: IntValue,
        radix: Radix,
    },
    Float(Decimal),
    /// The non-finite float value NaN — a single canonical, sign-less, payloadless leaf. Every NaN
    /// bit-pattern collapses to this one value (so it is `Eq`/`Hash` and byte-identical by construction),
    /// letting `Ast.encode` of a computed NaN succeed where the finite-only [`Leaf::Float`] decimal cannot
    /// represent it. Distinct codec tag; no body.
    FloatNan,
    /// A non-finite float value infinity — `+∞` (`negative == false`) or `−∞` (`negative == true`). One
    /// variant with a sign discriminant (mirroring [`Leaf::Bool`] → two payloadless codec tags), so a
    /// computed infinite float `Ast.encode`s to a payloadless, canonical leaf the finite-only
    /// [`Leaf::Float`] decimal cannot hold. Distinct codec tags per sign; no body.
    FloatInf {
        negative: bool,
    },
    Str(alloc::rc::Rc<str>),
    /// A CHAR literal (`#\a`, `#\u+00E9`) — a single Unicode scalar value, the element type of a string's
    /// scalar sequence (`collections-and-text.md` §A Char Is A Single Unicode Scalar Value). A `char` is a
    /// scalar by construction, so this only ever holds a valid scalar; a literal spelling a NON-scalar
    /// (`#\u+D800`) is the `BadChar` marker instead.
    Char(char),
    /// A BYTE SEQUENCE literal — the value form of a `Bytes` (`b"…"`). Holds the raw bytes (arbitrary,
    /// NOT necessarily UTF-8, so distinct from `Str`); rendered `b"…"` (printable ASCII raw, `\n \r \t
    /// \\ \"` named, else `\xNN`). This is how a constant `Bytes` value crosses the boundary and reads
    /// back — the canonical value-form leaf for a byte sequence, the companion of `Str` for text.
    Bytes(Vec<u8>),
    Bool(bool),
    /// A SYMBOL literal (`#"meter"`) — an interned name value whose identity is its CONTENT, distinct
    /// from `Str` (a text value) and `Name` (an identifier reference). Written `#"…"` (reusing string
    /// lexing/escapes); its only observations are equality and `to-string`
    /// (`symbol-interning-direction`). Holds the symbol's text; rendered back `#"…"`. In the
    /// units-of-measure layer a base dimension is named by such a symbol (`(Unit.base #"meter")`) — this
    /// is the minimal symbol-literal slice that unblocks the units corpus surface; the full `Symbol`
    /// TYPE + intern table arrive with the symbols vertical.
    Sym(alloc::rc::Rc<str>),
    /// An identifier: a name reference, a construct head, a variant, or a qualified name segment.
    Name(alloc::rc::Rc<str>),
    /// A string literal carrying an UNRECOGNIZED ESCAPE (`"\q"`) — a reader-detected lexical defect that
    /// the front-end cannot report through the artifact channel, so it rides the binary AST as a MARKER.
    /// Resolving it is a `CDZ0001` rejection (`collections-and-text.md` §A String Literal's Escapes Are A
    /// Closed Set): the compiler is the diagnostic surface, not the reader. Holds the offending escape char.
    BadEscape(char),
    /// A CHAR literal naming a NON-scalar code point (`#\u+D800`, a surrogate) — a reader-detected lexical
    /// defect riding the binary AST as a MARKER. Resolving it is a `CDZ0002` rejection
    /// (`collections-and-text.md` §A Char Is A Single Unicode Scalar Value). Holds the literal's text.
    BadChar(alloc::rc::Rc<str>),
}

/// An arbitrary-precision integer value: a sign plus a big-endian magnitude. This is the whole of
/// what the encoding needs — a sign and a vector of bytes — so there is deliberately NO bignum
/// library behind it. The AST only CARRIES the value; compile-time arithmetic on it is a separate
/// concern (`lower::fold_arith` folds `+`/`-`/`*`/… over an `IntValue`, checked, with a provable-trap
/// fallback), reading the magnitude rather than depending on a bignum crate. The concrete machine
/// width a literal takes is a downstream type decision, not fixed here.
///
/// Canonical invariant for a value built through [`IntValue::from_i64`] / [`IntValue::zero`]: the
/// magnitude carries no leading zero bytes and is empty iff the value is zero, so equal values share
/// one representation (and one leaf-pool entry). A magnitude read off the wire is stored verbatim so
/// that `decode` is a faithful inverse of `encode`.
///
/// Because a compile-time-computed exact integer (`fold_arith` of `+`/`-`/`*`/…) canonicalizes to the
/// SAME `IntValue` as the literal of that value — one canonical magnitude, hence one leaf-pool entry and
/// one encoding — its serialized byte form depends only on the value, never on how it was computed.
//= spec/contracts/deterministic-value-form.md#numeric-values-serialize-deterministically
//# An exact numeric value MUST serialize to a byte form that is independent of how the value was computed.
#[derive(Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct IntValue {
    pub negative: bool,
    /// Big-endian magnitude bytes (most-significant first). Empty represents zero.
    pub magnitude: Vec<u8>,
}

impl IntValue {
    /// The integer zero (positive sign, empty magnitude — zero is never negative on the wire).
    pub fn zero() -> IntValue {
        IntValue {
            negative: false,
            magnitude: Vec::new(),
        }
    }

    /// Build from a machine `i64`, producing the canonical minimal big-endian magnitude.
    pub fn from_i64(v: i64) -> IntValue {
        if v == 0 {
            return IntValue::zero();
        }
        // Widen before taking the magnitude so `i64::MIN` does not overflow.
        let mag: u128 = (v as i128).unsigned_abs();
        let bytes = mag.to_be_bytes();
        // Strip leading zero bytes: the first non-zero byte begins the minimal magnitude.
        let mut start = 0;
        while start < bytes.len() && bytes[start] == 0 {
            start += 1;
        }
        IntValue {
            negative: v < 0,
            magnitude: bytes[start..].to_vec(),
        }
    }

    /// Build from an unsigned `u128`, producing the canonical minimal big-endian magnitude. Covers
    /// every unsigned bound up to 128 bits — in particular `UInt64.max = 2^64 - 1`, which does not fit
    /// an `i64`.
    pub fn from_u128(v: u128) -> IntValue {
        if v == 0 {
            return IntValue::zero();
        }
        let bytes = v.to_be_bytes();
        let mut start = 0;
        while start < bytes.len() && bytes[start] == 0 {
            start += 1;
        }
        IntValue {
            negative: false,
            magnitude: bytes[start..].to_vec(),
        }
    }

    /// Build the signed value `-v` for an unsigned magnitude `v` (v ≤ 2^127) — the negative integer
    /// bounds (`Int64.min = -(2^63)`) whose magnitude fits `u128` but whose value is negative.
    pub fn from_neg_u128(v: u128) -> IntValue {
        let mut iv = IntValue::from_u128(v);
        iv.negative = v != 0;
        iv
    }

    /// Whether this value fits a `(signed, width)` integer type — the range check an annotation
    /// `(: v IntN)` / `(: v UIntN)` performs: a signed N-bit holds `-(2^(N-1)) ..= 2^(N-1) - 1`, an
    /// unsigned N-bit holds `0 ..= 2^N - 1`. A value outside the range is rejected (never truncated).
    /// Widths `1..=128` are supported here (the fold's arbitrary-precision range).
    pub fn fits_width(&self, signed: bool, width: u32) -> bool {
        if width == 0 || width > 128 {
            return false;
        }
        // The magnitude as a u128 (values wider than 128 bits never fit a ≤128 width).
        if self.magnitude.len() > 16 {
            return false;
        }
        let mut mag: u128 = 0;
        for &b in &self.magnitude {
            mag = (mag << 8) | (b as u128);
        }
        if signed {
            if self.negative {
                // -mag fits iff mag <= 2^(width-1).
                mag <= (1u128 << (width - 1))
            } else {
                // +mag fits iff mag < 2^(width-1)  (i.e. <= 2^(width-1) - 1).
                mag < (1u128 << (width - 1))
            }
        } else {
            // Unsigned: no negatives; mag < 2^width  (i.e. <= 2^width - 1). (width==128 max = u128::MAX.)
            if self.negative && mag != 0 {
                return false;
            }
            if width == 128 {
                true
            } else {
                mag < (1u128 << width)
            }
        }
    }

    /// The 64-bit two's-complement BIT PATTERN of this value, as the `i64` an `i64.const` carries. For
    /// a value in signed range this is the value itself; for an UNSIGNED value at/above `2^63` (e.g.
    /// `UInt64.max = 2^64-1`) it is the negative `i64` with the same 64 bits (`-1`), which the unsigned
    /// boundary lift reinterprets correctly. Assumes the value FITS 64 bits (checked before selection).
    pub fn to_i64_bits(&self) -> i64 {
        let mut acc: u64 = 0;
        for &b in &self.magnitude {
            acc = (acc << 8) | (b as u64);
        }
        let bits = if self.negative {
            acc.wrapping_neg()
        } else {
            acc
        };
        bits as i64
    }

    /// The 32-bit two's-complement bit pattern for a ≤32-bit value, as the `i32` an `i32.const`
    /// carries — the low 32 bits of the value's two's-complement representation. A SIGNED negative
    /// (`-128 : Int8`) keeps its sign (`-128` as i32, sign-extended — NOT the truncated `0x80`); an
    /// UNSIGNED value at/above `2^31` (`UInt32.max = 2^32-1`) is the negative i32 with the same bits
    /// (`-1`), which the unsigned boundary lift reinterprets. Assumes the value FITS the width (checked
    /// before selection), so the low 32 bits are exactly the value — no width-masking that would strip
    /// a sign bit.
    pub fn to_i32_bits(&self, _width: u32) -> i32 {
        self.to_i64_bits() as i32
    }

    /// TRUNCATE this value to a `(signed, width)` integer, keeping the low `width` bits of its two's-
    /// complement representation and interpreting them at the target — the value `T.wrap` produces. A
    /// value already in range is unchanged (`200 → UInt8 200`); one out of range keeps its low bits
    /// (`256 → UInt8 0`, `-1 → UInt8 255`, `-1 → Int8 -1`). Total (never traps), the defining property of
    /// `wrap`. Returns the truncated value as a canonical [`IntValue`]. Widths `1..=128` supported.
    ///
    /// The low-`width` bits are taken from the value's INFINITE two's-complement expansion (a negative
    /// value has an all-ones high extension), so `-1` wrapped to any width is that width's all-ones
    /// pattern. Reinterpreting those bits at the target sign then decides the result's sign: unsigned
    /// keeps them as a magnitude; signed treats bit `width-1` as the sign bit.
    pub fn wrap_to(&self, signed: bool, width: u32) -> IntValue {
        debug_assert!((1..=128).contains(&width), "width out of range");
        // The value's low-128-bit two's-complement pattern (enough for widths ≤ 128): a magnitude for a
        // non-negative value, its two's-complement negation for a negative one.
        let mut mag: u128 = 0;
        for &b in &self.magnitude {
            mag = mag.wrapping_shl(8) | (b as u128);
        }
        let bits: u128 = if self.negative {
            mag.wrapping_neg()
        } else {
            mag
        };
        // Keep the low `width` bits.
        let low = if width >= 128 {
            bits
        } else {
            bits & ((1u128 << width) - 1)
        };
        if signed && width >= 1 && (low >> (width - 1)) & 1 == 1 {
            // The target's sign bit is set → a negative value: `low - 2^width`, i.e. magnitude
            // `2^width - low`, negative.
            let modulus = if width >= 128 { 0u128 } else { 1u128 << width };
            // width < 128 here (a 128-bit signed value with its top bit set still fits u128 magnitude
            // 2^128 - low, which overflows u128 only when low==0 — but low==0 means non-negative, so the
            // sign-bit branch is unreachable at width==128; guard with wrapping to stay total).
            let magnitude = modulus.wrapping_sub(low);
            IntValue::from_neg_u128(magnitude)
        } else {
            // Non-negative: the low bits ARE the magnitude.
            IntValue::from_u128(low)
        }
    }

    /// Narrow to a machine `i64`, or `None` if the value does not fit. Used where a downstream pass
    /// requires a fixed-width integer and must decline (not truncate) an out-of-range literal.
    pub fn to_i64(&self) -> Option<i64> {
        if self.magnitude.len() > 8 {
            return None;
        }
        let mut acc: u128 = 0;
        for &b in &self.magnitude {
            acc = (acc << 8) | (b as u128);
        }
        if self.negative {
            // A negative value fits iff its magnitude is ≤ |i64::MIN| = 2^63.
            if acc > (i64::MAX as u128) + 1 {
                return None;
            }
            Some(-(acc as i128) as i64)
        } else {
            if acc > i64::MAX as u128 {
                return None;
            }
            Some(acc as i64)
        }
    }

    /// Narrow to a machine `i128`, or `None` if the value does not fit (magnitude over 128 bits, or the
    /// `i128::MIN` boundary). A pure conversion — no arbitrary-precision ARITHMETIC (this crate has
    /// none). Used to read a compile-time unit-SCALE ratio, which is always a small machine integer
    /// (`tera` = 10¹², `tebi` = 2⁴⁰) — comfortably inside `i128`.
    pub fn to_i128(&self) -> Option<i128> {
        if self.magnitude.len() > 16 {
            return None;
        }
        let mut acc: u128 = 0;
        for &b in &self.magnitude {
            acc = (acc << 8) | (b as u128);
        }
        if self.negative {
            if acc > (i128::MAX as u128) + 1 {
                return None;
            }
            if acc == (i128::MAX as u128) + 1 {
                Some(i128::MIN)
            } else {
                Some(-(acc as i128))
            }
        } else {
            if acc > i128::MAX as u128 {
                return None;
            }
            Some(acc as i128)
        }
    }

    /// The NON-NEGATIVE magnitude as a `u128`, or `None` if it exceeds 128 bits OR the value is negative.
    /// A pure read of the magnitude bytes (big-endian) — used to fold a wide UNSIGNED shift/bitwise op over
    /// the low-width bit pattern (the operand reaching that fold is a non-negative unsigned value, so its
    /// magnitude IS its bit pattern). The unsigned twin of [`to_i128`].
    pub fn to_u128(&self) -> Option<u128> {
        if self.negative && !self.is_zero() {
            return None;
        }
        if self.magnitude.len() > 16 {
            return None;
        }
        let mut acc: u128 = 0;
        for &b in &self.magnitude {
            acc = (acc << 8) | (b as u128);
        }
        Some(acc)
    }

    /// Build from a machine `i128`, the inverse of [`to_i128`] — the canonical minimal-magnitude form.
    /// A pure conversion (no arithmetic): used to rebuild an `IntValue` from a folded unit-conversion
    /// result, which is always a machine-range integer.
    pub fn from_i128(v: i128) -> IntValue {
        if v == 0 {
            return IntValue::zero();
        }
        let mag: u128 = v.unsigned_abs();
        let bytes = mag.to_be_bytes();
        let mut start = 0;
        while start < bytes.len() && bytes[start] == 0 {
            start += 1;
        }
        IntValue {
            negative: v < 0,
            magnitude: bytes[start..].to_vec(),
        }
    }

    /// Whether two integers are EQUAL BY VALUE, independent of magnitude representation. The struct
    /// derives `PartialEq` over the raw fields, but the magnitude is NOT canonicalized on every path —
    /// a literal `0` may carry `[0]` while a folded `0` carries `[]` (empty), and both denote zero. So
    /// value comparisons (e.g. a match probe testing a folded scrutinee against a literal pattern) MUST
    /// use this, not `==`: strip leading zero bytes, and treat a zero magnitude as sign-agnostic.
    pub fn eq_value(&self, other: &IntValue) -> bool {
        fn trimmed(m: &[u8]) -> &[u8] {
            let mut i = 0;
            while i < m.len() && m[i] == 0 {
                i += 1;
            }
            &m[i..]
        }
        let (a, b) = (trimmed(&self.magnitude), trimmed(&other.magnitude));
        if a != b {
            return false;
        }
        // Equal magnitudes: sign matters only for a non-zero value (zero is never "negative").
        a.is_empty() || self.negative == other.negative
    }

    // ── Arbitrary-precision arithmetic (the "later compile-time-evaluation concern" this crate's
    // Cargo.toml names). Hand-written schoolbook algorithms over the big-endian magnitude bytes — the
    // COPY-DON'T-DEPEND rule forbids a bignum crate here (the compiler ports to Cadenza then back to
    // Rust; a shared external crate would break that round-trip). Used by `Rational` constant folding
    // (normalize via `gcd`, arithmetic via `mul`/`divmod`) and BigInt constant compare. Every op keeps
    // the canonical minimal magnitude (no leading zero bytes; zero is the empty magnitude, never
    // negative). ─────────────────────────────────────────────────────────────────────────────────────

    /// True iff the value is zero (empty canonical magnitude, or all-zero bytes).
    pub fn is_zero(&self) -> bool {
        self.magnitude.iter().all(|&b| b == 0)
    }

    /// Strip leading zero bytes + force a zero value to the canonical (positive, empty) form.
    fn normalize(mut neg: bool, mut mag: Vec<u8>) -> IntValue {
        let mut start = 0;
        while start < mag.len() && mag[start] == 0 {
            start += 1;
        }
        mag.drain(..start);
        if mag.is_empty() {
            neg = false; // zero is never negative
        }
        IntValue {
            negative: neg,
            magnitude: mag,
        }
    }

    /// Compare two MAGNITUDES (unsigned) — big-endian byte vectors, ignoring leading zeros. `Less`/
    /// `Equal`/`Greater` on the numeric magnitude.
    fn cmp_mag(a: &[u8], b: &[u8]) -> core::cmp::Ordering {
        let trim = |m: &[u8]| {
            let mut i = 0;
            while i < m.len() && m[i] == 0 {
                i += 1;
            }
            m.len() - i
        };
        let (la, lb) = (trim(a), trim(b));
        if la != lb {
            return la.cmp(&lb);
        }
        // Same significant length: compare the significant bytes MSB-first.
        let (sa, sb) = (&a[a.len() - la..], &b[b.len() - lb..]);
        sa.cmp(sb)
    }

    /// Add two magnitudes (unsigned big-endian), returning the canonical minimal sum magnitude.
    fn add_mag(a: &[u8], b: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(a.len().max(b.len()) + 1);
        let mut carry = 0u16;
        let (mut ia, mut ib) = (a.len(), b.len());
        while ia > 0 || ib > 0 || carry > 0 {
            let da = if ia > 0 {
                ia -= 1;
                a[ia] as u16
            } else {
                0
            };
            let db = if ib > 0 {
                ib -= 1;
                b[ib] as u16
            } else {
                0
            };
            let s = da + db + carry;
            out.push((s & 0xff) as u8);
            carry = s >> 8;
        }
        out.reverse();
        out
    }

    /// Subtract two magnitudes (unsigned big-endian), REQUIRES `a >= b`. Returns the canonical minimal
    /// difference magnitude.
    fn sub_mag(a: &[u8], b: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(a.len());
        let mut borrow = 0i16;
        let (mut ia, mut ib) = (a.len(), b.len());
        while ia > 0 {
            ia -= 1;
            let da = a[ia] as i16;
            let db = if ib > 0 {
                ib -= 1;
                b[ib] as i16
            } else {
                0
            };
            let mut d = da - db - borrow;
            if d < 0 {
                d += 256;
                borrow = 1;
            } else {
                borrow = 0;
            }
            out.push(d as u8);
        }
        out.reverse();
        let mut start = 0;
        while start < out.len() && out[start] == 0 {
            start += 1;
        }
        out.drain(..start);
        out
    }

    /// Multiply two magnitudes (unsigned big-endian), returning the canonical minimal product magnitude.
    fn mul_mag(a: &[u8], b: &[u8]) -> Vec<u8> {
        if a.is_empty() || b.is_empty() {
            return Vec::new();
        }
        // Schoolbook: product limb array is little-endian during accumulation, then reversed.
        let mut acc = vec![0u32; a.len() + b.len()];
        for (i, &ai) in a.iter().rev().enumerate() {
            let mut carry = 0u32;
            for (j, &bj) in b.iter().rev().enumerate() {
                let idx = i + j;
                let cur = acc[idx] + (ai as u32) * (bj as u32) + carry;
                acc[idx] = cur & 0xff;
                carry = cur >> 8;
            }
            let mut k = i + b.len();
            while carry > 0 {
                let cur = acc[k] + carry;
                acc[k] = cur & 0xff;
                carry = cur >> 8;
                k += 1;
            }
        }
        let mut out: Vec<u8> = acc.iter().rev().map(|&d| d as u8).collect();
        let mut start = 0;
        while start < out.len() && out[start] == 0 {
            start += 1;
        }
        out.drain(..start);
        out
    }

    /// Divide magnitude `a` by magnitude `b` (unsigned big-endian, `b` nonzero), returning `(quotient,
    /// remainder)` magnitudes, both canonical. Bit-at-a-time long division — `a` is small at compile
    /// time (a folded literal), so simplicity beats a limb-wise Knuth division.
    fn divmod_mag(a: &[u8], b: &[u8]) -> (Vec<u8>, Vec<u8>) {
        // Number of significant bits in `a` (MSB-first).
        let sig = |m: &[u8]| -> usize {
            let mut i = 0;
            while i < m.len() && m[i] == 0 {
                i += 1;
            }
            m.len() - i
        };
        let a_sig = sig(a);
        if a_sig == 0 || IntValue::cmp_mag(a, b) == core::cmp::Ordering::Less {
            // a < b → quotient 0, remainder a (canonicalized).
            return (Vec::new(), IntValue::sub_mag(a, &[]));
        }
        let total_bits = a_sig * 8;
        // Build the quotient bit-by-bit from the most-significant bit down.
        let bit = |m: &[u8], idx: usize| -> u8 {
            // idx counts from LSB (0) up; return that bit.
            let byte_from_end = idx / 8;
            let bit_in_byte = idx % 8;
            if byte_from_end >= m.len() {
                return 0;
            }
            (m[m.len() - 1 - byte_from_end] >> bit_in_byte) & 1
        };
        let mut rem: Vec<u8> = Vec::new(); // running remainder magnitude (canonical)
        let mut quo_bits: Vec<u8> = Vec::with_capacity(total_bits);
        for i in (0..total_bits).rev() {
            // rem = rem << 1 | bit_i(a)
            rem = IntValue::shl1_mag(&rem);
            if bit(a, i) == 1 {
                // set the low bit of rem
                if rem.is_empty() {
                    rem = vec![1];
                } else {
                    let last = rem.len() - 1;
                    rem[last] |= 1;
                }
            }
            if IntValue::cmp_mag(&rem, b) != core::cmp::Ordering::Less {
                rem = IntValue::sub_mag(&rem, b);
                quo_bits.push(1);
            } else {
                quo_bits.push(0);
            }
        }
        // quo_bits is MSB-first; pack into bytes.
        let quo = IntValue::pack_bits_be(&quo_bits);
        (quo, rem)
    }

    /// Shift a magnitude left by one bit (multiply by 2), canonical result.
    fn shl1_mag(m: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(m.len() + 1);
        let mut carry = 0u8;
        for &byte in m.iter().rev() {
            let v = ((byte as u16) << 1) | carry as u16;
            out.push((v & 0xff) as u8);
            carry = (v >> 8) as u8;
        }
        if carry > 0 {
            out.push(carry);
        }
        out.reverse();
        let mut start = 0;
        while start < out.len() && out[start] == 0 {
            start += 1;
        }
        out.drain(..start);
        out
    }

    /// Shift a magnitude RIGHT by one bit (divide by 2, floor), canonical result. The LSB of each byte
    /// flows into the MSB (bit 7) of the next-less-significant byte's result, processing MSB-first.
    fn shr1_mag(m: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(m.len());
        let mut carry = 0u8; // the low bit of the more-significant byte just processed
        for &byte in m.iter() {
            out.push((carry << 7) | (byte >> 1));
            carry = byte & 1;
        }
        let mut start = 0;
        while start < out.len() && out[start] == 0 {
            start += 1;
        }
        out.drain(..start);
        out
    }

    /// Pack an MSB-first bit vector into canonical big-endian bytes.
    fn pack_bits_be(bits: &[u8]) -> Vec<u8> {
        // Drop leading zero bits.
        let first_one = bits.iter().position(|&b| b == 1);
        let Some(first) = first_one else {
            return Vec::new();
        };
        let sig = &bits[first..];
        let pad = (8 - sig.len() % 8) % 8;
        let mut out = Vec::new();
        let mut cur = 0u8;
        let mut count = 0usize;
        for &b in core::iter::repeat_n(&0u8, pad).chain(sig.iter()) {
            cur = (cur << 1) | b;
            count += 1;
            if count == 8 {
                out.push(cur);
                cur = 0;
                count = 0;
            }
        }
        out
    }

    /// The greatest common divisor of two MAGNITUDES (unsigned). `gcd(0,0)=0`; `gcd(a,0)=a`.
    ///
    /// BINARY GCD (Stein's algorithm), not Euclidean: it uses only halving (`shr1_mag`), subtraction, and
    /// comparison — never `divmod_mag`. This is the HOT compile-time path (`normalized_rational` reduces
    /// every folded `Rational` to lowest terms), and `divmod_mag` is bit-serial (`8·len(a)` iterations per
    /// call regardless of quotient size), so a Euclidean gcd of two large coprime magnitudes — the shape a
    /// chained exact-rational sum produces, where distinct denominators MULTIPLY without cancellation — was
    /// super-cubic (a 160-term `(+ (Rational.of 1 p0) …)` fold: ~1.8s, 99% in `divmod_mag`). Binary GCD
    /// removes the trial division entirely: each step strips shared/individual factors of two and does one
    /// subtract, so it is O(bits²) with a small constant on the same shape (the fold drops to milliseconds).
    fn gcd_mag(a: &[u8], b: &[u8]) -> Vec<u8> {
        let mut x = IntValue::sub_mag(a, &[]); // canonicalize (strip leading zeros)
        let mut y = IntValue::sub_mag(b, &[]);
        // gcd(a,0)=a, gcd(0,b)=b (and gcd(0,0)=0 falls out — x stays empty).
        if x.is_empty() {
            return y;
        }
        if y.is_empty() {
            return x;
        }
        // Factor out the largest power of two dividing BOTH — `shift` common trailing zero bits. `gcd =
        // 2^shift · gcd(x>>shift, y>>shift)`, restored by shifting the odd-core result back up at the end.
        let mut shift = 0usize;
        while (x[x.len() - 1] & 1) == 0 && (y[y.len() - 1] & 1) == 0 {
            x = IntValue::shr1_mag(&x);
            y = IntValue::shr1_mag(&y);
            shift += 1;
        }
        // Remove remaining factors of two from x, so x is odd at each loop entry.
        while (x[x.len() - 1] & 1) == 0 {
            x = IntValue::shr1_mag(&x);
        }
        loop {
            // y is made odd (its factors of two cannot be common — x is odd).
            while !y.is_empty() && (y[y.len() - 1] & 1) == 0 {
                y = IntValue::shr1_mag(&y);
            }
            if y.is_empty() {
                break;
            }
            // Both x and y are now odd; subtract the smaller from the larger (the difference is even and
            // handled by the halving at the loop top). Keep x ≤ y so x holds the running gcd core.
            if IntValue::cmp_mag(&x, &y) == core::cmp::Ordering::Greater {
                core::mem::swap(&mut x, &mut y);
            }
            y = IntValue::sub_mag(&y, &x);
        }
        // Restore the common factors of two.
        for _ in 0..shift {
            x = IntValue::shl1_mag(&x);
        }
        x
    }

    /// Signed comparison of two integer values (canonical or not) — the NUMERIC order, which is NOT the
    /// derived field order (`Ord` on the struct would compare `negative` then the raw magnitude bytes,
    /// getting negatives and non-canonical magnitudes wrong), so this is a named method, not an `Ord` impl.
    #[allow(clippy::should_implement_trait)]
    pub fn cmp(&self, other: &IntValue) -> core::cmp::Ordering {
        use core::cmp::Ordering::*;
        let (za, zb) = (self.is_zero(), other.is_zero());
        match (za, zb) {
            (true, true) => return Equal,
            (true, false) => return if other.negative { Greater } else { Less },
            (false, true) => return if self.negative { Less } else { Greater },
            (false, false) => {}
        }
        match (self.negative, other.negative) {
            (false, true) => Greater,
            (true, false) => Less,
            (false, false) => IntValue::cmp_mag(&self.magnitude, &other.magnitude),
            (true, true) => IntValue::cmp_mag(&other.magnitude, &self.magnitude),
        }
    }

    /// Signed addition.
    pub fn add(&self, other: &IntValue) -> IntValue {
        if self.negative == other.negative {
            // Same sign: add magnitudes, keep the sign.
            IntValue::normalize(
                self.negative,
                IntValue::add_mag(&self.magnitude, &other.magnitude),
            )
        } else {
            // Opposite signs: subtract the smaller magnitude from the larger; sign of the larger.
            match IntValue::cmp_mag(&self.magnitude, &other.magnitude) {
                core::cmp::Ordering::Equal => IntValue::zero(),
                core::cmp::Ordering::Greater => IntValue::normalize(
                    self.negative,
                    IntValue::sub_mag(&self.magnitude, &other.magnitude),
                ),
                core::cmp::Ordering::Less => IntValue::normalize(
                    other.negative,
                    IntValue::sub_mag(&other.magnitude, &self.magnitude),
                ),
            }
        }
    }

    /// Signed negation.
    pub fn neg(&self) -> IntValue {
        if self.is_zero() {
            IntValue::zero()
        } else {
            IntValue {
                negative: !self.negative,
                magnitude: self.magnitude.clone(),
            }
        }
    }

    /// Signed subtraction (`self - other`).
    pub fn sub(&self, other: &IntValue) -> IntValue {
        self.add(&other.neg())
    }

    /// Signed multiplication.
    pub fn mul(&self, other: &IntValue) -> IntValue {
        let mag = IntValue::mul_mag(&self.magnitude, &other.magnitude);
        IntValue::normalize(self.negative != other.negative, mag)
    }

    /// Truncating signed division `(quotient, remainder)` — quotient toward zero, remainder takes the
    /// DIVIDEND's sign (matching fixed-width `/`/`%`). Returns `None` on a zero divisor. `q*d + r == n`.
    pub fn divmod(&self, other: &IntValue) -> Option<(IntValue, IntValue)> {
        if other.is_zero() {
            return None;
        }
        let (q_mag, r_mag) = IntValue::divmod_mag(&self.magnitude, &other.magnitude);
        let q = IntValue::normalize(self.negative != other.negative, q_mag);
        let r = IntValue::normalize(self.negative, r_mag); // remainder takes the dividend's sign
        Some((q, r))
    }

    /// The GCD of two values (by magnitude — the result is non-negative). `gcd(0,0)=0`.
    pub fn gcd(&self, other: &IntValue) -> IntValue {
        IntValue::normalize(false, IntValue::gcd_mag(&self.magnitude, &other.magnitude))
    }

    /// The DECIMAL string of this value (with a leading `-` for a negative), independent of magnitude
    /// size — extracts digits by repeated division by 10 over the magnitude (this crate has no bignum
    /// crate, so `to_i128` + `format!` would cap the value; a folded Rational numerator can exceed i128).
    /// `0` renders `"0"`.
    pub fn to_decimal_string(&self) -> String {
        if self.is_zero() {
            return "0".to_string();
        }
        let ten = [10u8];
        let mut mag = IntValue::sub_mag(&self.magnitude, &[]); // canonical copy
        let mut digits = Vec::new();
        while !mag.is_empty() {
            let (q, r) = IntValue::divmod_mag(&mag, &ten);
            let d = r.last().copied().unwrap_or(0);
            digits.push(b'0' + d);
            mag = q;
        }
        if self.negative {
            digits.push(b'-');
        }
        digits.reverse();
        String::from_utf8(digits).expect("ascii digits")
    }
}

/// The base an integer literal's text used. Display-only — it does not change the value.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum Radix {
    Dec,
    Hex,
    Bin,
}

/// A structure entry. Frozen at 2 variants.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Struct {
    /// An occurrence of a leaf value.
    Atom(LeafId),
    /// A form `(child…)`: an ordered sequence of child occurrences.
    List(Vec<StructId>),
}

/// The primitive compound-value constructor a node denotes — the first-class TAG that says "this node
/// is a record / tuple / list / map". It is read from the reserved STRING-LITERAL head (`("record" …)`
/// etc.) via [`Arenas::compound_ctor`], the unshadowable primitive form the resolver dispatches
/// structurally. The shadowable prelude ALIAS (`(record …)`, a NAME head) is deliberately NOT a tag —
/// it resolves lexically-first, so a program binding named `record` shadows it. Recognizing the kind by
/// this typed tag rather than by re-comparing head text at each consumer is the native-compound-data
/// migration (see `implementation/design/DESIGN-native-ast-compound-data.md`). (`set` is not yet a
/// primitive constructor — held for operator decision D2 in that design.)
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CompoundCtor {
    /// `("record" (= k v)…)` — a record: `(= key value)` field-pair children.
    Record,
    /// `("tuple" e…)` — a tuple: positional element children.
    Tuple,
    /// `("list" e…)` — a list: element children.
    List,
    /// `("map" (k v)…)` — a map: key/value entry-pair children.
    Map,
    /// `("set" e…)` — a set: element children (dedup at build). A first-class tagged construction pulled
    /// all the way through the compiler (operator ruling 2026-08-27); lowers to `Core::SetOf`. The set
    /// VALUE still renders `(Set.of (list …sorted))`.
    Set,
}

impl CompoundCtor {
    /// Map a reserved compound-ctor head spelling to its typed tag — the single place this crate matches
    /// the reserved compound vocabulary. `None` for any other spelling.
    fn from_spelling(s: &str) -> Option<CompoundCtor> {
        match s {
            "record" => Some(CompoundCtor::Record),
            "tuple" => Some(CompoundCtor::Tuple),
            "list" => Some(CompoundCtor::List),
            "map" => Some(CompoundCtor::Map),
            "set" => Some(CompoundCtor::Set),
            _ => None,
        }
    }
}

/// Index into the leaf pool.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct LeafId(pub u32);

/// Index into the structure arena.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct StructId(pub u32);

/// An exact, width-free decimal value: `(-1)^negative * significand * 10^exponent`.
///
/// The significand is an arbitrary-precision non-negative magnitude stored as big-endian bytes (the
/// same dependency-free representation as [`IntValue::magnitude`]); the sign lives in `negative` so
/// that `-0.0` (negative, empty significand) is preserved distinctly from `0.0`. This captures a
/// source float literal EXACTLY (no `f64` rounding), so a later type-directed rounding to a chosen
/// width happens once, from the exact value.
#[derive(Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct Decimal {
    pub negative: bool,
    /// Big-endian non-negative magnitude of the significand. Empty represents zero.
    pub significand: Vec<u8>,
    /// Base-10 exponent.
    pub exponent: i64,
}

impl Decimal {
    /// The IEEE-754 double the literal denotes, as its raw BIT PATTERN (`f64::to_bits`). Bits, not the
    /// `f64` itself, so structural equality is exact: `-0.0` and `0.0` have DISTINCT bits (they compare
    /// UNEQUAL, as the canonical value form requires), and a NaN compares unequal to itself by value but
    /// its bits are stable. The exact `Decimal` is reconstructed as a decimal string (`significand ·
    /// 10^exponent`, with sign) and parsed by Rust's `f64` reader — correctly-rounded to the nearest
    /// double, the type-directed rounding `numeric-model.md` pins for a `Float64`. Overflow parses to
    /// `±inf` (a defined double), underflow to `±0.0` — both real Float64 values.
    pub fn to_f64_bits(&self) -> u64 {
        // The significand is a big-endian BASE-256 magnitude (like `IntValue::magnitude`), so convert it
        // to base-10 DIGITS first — Horner over the bytes, each step `acc = acc*256 + byte` carried out
        // in a decimal-digit vector (little-endian digits, printed reversed). Then reconstruct
        // `[-]<decimal-digits>e<exponent>` and let the standard library round it to the nearest double —
        // the type-directed rounding `numeric-model.md` pins for `Float64`. An empty significand is zero
        // ("0"), so `-0.0` keeps its sign through the `-` prefix; overflow → `±inf`, underflow → `±0.0`.
        let mut digits: Vec<u8> = vec![0]; // little-endian base-10 digits; starts at zero
        for &byte in &self.significand {
            let mut carry = byte as u32;
            for d in digits.iter_mut() {
                let v = (*d as u32) * 256 + carry;
                *d = (v % 10) as u8;
                carry = v / 10;
            }
            while carry > 0 {
                digits.push((carry % 10) as u8);
                carry /= 10;
            }
        }
        let mut s: String = digits.iter().rev().map(|d| (b'0' + d) as char).collect();
        // Trim leading zeros the reversed build may have left (e.g. "007").
        let trimmed = s.trim_start_matches('0');
        s = if trimmed.is_empty() {
            "0".to_string()
        } else {
            trimmed.to_string()
        };
        let sign = if self.negative { "-" } else { "" };
        let text = format!("{sign}{s}e{}", self.exponent);
        // A well-formed `Decimal` always parses; a pathological one falls back to a canonical zero.
        text.parse::<f64>().unwrap_or(0.0).to_bits()
    }

    /// Build a `Decimal` from a computed `f64` so that `to_f64_bits()` returns EXACTLY this value's
    /// bits — the inverse used by the float-arithmetic fold to represent a computed result (`(+ 0.1
    /// 0.2)` → the f64 `0.30000000000000004`) as a `Core::ConstFloat`. `None` for a NON-FINITE result
    /// (`±inf`/NaN): a `Decimal` holds only finite values, and inf/NaN have no written form the reader
    /// accepts (the float-literal-overflow gap), so a fold producing one DECLINES rather than inventing
    /// a value form. Uses Rust's shortest round-tripping formatting (`{:e}`, Ryū) — the produced decimal
    /// re-parses to the same f64 bit-for-bit — then converts the base-10 significand to the base-256
    /// magnitude `to_f64_bits` reads. `-0.0` is preserved (its `{:e}` form carries the leading `-`).
    pub fn from_f64(f: f64) -> Option<Decimal> {
        if !f.is_finite() {
            return None;
        }
        // A WHOLE float uses its FULL exact expansion (`{f:.0}`), matching scalar display_float + rust +
        // the runtime `float_leaf`; a non-whole keeps `{:e}` (shortest == written form). Both round-trip to
        // the same bits — this changes the digit REPRESENTATION, not the value.
        // `f.fract() == 0.0` (is `f` a whole number) without the std-only `f64::fract`, so this file
        // compiles under `#![no_std]` (cdz-runtime `include!`s it). Exact and identical to `fract() ==
        // 0.0` for every finite f64 (guarded above): inspect the IEEE-754 fields — an unbiased exponent
        // < 0 (|f| < 1) is whole only at ±0.0; an exponent >= 52 leaves no fractional mantissa bits;
        // otherwise f is whole iff its low `52 - exp` mantissa bits are all zero.
        let is_whole = {
            let bits = f.to_bits();
            let exp = ((bits >> 52) & 0x7ff) as i64 - 1023;
            if exp < 0 {
                f == 0.0
            } else if exp >= 52 {
                true
            } else {
                (bits & ((1u64 << (52 - exp)) - 1)) == 0
            }
        };
        let s = if is_whole {
            format!("{f:.0}")
        } else {
            format!("{f:e}")
        };
        let (negative, rest) = match s.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, s.as_str()),
        };
        let (mantissa, exp10): (&str, i64) = match rest.split_once('e') {
            Some((m, e)) => (m, e.parse().ok()?),
            None => (rest, 0),
        };
        // Fold the fractional digits into the exponent: `D.FFFF` with k frac digits = `DFFFF · 10^-k`.
        let (int_part, frac_part) = match mantissa.split_once('.') {
            Some((i, fr)) => (i, fr),
            None => (mantissa, ""),
        };
        let mut digits = String::from(int_part);
        digits.push_str(frac_part);
        let exponent = exp10 - frac_part.len() as i64;
        // Convert the base-10 digit string to a big-endian base-256 magnitude (Horner: acc = acc*10 + d,
        // carried in a little-endian byte vector). Leading zeros collapse to the empty magnitude (zero).
        let mut mag: Vec<u8> = Vec::new(); // little-endian base-256 during build
        for ch in digits.bytes() {
            if !ch.is_ascii_digit() {
                return None;
            }
            let mut carry = (ch - b'0') as u32;
            for byte in mag.iter_mut() {
                let v = (*byte as u32) * 10 + carry;
                *byte = (v & 0xff) as u8;
                carry = v >> 8;
            }
            while carry > 0 {
                mag.push((carry & 0xff) as u8);
                carry >>= 8;
            }
        }
        // Strip leading (most-significant) zeros → big-endian, minimal, empty iff zero.
        while mag.last() == Some(&0) {
            mag.pop();
        }
        mag.reverse();
        Some(Decimal {
            negative,
            significand: mag,
            exponent,
        })
    }

    /// Whether the value this decimal denotes rounds to a FINITE `Float64`. A literal whose magnitude
    /// exceeds the largest finite double (`~1.8e308`) rounds to `±inf` — a value with no written form
    /// the reader accepts — so it is a MALFORMED literal (`numeric-model.md` §A Floating-Point Literal
    /// That Denotes No Representable Value Is Malformed), the float analogue of an out-of-range integer
    /// literal. A `Decimal` is always finite itself (the reader produces no `inf`), so this asks only
    /// whether the ROUNDING overflows. (`Float32` is narrower, so a `(: 1e40 Float32)` overflow is
    /// caught at the annotation, not here; the bare-literal default is `Float64`.)
    pub fn is_finite_f64(&self) -> bool {
        f64::from_bits(self.to_f64_bits()).is_finite()
    }

    /// Whether the literal fits `Float32` — its `f64` value, cast to `f32`, is still FINITE. A magnitude
    /// past the largest finite `f32` (`~3.4e38`) rounds to `±inf` in `Float32` (a value with no written
    /// form), so `(: 1e40 Float32)` is a MALFORMED-for-the-width literal, the `Float32` analogue of
    /// `is_finite_f64` (which guards the `Float64` default). A value that already overflows `f64` fails
    /// this too (its `f64` is `±inf`, and `inf as f32` stays `inf`).
    pub fn fits_f32(&self) -> bool {
        (f64::from_bits(self.to_f64_bits()) as f32).is_finite()
    }
}

/// The two arenas plus the root occurrence — the whole AST of one program unit.
#[derive(Clone, PartialEq, Debug)]
pub struct Arenas {
    pub leaves: Vec<Leaf>,
    pub structure: Vec<Struct>,
    pub root: StructId,
}

/// Builds `Arenas`: interns leaves on insert (dedup), appends structure occurrences (no dedup, so
/// each call is a distinct occurrence and spans stay 1:1). `root` is set once the top occurrence
/// is known via [`Builder::finish`].
#[derive(Default)]
pub struct Builder {
    leaves: Vec<Leaf>,
    leaf_index: BTreeMap<Leaf, LeafId>,
    structure: Vec<Struct>,
}

impl Builder {
    pub fn new() -> Builder {
        Builder::default()
    }

    /// Intern a leaf, returning its (possibly pre-existing) id.
    pub fn leaf(&mut self, leaf: Leaf) -> LeafId {
        if let Some(&id) = self.leaf_index.get(&leaf) {
            return id;
        }
        let id = LeafId(self.leaves.len() as u32);
        self.leaves.push(leaf.clone());
        self.leaf_index.insert(leaf, id);
        id
    }

    /// Push an `Atom` occurrence of a leaf. Not deduplicated — a fresh occurrence every call.
    pub fn atom(&mut self, leaf: LeafId) -> StructId {
        self.push(Struct::Atom(leaf))
    }

    /// Intern a leaf WITHOUT deduplication — a fresh pool entry every call, even for an equal leaf. The
    /// value-form TEMPLATE (`lower::runtime_value_form_template`) needs every placeholder leaf to be its
    /// OWN pool entry so each has a DISTINCT byte offset a runtime hole can write independently (two
    /// equal placeholders must not collapse to one offset). The ordinary `leaf` still dedups.
    pub fn leaf_unique(&mut self, leaf: Leaf) -> LeafId {
        let id = LeafId(self.leaves.len() as u32);
        self.leaves.push(leaf);
        id
    }

    /// Push a `List` occurrence. Not deduplicated.
    pub fn list(&mut self, children: Vec<StructId>) -> StructId {
        self.push(Struct::List(children))
    }

    /// Convenience: intern `leaf` and push an `Atom` occurrence of it in one step.
    pub fn atom_leaf(&mut self, leaf: Leaf) -> StructId {
        let id = self.leaf(leaf);
        self.atom(id)
    }

    /// Convenience: an atom occurrence of a `Name`.
    pub fn name(&mut self, name: impl Into<alloc::rc::Rc<str>>) -> StructId {
        self.atom_leaf(Leaf::Name(name.into()))
    }

    /// Build a canonical `(= key value)` FIELD PAIR node — the shape shared by record fields and map
    /// entries. The emit twin of [`Arenas::field_pair`], so record/map construction routes through one
    /// place. See `implementation/design/DESIGN-native-ast-compound-data.md`.
    pub fn field_pair(&mut self, key: StructId, value: StructId) -> StructId {
        let eq = self.name("=");
        self.list(vec![eq, key, value])
    }

    // ── Canonical WIT schema-descriptor builders (schema-hash-only effect identity) ──────────────
    //
    // COPY, DON'T DEPEND (rcdzc/Cargo.toml directive): these are a VERBATIM copy of the WIT-descriptor
    // builders that produce an effect's schema-hash-bearing tree, NOT a re-implementation and NOT a
    // dependency. `effect_schema_tree`/`wit_func_sig`/`wit_type_{prim,list,option,unit,tuple}` are copied
    // byte-for-byte from `cadenza-ast/src/ast.rs` (the builder the kernel builds its built-in descriptors
    // with); `wit_type_{record,variant}` are copied from `cdz-kernel/src/ast_marshal.rs` (the kernel's
    // directly-built forms, byte-identical to what `build_type` reflects off a component). rcdzc reifies a
    // USERSPACE effect's descriptor here (its own arena) so `Hash::of(codec::encode(tree))` matches the
    // kernel's built-in identity by CONSTRUCTION — same node shapes, same head-kinds (Name vs Str is
    // load-bearing), same order. A dev-test bridge (`tests.rs`) asserts these copies stay byte-identical to
    // their originals, exactly as the copied `codec.rs` byte-identity discipline does.
    //
    // Field/case ORDER is the identity (concierge ruling 2026-08-13): records NAME-SORTED, tuples
    // positional, variant cases DECL-ORDER. The caller (`lower::ty_to_wit_desc`) passes fields already in
    // name-sorted order (it iterates the `Ty::Record` BTreeMap in natural order) and cases in decl order —
    // these builders preserve the caller's order and never re-sort, so a caller cannot drift the identity.

    /// Build the effect SCHEMA tree `(effect Name (op OpName Sig)…)` — the tree whose
    /// `Hash::of(codec::encode(root))` is the effect's schema-hash identity. Structural heads
    /// (`effect`/`op`) are NAME atoms; each per-op `Sig` is a `wit_func_sig` node. `ops` is
    /// `(op_name, signature_node)` in the caller's order (op order participates in the hash — the caller
    /// sorts if it wants order-independent identity). No authz node: the schema is data-shape only.
    pub fn effect_schema_tree(&mut self, name: &str, ops: &[(&str, StructId)]) -> StructId {
        let mut children = Vec::with_capacity(2 + ops.len());
        let effect_head = self.name("effect");
        children.push(effect_head);
        let ename = self.name(name);
        children.push(ename);
        for &(op_name, sig) in ops {
            let op_head = self.name("op");
            let opn = self.name(op_name);
            let op_node = self.list(vec![op_head, opn, sig]);
            children.push(op_node);
        }
        self.list(children)
    }

    /// Build a WIT function-signature node `(func (param PName Desc)… (result Desc))`. Params are
    /// `(param_name, type_descriptor_node)` in declaration order (positional-and-named — order and name
    /// participate in the identity); `result` is the ALWAYS-present result descriptor (a no-return member
    /// passes a `unit` descriptor, never an omitted slot). `func`/`param`/`result` heads are NAME atoms.
    pub fn wit_func_sig(&mut self, params: &[(&str, StructId)], result: StructId) -> StructId {
        let mut children = Vec::with_capacity(1 + params.len() + 1);
        let func_head = self.name("func");
        children.push(func_head);
        for &(param_name, desc) in params {
            let param_head = self.name("param");
            let pn = self.name(param_name);
            let param_node = self.list(vec![param_head, pn, desc]);
            children.push(param_node);
        }
        let result_head = self.name("result");
        let result_node = self.list(vec![result_head, result]);
        children.push(result_node);
        self.list(children)
    }

    /// Build a PRIMITIVE WIT type descriptor `(kind)` — a one-element list whose sole child is a NAME atom
    /// naming the primitive (`u8`, `string`, `bool`, `s64`, `f64`, …). A primitive is a NAME-head one-element
    /// list, a compound is a STRING-head form — so a `Name` vs `Str` head DISTINGUISHES prim from compound,
    /// and the codec's distinct Name/Str bytes make the choice load-bearing for identity.
    pub fn wit_type_prim(&mut self, kind: &str) -> StructId {
        let head = self.name(kind);
        self.list(vec![head])
    }

    /// Build a `list<T>` WIT type descriptor `("list" <elem>)` — a STRING-atom head `list` then the element
    /// type descriptor.
    pub fn wit_type_list(&mut self, elem: StructId) -> StructId {
        let head = self.atom_leaf(Leaf::Str("list".into()));
        self.list(vec![head, elem])
    }

    /// Build an `option<T>` WIT type descriptor `("option" <inner>)` — a STRING-atom head `option` then the
    /// inner type descriptor (the TYPE-side option, distinct from a value's `Some`/`None` ctor).
    pub fn wit_type_option(&mut self, inner: StructId) -> StructId {
        let head = self.atom_leaf(Leaf::Str("option".into()));
        self.list(vec![head, inner])
    }

    /// Build a `unit` WIT type descriptor `("unit")` — a STRING-atom head `unit`, no children (a STR-head
    /// marker, not a component-model scalar).
    pub fn wit_type_unit(&mut self) -> StructId {
        let head = self.atom_leaf(Leaf::Str("unit".into()));
        self.list(vec![head])
    }

    /// Build a `tuple<A, B, …>` WIT type descriptor `("tuple" <a> <b> …)` — a STRING-atom head `tuple` then
    /// each element type descriptor in positional order (order is identity).
    pub fn wit_type_tuple(&mut self, elems: &[StructId]) -> StructId {
        let mut children = Vec::with_capacity(1 + elems.len());
        children.push(self.atom_leaf(Leaf::Str("tuple".into())));
        children.extend_from_slice(elems);
        self.list(children)
    }

    /// Build a `record{field: ty…}` WIT type descriptor `("record" (fname <ty>)…)` — a STRING-atom head
    /// `record`, then one `(name-node ty-node)` 2-list per field. Fields are passed in the caller's order
    /// (the caller iterates a name-sorted `Ty::Record` BTreeMap, so fields arrive NAME-SORTED, matching the
    /// kernel's now-name-sorted record descriptor). Copied from `cdz-kernel/src/ast_marshal.rs`.
    pub fn wit_type_record(&mut self, fields: &[(&str, StructId)]) -> StructId {
        let mut children = Vec::with_capacity(1 + fields.len());
        children.push(self.atom_leaf(Leaf::Str("record".into())));
        for &(name, ty) in fields {
            let name_node = self.name(name);
            let entry = self.list(vec![name_node, ty]);
            children.push(entry);
        }
        self.list(children)
    }

    /// Build a `variant{Case(T)?…}` WIT type descriptor `("variant" (Case <T>?)…)` — a STRING-atom head
    /// `variant`, then one `(CaseName ty?)` entry per case (a payload-bearing case is a 2-list `(CaseName
    /// ty)`, a payload-less case a 1-list `(CaseName)`). Cases are passed in the caller's DECL order (order
    /// participates in the identity). Copied from `cdz-kernel/src/ast_marshal.rs`.
    pub fn wit_type_variant(&mut self, cases: &[(&str, Option<StructId>)]) -> StructId {
        let mut children = Vec::with_capacity(1 + cases.len());
        children.push(self.atom_leaf(Leaf::Str("variant".into())));
        for &(case, ty) in cases {
            let case_head = self.name(case);
            let entry = match ty {
                Some(t) => self.list(vec![case_head, t]),
                None => self.list(vec![case_head]),
            };
            children.push(entry);
        }
        self.list(children)
    }

    /// Build an `enum{Case…}` WIT type descriptor `("enum" Case…)` — a STRING-atom head `enum`, then one
    /// bare NAME leaf per case (an enum is the degenerate variant: every case is payload-less). Cases are
    /// passed in DECL order (order participates in the identity).
    pub fn wit_type_enum(&mut self, cases: &[&str]) -> StructId {
        let mut children = Vec::with_capacity(1 + cases.len());
        children.push(self.atom_leaf(Leaf::Str("enum".into())));
        for &case in cases {
            let c = self.name(case);
            children.push(c);
        }
        self.list(children)
    }

    /// Build a `flags{Name…}` WIT type descriptor `("flags" Name…)` — a STRING-atom head `flags`, then one
    /// bare NAME leaf per bit. Same NODE shape as an enum but a DISTINCT type (the str head discriminates);
    /// names are in DECL order (order participates in the identity).
    pub fn wit_type_flags(&mut self, names: &[&str]) -> StructId {
        let mut children = Vec::with_capacity(1 + names.len());
        children.push(self.atom_leaf(Leaf::Str("flags".into())));
        for &name in names {
            let n = self.name(name);
            children.push(n);
        }
        self.list(children)
    }

    /// Build a `result<ok, err>` WIT type descriptor `("result" <ok-slot> <err-slot>)` — a STRING-atom head
    /// `result` then EXACTLY two slots, one per arm. A present arm is its type descriptor; an arm WIT omits
    /// (`result<T>` has no err, `result<_, E>` no ok, bare `result` neither) is the absent-marker `("none")`
    /// — a STR-head 1-list DISTINCT from `("unit")` (a present arm whose type IS `unit`). Fixed 2-arity (no
    /// optional slot) keeps the shape uniform so the byte-exact identity never drifts on a presence marker.
    pub fn wit_type_result(&mut self, ok: Option<StructId>, err: Option<StructId>) -> StructId {
        let head = self.atom_leaf(Leaf::Str("result".into()));
        let ok_slot = match ok {
            Some(t) => t,
            None => {
                let none = self.atom_leaf(Leaf::Str("none".into()));
                self.list(vec![none])
            }
        };
        let err_slot = match err {
            Some(t) => t,
            None => {
                let none = self.atom_leaf(Leaf::Str("none".into()));
                self.list(vec![none])
            }
        };
        self.list(vec![head, ok_slot, err_slot])
    }

    fn push(&mut self, s: Struct) -> StructId {
        let id = StructId(self.structure.len() as u32);
        self.structure.push(s);
        id
    }

    pub fn finish(self, root: StructId) -> Arenas {
        Arenas {
            leaves: self.leaves,
            structure: self.structure,
            root,
        }
    }
}

impl Arenas {
    /// The structure entry at `id`.
    pub fn get(&self, id: StructId) -> &Struct {
        &self.structure[id.0 as usize]
    }

    /// The leaf at `id`.
    pub fn leaf(&self, id: LeafId) -> &Leaf {
        &self.leaves[id.0 as usize]
    }

    /// If `id` is an `Atom` of a `Name`, that name.
    pub fn as_name(&self, id: StructId) -> Option<&str> {
        match self.get(id) {
            Struct::Atom(l) => match self.leaf(*l) {
                Leaf::Name(n) => Some(n),
                _ => None,
            },
            _ => None,
        }
    }

    /// If `id` is an `Atom` of a SYMBOL literal (`#"meter"`), its text. Distinct from [`as_name`] (an
    /// identifier) — a symbol is a `#"…"` name-value. Used to read a `Unit.define`/`Unit.of` family-unit
    /// name off its symbol argument.
    pub fn as_sym(&self, id: StructId) -> Option<&str> {
        match self.get(id) {
            Struct::Atom(l) => match self.leaf(*l) {
                Leaf::Sym(s) => Some(s),
                _ => None,
            },
            _ => None,
        }
    }

    /// If `id` is an `Atom` of an integer literal, its value. (Used to read an integer member key as a
    /// tuple position — `(. t 0)` — where a name key would be a record field instead.)
    pub fn as_int(&self, id: StructId) -> Option<&IntValue> {
        match self.get(id) {
            Struct::Atom(l) => match self.leaf(*l) {
                Leaf::Int { value, .. } => Some(value),
                _ => None,
            },
            _ => None,
        }
    }

    /// If `id` is an `Atom` of a decimal/float literal, its `Decimal`. The float analogue of [`as_int`],
    /// used by `default-fraction` literal-marking (an exact default grounds a written decimal `0.5` to
    /// `1/2`, so a decimal literal is marked alongside an integer one).
    pub fn as_float(&self, id: StructId) -> Option<&Decimal> {
        match self.get(id) {
            Struct::Atom(l) => match self.leaf(*l) {
                Leaf::Float(d) => Some(d),
                _ => None,
            },
            _ => None,
        }
    }

    /// If `id` is an `Atom` of a string literal, its contents.
    pub fn as_str(&self, id: StructId) -> Option<&str> {
        match self.get(id) {
            Struct::Atom(l) => match self.leaf(*l) {
                Leaf::Str(s) => Some(s),
                _ => None,
            },
            _ => None,
        }
    }

    /// The value of a boolean-literal `Atom` (`true`/`false`), if `id` is one. (A surface `true`/`false`
    /// is a `Leaf::Bool`, not a `Leaf::Name`, so `as_name` does not see it.)
    pub fn as_bool(&self, id: StructId) -> Option<bool> {
        match self.get(id) {
            Struct::Atom(l) => match self.leaf(*l) {
                Leaf::Bool(b) => Some(*b),
                _ => None,
            },
            _ => None,
        }
    }

    /// The head name of a `List` occurrence, if its first child is an `Atom(Name)`.
    pub fn head_name(&self, id: StructId) -> Option<&str> {
        match self.get(id) {
            Struct::List(items) => items.first().and_then(|&h| self.as_name(h)),
            _ => None,
        }
    }

    /// The head STRING-LITERAL of a `List` occurrence, if its first child is an `Atom(Str)`. A string
    /// in head position is a PRIMITIVE CONSTRUCTOR spelling — `("tuple" …)`, `("record" …)` — the
    /// unshadowable counterpart of the shadowable NAME head (`head_name`). A string is unspellable as an
    /// identifier, so a primitive named by one can never be shadowed by a binding; the ordinary names
    /// `tuple`/`record` are prelude ALIASES to these primitives (see `resolve`). This is why the two
    /// accessors are distinct: the resolver dispatches a string head structurally, but looks a name head
    /// up (so a bound `tuple` wins). ("The strings are the symbols" — no invented sigils.)
    pub fn head_ctor(&self, id: StructId) -> Option<&str> {
        match self.get(id) {
            Struct::List(items) => items.first().and_then(|&h| self.as_str(h)),
            _ => None,
        }
    }

    /// The primitive compound constructor [`CompoundCtor`] this node denotes, if any — the typed TAG
    /// read from the reserved STRING-LITERAL head. This is the single place the reserved compound
    /// vocabulary is matched for structural dispatch, so a consumer branches on the returned tag rather
    /// than re-comparing head text. Only the unshadowable STRING primitive is a tag: a NAME head
    /// (`(record …)`, the shadowable alias) returns `None` here and resolves lexically-first, so a
    /// program binding named `record`/`tuple`/`list`/`map` still shadows the alias. See
    /// `implementation/design/DESIGN-native-ast-compound-data.md`.
    pub fn compound_ctor(&self, id: StructId) -> Option<CompoundCtor> {
        CompoundCtor::from_spelling(self.head_ctor(id)?)
    }

    /// The compound constructor a `List` node denotes accepting EITHER head spelling — the unshadowable
    /// STRING primitive (`("list" …)`) OR the shadowable NAME alias (`(list …)`). This is the transitional
    /// DUAL-READ recognizer for consumers that already accept both spellings (the
    /// `as_ctor_form(…).or_else(as_form(…))` idiom); it deliberately does NOT distinguish a shadowed name
    /// binding, so use it only where a compound literal is already expected, NOT for structural dispatch
    /// (that is [`compound_ctor`], the primitive-only form the resolver uses). See
    /// `implementation/design/DESIGN-native-ast-compound-data.md` (M1 dual-read).
    pub fn compound_ctor_either(&self, id: StructId) -> Option<CompoundCtor> {
        match self.get(id) {
            Struct::List(items) => {
                let &h = items.first()?;
                let spelling = self.as_name(h).or_else(|| self.as_str(h))?;
                CompoundCtor::from_spelling(spelling)
            }
            _ => None,
        }
    }

    /// The child occurrences of `id` if it is a `List` headed by the compound ctor `want`, accepting
    /// EITHER head spelling (the transitional dual-read of [`compound_ctor_either`]) — the tag-typed twin
    /// of the `as_form(id, "…").or_else(|| as_ctor_form(id, "…"))` idiom for the four compound ctors.
    /// Like `as_form`/`as_ctor_form`, an empty tail (a head-only list) yields `Some(&[])`.
    pub fn compound_form_of(&self, id: StructId, want: CompoundCtor) -> Option<&[StructId]> {
        match self.get(id) {
            Struct::List(items) => {
                let &h = items.first()?;
                let spelling = self.as_name(h).or_else(|| self.as_str(h))?;
                (CompoundCtor::from_spelling(spelling) == Some(want)).then_some(&items[1..])
            }
            _ => None,
        }
    }

    /// The `(key, value)` of a canonical `(= key value)` FIELD PAIR node — a `List` of exactly three
    /// children whose head is the `=` FieldPair marker. This is the ONE reader for the shape shared by
    /// record fields and (once unified, per DESIGN-native-ast-compound-data) map entries, so a consumer
    /// extracts key/value through it instead of re-matching the triple. `None` for anything that is not a
    /// well-formed `(= k v)` triple (a caller distinguishes a malformed `=`-led or a legacy `(k v)` form
    /// itself). The read twin of [`Builder::field_pair`].
    pub fn field_pair(&self, id: StructId) -> Option<(StructId, StructId)> {
        match self.get(id) {
            Struct::List(kv) if kv.len() == 3 && self.as_name(kv[0]) == Some("=") => {
                Some((kv[1], kv[2]))
            }
            _ => None,
        }
    }

    /// If `id` is a `List` headed by the NAME `head`, the tail (the argument occurrences).
    pub fn as_form(&self, id: StructId, head: &str) -> Option<&[StructId]> {
        match self.get(id) {
            Struct::List(items) => match items.first() {
                Some(&h) if self.as_name(h) == Some(head) => Some(&items[1..]),
                _ => None,
            },
            _ => None,
        }
    }

    /// The declared TYPE NAME from a `(type …)` decl's FIRST tail element `head_occ` (the element after the
    /// `type` keyword). Two spellings: a BARE atom `(type Box …)` — the atom IS the name — OR a
    /// PARENTHESIZED head `(type (Box a b…) …)` — a `(Name params…)` list whose HEAD atom is the name.
    /// `None` if `head_occ` is neither (a malformed decl). The ONE place both spellings are decoded, so every
    /// raw `(type …)`-tail name-reader (the linker's export/import `top_item_defined_name`, the
    /// invariant-establish plan, the proptest `classify_sum`/`name_resolves_to_user_type`, and
    /// `scan_type_decl` itself) agrees — a bare `head_occ.as_name()` returns `None` for a `(List)` head, so
    /// without this a parenthesized-head generic type was INVISIBLE to those readers (un-exported / not a
    /// known user type). See [`crate::db::scan_type_decl`].
    pub fn type_decl_head_name(&self, head_occ: StructId) -> Option<&str> {
        match self.get(head_occ) {
            // Bare-atom name: `(type Box …)`.
            Struct::Atom(_) => self.as_name(head_occ),
            // Parenthesized `(Name params…)` head: the list's head atom is the name.
            Struct::List(kids) => kids.first().and_then(|&h| self.as_name(h)),
        }
    }

    /// If `id` is a `List` headed by the STRING-LITERAL `head` (a primitive constructor spelling like
    /// `"tuple"`/`"record"`), the tail (the argument occurrences). The string-head twin of [`as_form`].
    pub fn as_ctor_form(&self, id: StructId, head: &str) -> Option<&[StructId]> {
        match self.get(id) {
            Struct::List(items) => match items.first() {
                Some(&h) if self.as_str(h) == Some(head) => Some(&items[1..]),
                _ => None,
            },
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── IntValue arbitrary-precision arithmetic (the B4-1 Rational-fold foundation). Differential-test
    // every op against i128 over a spread of in-range values (both signs, zero, powers of two, primes),
    // then pin the OUT-of-i128 cases the whole point of bignum is to get right. ──────────────────────

    fn iv(v: i128) -> IntValue {
        if v >= 0 {
            IntValue::from_u128(v as u128)
        } else {
            IntValue::from_neg_u128((-v) as u128)
        }
    }

    #[test]
    fn bignum_arithmetic_matches_i128_over_a_spread() {
        let vals: [i128; 17] = [
            0,
            1,
            -1,
            2,
            -2,
            7,
            -7,
            10,
            255,
            256,
            -256,
            1000,
            -1000,
            65536,
            123456789,
            -987654321,
            1_000_000_000_000,
        ];
        for &a in &vals {
            for &b in &vals {
                let (ia, ib) = (iv(a), iv(b));
                assert_eq!(ia.add(&ib).to_i128(), Some(a + b), "add {a}+{b}");
                assert_eq!(ia.sub(&ib).to_i128(), Some(a - b), "sub {a}-{b}");
                assert_eq!(ia.mul(&ib).to_i128(), Some(a * b), "mul {a}*{b}");
                assert_eq!(ia.cmp(&ib), a.cmp(&b), "cmp {a} vs {b}");
                if b != 0 {
                    let (q, r) = ia.divmod(&ib).expect("nonzero divisor");
                    assert_eq!(q.to_i128(), Some(a / b), "div {a}/{b}");
                    assert_eq!(r.to_i128(), Some(a % b), "rem {a}%{b}");
                    // q*d + r == n
                    assert!(q.mul(&ib).add(&r).eq_value(&ia), "q*d+r == n for {a}/{b}");
                }
            }
        }
    }

    #[test]
    fn bignum_divmod_by_zero_is_none() {
        assert!(iv(5).divmod(&iv(0)).is_none());
        assert!(iv(0).divmod(&iv(0)).is_none());
    }

    #[test]
    fn bignum_gcd_matches_a_reference() {
        // gcd(a,b) over magnitudes (non-negative result).
        fn ref_gcd(mut a: u128, mut b: u128) -> u128 {
            while b != 0 {
                let t = a % b;
                a = b;
                b = t;
            }
            a
        }
        for a in [0u128, 1, 2, 6, 12, 18, 100, 255, 1024, 123456, 999983] {
            for b in [0u128, 1, 4, 8, 9, 24, 36, 255, 1000, 123456, 999983] {
                assert_eq!(
                    iv(a as i128).gcd(&iv(b as i128)).to_i128(),
                    Some(ref_gcd(a, b) as i128),
                    "gcd({a},{b})"
                );
            }
        }
        // GCD ignores sign (result is the non-negative common divisor).
        assert_eq!(iv(-12).gcd(&iv(18)).to_i128(), Some(6));
        assert_eq!(iv(-12).gcd(&iv(-18)).to_i128(), Some(6));
    }

    #[test]
    fn bignum_binary_gcd_matches_beyond_i128() {
        // `gcd_mag` is BINARY GCD (Stein's) — the O(bits²) hot compile-time path that reduces a folded
        // `Rational` to lowest terms. Pin it on values BEYOND i128 (where the differential-against-i128
        // test above cannot reach) and on the coprime/odd/power-of-two shapes Stein's special-cases.
        let two_64 = iv(1).add(&iv(u64::MAX as i128)); // 2^64
        let two_128 = two_64.mul(&two_64); // 2^128
        // Two large numbers sharing exactly 2^64: gcd(2^128, 3·2^64) == 2^64 (the common power of two × the
        // gcd of the odd cores 2^64 and 3, which is 1).
        let three_2_64 = two_64.mul(&iv(3));
        assert!(
            two_128.gcd(&three_2_64).eq_value(&two_64),
            "gcd(2^128, 3·2^64) == 2^64"
        );
        // Coprime large ODDs: gcd(2^128+1, 2^128-1) == 1 (consecutive odds differ by 2, share no odd factor).
        let big_odd_hi = two_128.add(&iv(1));
        let big_odd_lo = two_128.sub(&iv(1));
        assert!(
            big_odd_hi.gcd(&big_odd_lo).eq_value(&iv(1)),
            "gcd(2^128+1, 2^128-1) == 1 (coprime)"
        );
        // A large shared ODD factor: gcd(999983·2^64, 999983·3) == 999983 (a prime × its non-common cofactors).
        let p = iv(999983);
        let a = p.mul(&two_64);
        let b = p.mul(&iv(3));
        assert!(
            a.gcd(&b).eq_value(&p),
            "gcd(999983·2^64, 999983·3) == 999983"
        );
        // Identities across the zero/self boundary (Stein's early-outs).
        assert!(two_128.gcd(&iv(0)).eq_value(&two_128), "gcd(x,0)==x");
        assert!(iv(0).gcd(&two_128).eq_value(&two_128), "gcd(0,x)==x");
        assert!(two_128.gcd(&two_128).eq_value(&two_128), "gcd(x,x)==x");
    }

    #[test]
    fn bignum_handles_values_beyond_i128() {
        // 2^64 (does not fit i64) squared = 2^128 (does not fit i128) — the whole reason the type exists.
        let two_64 = iv(1).add(&iv(u64::MAX as i128)); // 2^64
        let two_128 = two_64.mul(&two_64); // 2^128
        assert_eq!(two_128.to_i128(), None, "2^128 exceeds i128");
        // Divide it back down: 2^128 / 2^64 = 2^64.
        let (q, r) = two_128.divmod(&two_64).expect("nonzero");
        assert!(q.eq_value(&two_64), "2^128 / 2^64 == 2^64");
        assert!(r.is_zero(), "exact division, zero remainder");
        // 2^128 - 1 is one less, and (2^128 - 1) % 2^64 == 2^64 - 1.
        let big = two_128.sub(&iv(1));
        let (_q2, r2) = big.divmod(&two_64).expect("nonzero");
        assert!(
            r2.eq_value(&two_64.sub(&iv(1))),
            "(2^128-1) mod 2^64 == 2^64-1"
        );
        // gcd of two large even numbers carries a factor of 2^64.
        let g = two_128.gcd(&two_64);
        assert!(g.eq_value(&two_64), "gcd(2^128, 2^64) == 2^64");
    }

    #[test]
    fn leaves_dedup_occurrences_do_not() {
        // (+ x x): two `x` occurrences share ONE leaf, but are distinct structure ids.
        let mut b = Builder::new();
        let plus = b.name("+");
        let x1 = b.name("x");
        let x2 = b.name("x");
        let root = b.list(vec![plus, x1, x2]);
        let a = b.finish(root);

        // Distinct occurrences.
        assert_ne!(x1, x2);
        // One interned leaf for "x" (plus one for "+").
        assert_eq!(a.leaves.len(), 2);
        // Both x occurrences resolve to the same leaf.
        let (Struct::Atom(l1), Struct::Atom(l2)) = (a.get(x1), a.get(x2)) else {
            panic!("expected atoms");
        };
        assert_eq!(l1, l2);
        assert_eq!(a.head_name(root), Some("+"));
        assert_eq!(a.as_form(root, "+").map(|t| t.len()), Some(2));
    }

    #[test]
    fn compound_ctor_tags_the_string_primitive_not_the_name_alias() {
        let mut b = Builder::new();
        // `("record" (= x 1))` — the unshadowable STRING primitive head is the tag.
        let rec_head = b.atom_leaf(Leaf::Str("record".into()));
        let one = b.atom_leaf(Leaf::Str("_".into())); // payload shape is irrelevant to the tag
        let rec = b.list(vec![rec_head, one]);
        // `(record …)` — the shadowable NAME alias is deliberately NOT a tag (resolves lexically).
        let alias_head = b.name("record");
        let alias = b.list(vec![alias_head]);
        // A non-list atom.
        let atom = b.name("x");
        // A string-headed but non-compound word.
        let other_head = b.atom_leaf(Leaf::Str("if".into()));
        let other = b.list(vec![other_head]);
        let a = b.finish(rec);

        assert_eq!(a.compound_ctor(rec), Some(CompoundCtor::Record));
        // The NAME alias is not a primitive tag — must be None (shadowability invariant).
        assert_eq!(a.compound_ctor(alias), None);
        // Non-list and non-compound string heads are not tags.
        assert_eq!(a.compound_ctor(atom), None);
        assert_eq!(a.compound_ctor(other), None);

        // All four primitive spellings map to their tag.
        for (spelling, want) in [
            ("record", CompoundCtor::Record),
            ("tuple", CompoundCtor::Tuple),
            ("list", CompoundCtor::List),
            ("map", CompoundCtor::Map),
        ] {
            let mut b = Builder::new();
            let h = b.atom_leaf(Leaf::Str(spelling.into()));
            let node = b.list(vec![h]);
            let a = b.finish(node);
            assert_eq!(a.compound_ctor(node), Some(want), "tag for `{spelling}`");
        }
    }

    #[test]
    fn compound_ctor_either_accepts_both_head_kinds() {
        // The transitional DUAL-READ recognizer: unlike `compound_ctor` (STRING primitive only), this
        // accepts EITHER the STRING primitive `("list" …)` OR the NAME alias `(list …)`.
        let mut b = Builder::new();
        let str_head = b.atom_leaf(Leaf::Str("list".into()));
        let str_list = b.list(vec![str_head]);
        let name_head = b.name("list");
        let name_list = b.list(vec![name_head]);
        let non_ctor = b.name("if");
        let non_ctor_form = b.list(vec![non_ctor]);
        let root = b.list(vec![str_list, name_list, non_ctor_form]);
        let a = b.finish(root);

        // Both spellings are recognized as List by the dual-read form...
        assert_eq!(a.compound_ctor_either(str_list), Some(CompoundCtor::List));
        assert_eq!(a.compound_ctor_either(name_list), Some(CompoundCtor::List));
        // ...but the primitive-only form still rejects the NAME alias (shadowing-safe).
        assert_eq!(a.compound_ctor(str_list), Some(CompoundCtor::List));
        assert_eq!(a.compound_ctor(name_list), None);
        // A non-ctor head is neither.
        assert_eq!(a.compound_ctor_either(non_ctor_form), None);
    }

    #[test]
    fn compound_form_of_returns_children_for_either_head_of_the_wanted_ctor() {
        let mut b = Builder::new();
        // `("tuple" a b)` — STRING head, two elements.
        let th = b.atom_leaf(Leaf::Str("tuple".into()));
        let a1 = b.name("a");
        let b1 = b.name("b");
        let str_tuple = b.list(vec![th, a1, b1]);
        // `(tuple c)` — NAME head, one element.
        let nh = b.name("tuple");
        let c1 = b.name("c");
        let name_tuple = b.list(vec![nh, c1]);
        // `(record …)` — a different ctor.
        let rh = b.name("record");
        let rec = b.list(vec![rh]);
        let root = b.list(vec![str_tuple, name_tuple, rec]);
        let a = b.finish(root);

        // Either head of the wanted ctor yields the tail (children)...
        assert_eq!(
            a.compound_form_of(str_tuple, CompoundCtor::Tuple)
                .map(|t| t.len()),
            Some(2)
        );
        assert_eq!(
            a.compound_form_of(name_tuple, CompoundCtor::Tuple)
                .map(|t| t.len()),
            Some(1)
        );
        // ...the wrong ctor yields None...
        assert_eq!(a.compound_form_of(rec, CompoundCtor::Tuple), None);
        assert_eq!(a.compound_form_of(str_tuple, CompoundCtor::Record), None);
        // ...and a record head is recognized as Record.
        assert_eq!(
            a.compound_form_of(rec, CompoundCtor::Record)
                .map(|t| t.len()),
            Some(0)
        );
    }

    #[test]
    fn field_pair_builds_and_reads_the_canonical_eq_kv_node() {
        let mut b = Builder::new();
        let k = b.name("x");
        let v = b.name("1");
        // Builder::field_pair emits `(= x 1)`; Arenas::field_pair reads it back.
        let fp = b.field_pair(k, v);
        // A non-field-pair node (a plain 2-elem `(x 1)` and a wrong-head `(tuple x 1)`) reads as None.
        let x2 = b.name("x");
        let one2 = b.name("1");
        let plain = b.list(vec![x2, one2]);
        let th = b.name("tuple");
        let x3 = b.name("x");
        let one3 = b.name("1");
        let non_eq = b.list(vec![th, x3, one3]);
        let root = b.list(vec![fp, plain, non_eq]);
        let a = b.finish(root);

        // Emit→read round-trips to the same key/value occurrences.
        assert_eq!(a.field_pair(fp), Some((k, v)));
        // The head is the `=` FieldPair marker, three children.
        assert!(
            matches!(a.get(fp), Struct::List(kv) if kv.len() == 3 && a.as_name(kv[0]) == Some("="))
        );
        // A legacy plain `(x 1)` pair and a `(tuple …)` are NOT field pairs.
        assert_eq!(a.field_pair(plain), None);
        assert_eq!(a.field_pair(non_eq), None);
    }

    #[test]
    fn eq_value_ignores_magnitude_representation() {
        // The load-bearing correctness property for match probes: zero has two representations —
        // `zero()` (empty magnitude) and a literal `0` (`[0]`) — and they must compare EQUAL by value
        // (struct `==` does NOT). A folded `(% 4 2)` = empty-magnitude zero must match a literal `0`.
        let empty_zero = IntValue::zero();
        let byte_zero = IntValue {
            negative: false,
            magnitude: vec![0],
        };
        assert_ne!(
            empty_zero, byte_zero,
            "struct == distinguishes the representations"
        );
        assert!(
            empty_zero.eq_value(&byte_zero),
            "eq_value compares by value"
        );
        // Leading zeros are ignored; genuine values still compare correctly.
        assert!(IntValue::from_i64(5).eq_value(&IntValue {
            negative: false,
            magnitude: vec![0, 0, 5],
        }));
        assert!(!IntValue::from_i64(5).eq_value(&IntValue::from_i64(6)));
        // Sign matters for non-zero, not for zero.
        assert!(!IntValue::from_i64(-5).eq_value(&IntValue::from_i64(5)));
        assert!(empty_zero.eq_value(&IntValue {
            negative: true,
            magnitude: vec![],
        }));
    }

    #[test]
    fn decimal_to_f64_bits_converts_base256_significand_and_distinguishes_signed_zero() {
        // `to_f64_bits` reconstructs the exact decimal (base-256 significand → base-10 digits) and rounds
        // to the nearest double. Pins the base-256→base-10 conversion (the load-bearing arithmetic) + the
        // canonical-bits contract (`-0.0` ≠ `0.0`, a value equals its own spelling).
        let dec = |neg: bool, sig: Vec<u8>, exp: i64| Decimal {
            negative: neg,
            significand: sig,
            exponent: exp,
        };
        // 3.5 = 35 × 10^-1; 35 = 0x23 = one byte [35].
        assert_eq!(dec(false, vec![35], -1).to_f64_bits(), 3.5f64.to_bits());
        // 1e19 = 1 × 10^19 — a value beyond i64, must NOT saturate; equals the plain double.
        assert_eq!(dec(false, vec![1], 19).to_f64_bits(), 1e19f64.to_bits());
        // 256 × 10^0 — exercises a MULTI-byte base-256 significand ([1,0] = 256).
        assert_eq!(dec(false, vec![1, 0], 0).to_f64_bits(), 256.0f64.to_bits());
        // Signed zero: empty significand is 0; the `negative` flag distinguishes -0.0 from 0.0 by BITS.
        assert_eq!(dec(true, vec![], 0).to_f64_bits(), (-0.0f64).to_bits());
        assert_eq!(dec(false, vec![], 0).to_f64_bits(), (0.0f64).to_bits());
        assert_ne!(
            dec(true, vec![], 0).to_f64_bits(),
            dec(false, vec![], 0).to_f64_bits(),
            "-0.0 and 0.0 have distinct canonical bits"
        );
    }

    #[test]
    fn wrap_to_truncates_and_reinterprets() {
        let iv = |n: i64| IntValue::from_i64(n);
        // In range: unchanged.
        assert_eq!(iv(200).wrap_to(false, 8), IntValue::from_u128(200)); // UInt8
        assert_eq!(iv(-1).wrap_to(true, 8), iv(-1)); // Int8: -1 stays -1
        // Out of range unsigned: keep low 8 bits.
        assert_eq!(iv(256).wrap_to(false, 8), IntValue::zero()); // 0x100 → 0
        assert_eq!(iv(511).wrap_to(false, 8), IntValue::from_u128(255)); // 0x1FF → 0xFF
        // Negative into unsigned: two's-complement low bits.
        assert_eq!(iv(-1).wrap_to(false, 8), IntValue::from_u128(255)); // all ones
        assert_eq!(iv(-256).wrap_to(false, 8), IntValue::zero()); // 0x...F00 → 0
        // Into signed: the target's sign bit decides. 200 = 0xC8, bit7 set → -56 as Int8.
        assert_eq!(iv(200).wrap_to(true, 8), iv(-56));
        assert_eq!(iv(128).wrap_to(true, 8), iv(-128)); // Int8.min
        // A wide value truncated to 48 bits (a non-aliased internal width): -1 → 2^48-1.
        assert_eq!(
            iv(-1).wrap_to(false, 48),
            IntValue::from_u128((1u128 << 48) - 1)
        );
        // Width 64: -1 → UInt64.max (2^64-1).
        assert_eq!(
            iv(-1).wrap_to(false, 64),
            IntValue::from_u128(u64::MAX as u128)
        );
    }
}
