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

use std::collections::HashMap;

/// A leaf primitive value. Frozen at 5 variants.
///
/// `Int` is arbitrary-precision and `Float` is an exact width-free decimal: a literal's magnitude
/// or precision is never a well-formedness ceiling, and the concrete machine width (`Int64`,
/// `(Int N)`, `f32`, `f64`, …) is a *type* decision made downstream, not a representation choice
/// made here. `nan`/`inf`/`-inf` are ordinary `Name`s, so a `Float` only ever holds a finite value.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Leaf {
    /// An integer literal: its exact value plus the base its text used. The base is display-only
    /// (`42`, `0x2A`, `0b101010` are the same value) but is recorded so the printed form re-reads to
    /// the same leaf — a faithful text round-trip. Digit-separator (`_`) positions are NOT recorded.
    Int {
        value: IntValue,
        radix: Radix,
    },
    Float(Decimal),
    Str(String),
    Bool(bool),
    /// An identifier: a name reference, a construct head, a variant, or a qualified name segment.
    Name(String),
}

/// An arbitrary-precision integer value: a sign plus a big-endian magnitude. This is the whole of
/// what the encoding needs — a sign and a vector of bytes — so there is deliberately NO bignum
/// library behind it. The AST only CARRIES the value; arithmetic on an integer literal is a later
/// compile-time-evaluation concern that will operate on these bytes directly. Arbitrary precision
/// with nothing to depend on. The concrete machine width a literal takes is a downstream type
/// decision, not fixed here.
///
/// Canonical invariant for a value built through [`IntValue::from_i64`] / [`IntValue::zero`]: the
/// magnitude carries no leading zero bytes and is empty iff the value is zero, so equal values share
/// one representation (and one leaf-pool entry). A magnitude read off the wire is stored verbatim so
/// that `decode` is a faithful inverse of `encode`.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
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
}

/// The base an integer literal's text used. Display-only — it does not change the value.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
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
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Decimal {
    pub negative: bool,
    /// Big-endian non-negative magnitude of the significand. Empty represents zero.
    pub significand: Vec<u8>,
    /// Base-10 exponent.
    pub exponent: i64,
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
    leaf_index: HashMap<Leaf, LeafId>,
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
    pub fn name(&mut self, name: impl Into<String>) -> StructId {
        self.atom_leaf(Leaf::Name(name.into()))
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

    /// The head name of a `List` occurrence, if its first child is an `Atom(Name)`.
    pub fn head_name(&self, id: StructId) -> Option<&str> {
        match self.get(id) {
            Struct::List(items) => items.first().and_then(|&h| self.as_name(h)),
            _ => None,
        }
    }

    /// If `id` is a `List` headed by the name `head`, the tail (the argument occurrences).
    pub fn as_form(&self, id: StructId, head: &str) -> Option<&[StructId]> {
        match self.get(id) {
            Struct::List(items) => match items.first() {
                Some(&h) if self.as_name(h) == Some(head) => Some(&items[1..]),
                _ => None,
            },
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
