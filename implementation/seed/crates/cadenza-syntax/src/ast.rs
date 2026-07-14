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

use num_bigint::BigInt;

/// A leaf primitive value. The value kinds plus one MARKER (`BadEscape`) the reader emits for a
/// lexically-malformed literal it cannot itself report.
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
        value: BigInt,
        radix: Radix,
    },
    Float(Decimal),
    Str(String),
    /// A CHAR literal (`#\a`, `#\newline`, `#\u+00E9`) — a single Unicode scalar value, the element type
    /// of a string's scalar sequence (`collections-and-text.md` §A Char Is A Single Unicode Scalar
    /// Value). A `char` is a scalar by construction (Rust `char` excludes the surrogate range), so this
    /// only ever holds a valid scalar; a literal spelling a NON-scalar (`#\u+D800`) is the `BadChar`
    /// marker instead. Printed `#\c` for a printable char, `#\u+HHHH` for a control/non-printable one.
    Char(char),
    /// A BYTE SEQUENCE literal (`b"…"`) — the value form of a `Bytes`. Holds the raw bytes (arbitrary,
    /// NOT necessarily UTF-8, so distinct from `Str`); printed `b"…"` (printable ASCII raw, `\n \r \t \\
    /// \"` named, else `\xNN`). The canonical value-form leaf a byte sequence crosses the boundary as.
    Bytes(Vec<u8>),
    Bool(bool),
    /// A SYMBOL literal (`#"metre"`) — an interned name value whose identity is its CONTENT, distinct
    /// from a `Str` (a text value) and a `Name` (an identifier reference). Written `#"…"` (reusing string
    /// lexing/escapes), it names a symbol whose only observations are equality and `to-string`
    /// (`symbol-interning-direction`; `options/symbol-interning/`). Holds the symbol's text. Printed back
    /// `#"…"` so it round-trips. In the units-of-measure layer a base dimension is named by such a symbol
    /// (`(Unit.base #"metre")`).
    Sym(String),
    /// An identifier: a name reference, a construct head, a variant, or a qualified name segment.
    Name(String),
    /// A string literal carrying an UNRECOGNIZED ESCAPE (`"\q"`) — a lexical well-formedness defect the
    /// reader detected but does not itself report (its stderr is not the diagnostic surface). The reader
    /// emits this MARKER instead of silently reading `\q` as the bare `q`; it survives the binary codec so
    /// the COMPILER rejects it (CDZ0001, `collections-and-text.md` §A String Literal's Escapes Are A Closed
    /// Set). Holds the offending escape character (for the diagnostic message).
    BadEscape(char),
    /// A CHAR literal that names a NON-scalar code point (`#\u+D800`, a surrogate) or is otherwise
    /// malformed — a lexical defect the reader detected but cannot itself report, so it rides the binary
    /// AST as a MARKER (like `BadEscape`). Resolving it is a `CDZ0002` rejection (`collections-and-text.md`
    /// §A Char Is A Single Unicode Scalar Value): a `char` cannot hold a surrogate, so the reader records
    /// the offending spelling here rather than fabricating an invalid scalar. Holds the literal's text.
    BadChar(String),
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
/// The significand is an arbitrary-precision non-negative magnitude; the sign lives in `negative`
/// so that `-0.0` (negative, zero significand) is preserved distinctly from `0.0`. This captures a
/// source float literal EXACTLY (no `f64` rounding), so a later type-directed rounding to a chosen
/// width happens once, from the exact value.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Decimal {
    pub negative: bool,
    pub significand: BigInt,
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
    // FxHash (not SipHash): the dedup key is the program's own leaf (a short identifier or literal),
    // never untrusted input, and `leaf` runs once per token during parse — SipHash's `hash_one` was
    // ~a quarter of front-end time. See `crate::fxhash`.
    leaf_index: crate::fxhash::FxHashMap<Leaf, LeafId>,
    // A SEPARATE dedup index for NAME leaves, keyed by the name STRING. `Name` is by far the most
    // common leaf (every identifier + construct head + qualified segment), and each occurrence arrives
    // as a `&str` slice of the source. Keying by `String` lets `leaf_name` look it up with a `&str`
    // (`String: Borrow<str>`) and allocate the owned `String` ONLY on a genuine cache miss — so a
    // repeated name (the norm in real code) costs zero allocation, instead of the old path that built a
    // `Leaf::Name(text.to_string())` for EVERY occurrence and discarded it on a dedup hit.
    name_index: crate::fxhash::FxHashMap<String, LeafId>,
    structure: Vec<Struct>,
}

