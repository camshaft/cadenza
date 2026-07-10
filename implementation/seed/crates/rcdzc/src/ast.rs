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
    Int { value: IntValue, radix: Radix },
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
        IntValue { negative: false, magnitude: Vec::new() }
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
        IntValue { negative: v < 0, magnitude: bytes[start..].to_vec() }
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
            Some((acc as i128 * -1) as i64)
        } else {
            if acc > i64::MAX as u128 {
                return None;
            }
            Some(acc as i64)
        }
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
        Arenas { leaves: self.leaves, structure: self.structure, root }
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
}