impl Builder {
    pub fn new() -> Builder {
        Builder::default()
    }

    /// Intern a leaf, returning its (possibly pre-existing) id. A `Name` leaf is deduped through the
    /// by-string `name_index` (so an already-interned name reuses its id without touching the general
    /// index); every other leaf kind uses the general `leaf_index`.
    pub fn leaf(&mut self, leaf: Leaf) -> LeafId {
        if let Leaf::Name(name) = leaf {
            return self.leaf_name(&name);
        }
        if let Some(&id) = self.leaf_index.get(&leaf) {
            return id;
        }
        let id = LeafId(self.leaves.len() as u32);
        self.leaves.push(leaf.clone());
        self.leaf_index.insert(leaf, id);
        id
    }

    /// Intern a NAME leaf given its string SLICE, returning its (possibly pre-existing) id. Allocates
    /// an owned `String` ONLY on a cache miss — a repeated name (the common case) is a pure `&str`
    /// lookup with no allocation. This is the hot interning path (every identifier occurrence).
    pub fn leaf_name(&mut self, name: &str) -> LeafId {
        if let Some(&id) = self.name_index.get(name) {
            return id;
        }
        let id = LeafId(self.leaves.len() as u32);
        self.leaves.push(Leaf::Name(name.to_string()));
        self.name_index.insert(name.to_string(), id);
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

    /// Convenience: an atom occurrence of a `Name` given its string SLICE. The hot path — interns via
    /// `leaf_name` (no allocation on a dedup hit) and pushes the occurrence.
    pub fn name(&mut self, name: &str) -> StructId {
        let id = self.leaf_name(name);
        self.atom(id)
    }

    fn push(&mut self, s: Struct) -> StructId {
        let id = StructId(self.structure.len() as u32);
        self.structure.push(s);
        id
    }

    /// The number of structure occurrences pushed so far — i.e. the next `StructId`'s index. A
    /// span-tracking reader uses this to keep a parallel `SpanTable` exactly 1:1 with the arena.
    pub fn structure_len(&self) -> usize {
        self.structure.len()
    }

    /// The structure entry at `id` — read-only access to an already-pushed occurrence, so a caller can
    /// inspect a node it just built (e.g. the parser flattening a top-level `(do …)`). Mirrors
    /// [`Arenas::get`]; the builder is append-only, so any `id` from a prior push stays valid.
    pub fn get(&self, id: StructId) -> &Struct {
        &self.structure[id.0 as usize]
    }

    /// If `id` is a `List` whose head is the NAME `head`, its tail (the children after the head) —
    /// mirrors [`Arenas::as_form`], for inspecting a just-built node during parse.
    pub fn as_form(&self, id: StructId, head: &str) -> Option<&[StructId]> {
        match self.get(id) {
            Struct::List(items) => match items.first() {
                Some(&h) if self.head_leaf_is(h, head) => Some(&items[1..]),
                _ => None,
            },
            _ => None,
        }
    }

    /// True if `id` is an `Atom` of the NAME leaf `name`.
    fn head_leaf_is(&self, id: StructId, name: &str) -> bool {
        matches!(self.get(id), Struct::Atom(l) if matches!(&self.leaves[l.0 as usize], Leaf::Name(n) if n == name))
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

    /// The contents of a string-literal `Atom`, if `id` is one.
    pub fn as_str(&self, id: StructId) -> Option<&str> {
        match self.get(id) {
            Struct::Atom(l) => match self.leaf(*l) {
                Leaf::Str(s) => Some(s),
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

    /// The head STRING-LITERAL of a `List` occurrence, if its first child is an `Atom(Str)` — the
    /// compound-value CONSTRUCTOR primitive spelling (`"list"`/`"tuple"`/`"record"`/`"map"`). A string
    /// head is the unshadowable primitive a surface literal desugars to; the pretty-printer round-trips
    /// it back to the literal, distinct from a NAME head of the same spelling (an ordinary application).
    pub fn head_ctor(&self, id: StructId) -> Option<&str> {
        match self.get(id) {
            Struct::List(items) => items.first().and_then(|&h| self.as_str(h)),
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

    /// If `id` is a `List` headed by the STRING-LITERAL `head` (a constructor primitive), the tail.
    pub fn as_ctor_form(&self, id: StructId, head: &str) -> Option<&[StructId]> {
        match self.get(id) {
            Struct::List(items) => match items.first() {
                Some(&h) if self.as_str(h) == Some(head) => Some(&items[1..]),
                _ => None,
            },
            _ => None,
        }
    }

    /// Structural (denotational) equality with another arena: do the two `root`s denote the same
    /// tree of leaves? This is the right comparison for round-trips — the raw `Arenas` fields differ
    /// after a round-trip (leaf interning order, occurrence numbering) even when the programs are
    /// identical, so `derive(PartialEq)` is too strict. Canonical form (`canon`) is the alternative,
    /// but this direct walk needs no rewrite.
    pub fn structurally_eq(&self, other: &Arenas) -> bool {
        self.node_eq(self.root, other, other.root)
    }

    fn node_eq(&self, a: StructId, other: &Arenas, b: StructId) -> bool {
        match (self.get(a), other.get(b)) {
            (Struct::Atom(la), Struct::Atom(lb)) => self.leaf(*la) == other.leaf(*lb),
            (Struct::List(xs), Struct::List(ys)) => {
                if xs.len() != ys.len() {
                    return false;
                }
                // In HEAD position, a compound ctor's shadowable NAME alias and its unshadowable
                // STRING primitive denote the same construct (they compile identically). The pretty-
                // printer sugars an unshadowed name-headed `(record …)`/`(tuple …)`/`(list …)`/`(map …)`
                // to a literal, which the reader re-reads with a STRING head — so a name-headed input
                // still round-trips. Normalize the two head kinds here, but ONLY for the four ctors and
                // ONLY in head position, so a bare `list` name and the string value `"list"` elsewhere
                // stay distinct.
                if let (Some(&xh), Some(&yh)) = (xs.first(), ys.first()) {
                    let heads_eq = match (self.ctor_head_key(xh), other.ctor_head_key(yh)) {
                        (Some(x), Some(y)) => x == y,
                        _ => self.node_eq(xh, other, yh),
                    };
                    return heads_eq
                        && xs[1..]
                            .iter()
                            .zip(&ys[1..])
                            .all(|(&x, &y)| self.node_eq(x, other, y));
                }
                true // both empty (equal lengths, no head)
            }
            _ => false,
        }
    }

    /// The compound-ctor spelling an occurrence denotes as a LIST HEAD, collapsing the shadowable
    /// NAME alias and the unshadowable STRING primitive to one key — so head-kind normalization in
    /// [`node_eq`] can treat `Name("record")` and `Str("record")` as the same head. Only the four
    /// compound ctors qualify; every other name/string is left to exact leaf comparison.
    fn ctor_head_key(&self, id: StructId) -> Option<&str> {
        let spelling = match self.get(id) {
            Struct::Atom(l) => match self.leaf(*l) {
                Leaf::Name(n) => n.as_str(),
                Leaf::Str(s) => s.as_str(),
                _ => return None,
            },
            _ => return None,
        };
        matches!(spelling, "list" | "tuple" | "record" | "map").then_some(spelling)
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
