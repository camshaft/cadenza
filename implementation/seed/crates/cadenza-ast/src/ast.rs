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
use std::sync::Arc;

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
    Str(Arc<str>),
    /// A CHAR literal (`#\a`, `#\newline`, `#\u+00E9`) — a single Unicode scalar value, the element type
    /// of a string's scalar sequence (`collections-and-text.md` §A Char Is A Single Unicode Scalar
    /// Value). A `char` is a scalar by construction (Rust `char` excludes the surrogate range), so this
    /// only ever holds a valid scalar; a literal spelling a NON-scalar (`#\u+D800`) is the `BadChar`
    /// marker instead. Printed `#\c` for a printable char, `#\u+HHHH` for a control/non-printable one.
    Char(char),
    /// A BYTE SEQUENCE literal (`b"…"`) — the value form of a `Bytes`. Holds the raw bytes (arbitrary,
    /// NOT necessarily UTF-8, so distinct from `Str`); printed `b"…"` (printable ASCII raw, `\n \r \t \\
    /// \"` named, else `\xNN`). The canonical value-form leaf a byte sequence crosses the boundary as.
    Bytes(Arc<[u8]>),
    Bool(bool),
    /// A SYMBOL literal (`#"meter"`) — an interned name value whose identity is its CONTENT, distinct
    /// from a `Str` (a text value) and a `Name` (an identifier reference). Written `#"…"` (reusing string
    /// lexing/escapes), it names a symbol whose only observations are equality and `to-string`
    /// (`symbol-interning-direction`; `options/symbol-interning/`). Holds the symbol's text. Printed back
    /// `#"…"` so it round-trips. In the units-of-measure layer a base dimension is named by such a symbol
    /// (`(Unit.base #"meter")`).
    Sym(Arc<str>),
    /// An identifier: a name reference, a construct head, a variant, or a qualified name segment.
    Name(Arc<str>),
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
    BadChar(Arc<str>),
    /// A numeric literal carrying an explicit TYPE SUFFIX (`100N`, `0.5R`) — the Rust-style opt-in that
    /// selects an unbounded/exact numeric type per-literal instead of the fixed-width default. `N`
    /// selects `BigInt`, `R` selects `Rational`; the body is an ordinary integer or float literal
    /// (`0xFFN`, `1_000N`, `5R`, `1.25R`, `12e2R`). The reader DESUGARS a suffixed atom to the
    /// annotation `(: <this-leaf> BigInt|Rational)` so all typing/grounding reuses the annotation path
    /// (a suffix IS a terse annotation) — and the compiler's codec decodes this leaf straight to a
    /// plain `Int`/`Float`, so the compiler never needs a distinct variant. This leaf exists on the
    /// SYNTAX side purely so the PRINTER re-emits the suffix (`100N`, not `(: 100 BigInt)`): its printed
    /// form is DISTINCT from a value-output annotation over a bare literal (which prints `(: 100
    /// BigInt)`), which is why a self-describing marker leaf — not a bare `Int` — is required. Holds the
    /// body value and which type the suffix selects.
    Suffixed {
        value: SuffixBody,
        kind: SuffixKind,
    },
}

/// The numeric body a type suffix decorates — an exact integer (with its display radix) or an exact
/// width-free decimal, the same two shapes the bare `Int`/`Float` leaves carry.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum SuffixBody {
    Int { value: BigInt, radix: Radix },
    Float(Decimal),
}

/// The type a numeric literal suffix selects: `N` → `BigInt` (unbounded integer), `R` → `Rational`
/// (exact rational). A closed set — the lexer accepts only these two suffix letters.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SuffixKind {
    /// `N` — an arbitrary-precision `BigInt`.
    BigInt,
    /// `R` — an exact `Rational`.
    Rational,
}

impl SuffixKind {
    /// The suffix character (`N`/`R`) this kind renders with — the dual of the lexer's suffix scan, so
    /// a suffixed leaf round-trips to text that re-reads to the same leaf.
    pub fn suffix_char(self) -> char {
        match self {
            SuffixKind::BigInt => 'N',
            SuffixKind::Rational => 'R',
        }
    }

    /// The annotation TYPE NAME (`BigInt`/`Rational`) a suffix desugars to — the type in the
    /// `(: <literal> <type>)` form the reader builds so typing reuses the annotation-grounding path.
    pub fn type_name(self) -> &'static str {
        match self {
            SuffixKind::BigInt => "BigInt",
            SuffixKind::Rational => "Rational",
        }
    }

    /// Classify a single trailing suffix character into its kind, or `None` if it is not a suffix
    /// letter. The lexer/classifier's suffix set is exactly `{N, R}` — CASE-SENSITIVE (lowercase `n`/`r`
    /// is not a suffix, keeping one canonical spelling).
    pub fn from_char(c: char) -> Option<SuffixKind> {
        match c {
            'N' => Some(SuffixKind::BigInt),
            'R' => Some(SuffixKind::Rational),
            _ => None,
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
    // `Leaf::Name(text.into())` for EVERY occurrence and discarded it on a dedup hit.
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
    ///
    /// A name is NFC-NORMALIZED before it becomes the dedup KEY, so two Unicode-canonically-equal
    /// spellings (`café` precomposed U+00E9 vs decomposed `e`+U+0301) intern to the SAME leaf — otherwise
    /// they were distinct `Leaf::Name`s and a decomposed reference failed to resolve against a precomposed
    /// definition (silent CDZ0101 unbound; concierge-ruled 2026-07-21 to normalize, mirroring the
    /// string-literal/symbol parse-path NFC). Normalization MUST precede the `name_index` lookup or the
    /// dedup itself would not unify the two spellings. HOT-PATH GUARD: an ASCII name (the overwhelming
    /// majority) is ALWAYS already NFC, so `is_ascii()` — one cheap byte scan, no allocation — short-circuits
    /// to the original zero-alloc `&str` dedup path; only a non-ASCII name pays the `is_nfc`/`.nfc()` cost.
    pub fn leaf_name(&mut self, name: &str) -> LeafId {
        // ASCII fast path: ASCII is always NFC, so a pure-ASCII name (the common case) keeps the original
        // allocation-free `&str` dedup — no normalization work.
        if name.is_ascii() {
            return self.leaf_name_normalized(name);
        }
        // Non-ASCII: normalize to NFC first so canonically-equal spellings share a key. `is_nfc_quick`
        // avoids the `.nfc()` allocation when the name is already normalized (the usual case).
        use unicode_normalization::{IsNormalized, UnicodeNormalization, is_nfc_quick};
        match is_nfc_quick(name.chars()) {
            IsNormalized::Yes => self.leaf_name_normalized(name),
            _ => {
                let normalized: String = name.nfc().collect();
                self.leaf_name_normalized(&normalized)
            }
        }
    }

    /// The core intern — `name` is ALREADY NFC (an ASCII name, or the caller normalized it). Allocates
    /// only on a dedup MISS. Split out so the NFC guard in [`leaf_name`] runs exactly once per call.
    fn leaf_name_normalized(&mut self, name: &str) -> LeafId {
        if let Some(&id) = self.name_index.get(name) {
            return id;
        }
        let id = LeafId(self.leaves.len() as u32);
        self.leaves.push(Leaf::Name(Arc::from(name)));
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
        matches!(self.get(id), Struct::Atom(l) if matches!(&self.leaves[l.0 as usize], Leaf::Name(n) if &**n == name))
    }

    /// If `id` is an `Atom` of a `Name`, that name — for inspecting a just-built node during parse
    /// (the [`Arenas::as_name`] analogue on the in-progress builder).
    pub fn as_name(&self, id: StructId) -> Option<&str> {
        match self.get(id) {
            Struct::Atom(l) => match &self.leaves[l.0 as usize] {
                Leaf::Name(n) => Some(n),
                _ => None,
            },
            _ => None,
        }
    }

    /// Build the CANONICAL effect-schema tree `(effect Name (op OpName Sig)… (authz Authz)?)` and return
    /// its root — the single constructor for the shape whose `Hash::of(codec::encode(root))` is the
    /// effect-schema identity (DESIGN-userspace-effects; the wire key a resolver maps to a schema AST, its
    /// declared name read back by [`Arenas::schema_declared_name`]). cadenza-ast OWNS this shape; a caller
    /// (the kernel's `ast_marshal::build_type`) emits each op's type-SIGNATURE node INTO this same
    /// `Builder` and hands its `StructId` in via `ops`, so there is ONE structural-encode path (these
    /// nodes → this tree → `codec::encode` → `Hash`), never a parallel encoder.
    ///
    /// HEAD-KIND is FIXED here so two identical schemas hash-BYTE-identically (identity is byte-exact
    /// `Hash::of(encode)`, and the codec emits distinct bytes for a `Name` vs `Str` head of the same
    /// spelling — even though [`Arenas::structurally_eq`] normalizes the four compound-ctor heads): the
    /// effect-tree STRUCTURE heads (`effect`/`op`/`authz`) are NAME atoms (matching how
    /// `schema_declared_name` reads the name via `as_name`); the per-op signature nodes keep whatever heads
    /// their emitter chose (the kernel type descriptors are string-head `("record" …)`/`("list" …)`/… — a
    /// distinct, consistent layer this builder does not touch). By centering the wrapper here, a caller
    /// cannot drift the structural head-kind and split the identity of an otherwise-identical schema.
    ///
    /// `ops` is `(op_name, signature_node)` in the caller's order (op order is significant to the hash —
    /// the caller sorts if it wants order-independent identity). `authz`, when `Some`, is wrapped as a
    /// trailing `(authz <node>)`; `None` omits the slot entirely.
    pub fn effect_schema_tree(
        &mut self,
        name: &str,
        ops: &[(&str, StructId)],
        authz: Option<StructId>,
    ) -> StructId {
        let mut children = Vec::with_capacity(2 + ops.len() + authz.is_some() as usize);
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
        if let Some(a) = authz {
            let authz_head = self.name("authz");
            let authz_node = self.list(vec![authz_head, a]);
            children.push(authz_node);
        }
        self.list(children)
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

    /// The contents of a symbol-literal `Atom` (`#"json"` → `"json"`), if `id` is one. Distinct from
    /// [`as_name`] (an identifier) — a symbol is a `#"…"` name-value, e.g. the grammar tag of an
    /// `(embedded #<grammar> …)` node.
    pub fn as_sym(&self, id: StructId) -> Option<&str> {
        match self.get(id) {
            Struct::Atom(l) => match self.leaf(*l) {
                Leaf::Sym(s) => Some(s),
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

    /// The DECLARED NAME of an effect-schema AST: the `Name` head of a root `(effect Name (op …) …)`
    /// form — e.g. `Weather` for `(effect Weather (op get (-> Unit Reading)))`. `None` if the root is
    /// not an `(effect …)` form or its name slot is absent/not a name.
    ///
    /// This is the stable family/name that effect routing and authorization key on, resolved from a
    /// decoded schema AST alongside its content hash (`Hash::of(encode(&schema_ast))`): identity by
    /// hash, name by this reader (DESIGN-userspace-effects, envelope D14 — the schema-hash is the wire
    /// key, the resolver maps hash → schema AST → this declared name). Reading the head out is real
    /// extraction over the arena, not an alias — an `(effect …)`-shape check plus the name projection.
    pub fn schema_declared_name(&self) -> Option<&str> {
        let tail = self.as_form(self.root, "effect")?;
        self.as_name(*tail.first()?)
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
        // An EXPLICIT stack of `(self-id, other-id)` pairs to compare, not native recursion: an arena can
        // originate POST-DECODE, and `codec::decode` accepts arbitrarily-deep valid-tree arenas (no cap,
        // unlike the reader's `MAX_NESTING_DEPTH`), so a recursive parallel walk overflowed the native
        // stack on a deep tree. Every pair must be structurally equal; the FIRST mismatch short-circuits
        // to `false`. Order of comparison does not affect the boolean result, so a plain LIFO stack is
        // fine (no need to preserve left-to-right).
        let mut stack: Vec<(StructId, StructId)> = vec![(a, b)];
        while let Some((a, b)) = stack.pop() {
            match (self.get(a), other.get(b)) {
                (Struct::Atom(la), Struct::Atom(lb)) => {
                    if self.leaf(*la) != other.leaf(*lb) {
                        return false;
                    }
                }
                (Struct::List(xs), Struct::List(ys)) => {
                    if xs.len() != ys.len() {
                        return false;
                    }
                    // In HEAD position, a compound ctor's shadowable NAME alias and its unshadowable
                    // STRING primitive denote the same construct (they compile identically). The pretty-
                    // printer sugars an unshadowed name-headed `(record …)`/`(tuple …)`/`(list …)`/`(map
                    // …)` to a literal, which the reader re-reads with a STRING head — so a name-headed
                    // input still round-trips. Normalize the two head kinds here, but ONLY for the four
                    // ctors and ONLY in head position, so a bare `list` name and the string value `"list"`
                    // elsewhere stay distinct.
                    if let (Some(&xh), Some(&yh)) = (xs.first(), ys.first()) {
                        match (self.ctor_head_key(xh), other.ctor_head_key(yh)) {
                            // Both are compound-ctor heads: compare the collapsed key inline (do NOT
                            // descend into the head — a `Name`/`Str` head-kind difference is normalized).
                            (Some(x), Some(y)) => {
                                if x != y {
                                    return false;
                                }
                            }
                            // Otherwise the head is an ordinary pair to compare structurally.
                            _ => stack.push((xh, yh)),
                        }
                        // The remaining children are ordinary pairs.
                        for (&x, &y) in xs[1..].iter().zip(&ys[1..]) {
                            stack.push((x, y));
                        }
                    }
                    // (both empty — equal lengths, no head — is trivially equal: push nothing)
                }
                _ => return false,
            }
        }
        true
    }

    /// The compound-ctor spelling an occurrence denotes as a LIST HEAD, collapsing the shadowable
    /// NAME alias and the unshadowable STRING primitive to one key — so head-kind normalization in
    /// [`node_eq`] can treat `Name("record")` and `Str("record")` as the same head. Only the four
    /// compound ctors qualify; every other name/string is left to exact leaf comparison.
    fn ctor_head_key(&self, id: StructId) -> Option<&str> {
        let spelling: &str = match self.get(id) {
            Struct::Atom(l) => match self.leaf(*l) {
                Leaf::Name(n) => n,
                Leaf::Str(s) => s,
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
    fn suffix_kind_char_is_a_case_sensitive_bijection_with_from_char() {
        // `suffix_char` (kind → letter) and `from_char` (letter → kind) are duals — the printer renders
        // a suffixed leaf with `suffix_char`, and the lexer re-reads that letter with `from_char`, so a
        // suffixed literal round-trips to text that re-reads to the SAME kind. A future third suffix kind
        // that added a `suffix_char` arm but forgot the `from_char` arm (or vice versa) would silently
        // break that round-trip with nothing at the bottom crate to catch it. Pin the bijection over
        // EVERY kind, plus the deliberate CASE-SENSITIVITY (`n`/`r` are NOT suffixes — one canonical
        // spelling), and that every OTHER char is rejected.
        for kind in [SuffixKind::BigInt, SuffixKind::Rational] {
            let c = kind.suffix_char();
            assert_eq!(
                SuffixKind::from_char(c),
                Some(kind),
                "suffix_char/from_char are not inverse for {kind:?} (char {c:?})"
            );
            // The type name each desugars to is exactly the annotation type the reader grounds against.
            assert_eq!(
                kind.type_name(),
                match kind {
                    SuffixKind::BigInt => "BigInt",
                    SuffixKind::Rational => "Rational",
                }
            );
        }
        // Case-sensitive: the lowercase forms are not suffix letters.
        assert_eq!(
            SuffixKind::from_char('n'),
            None,
            "lowercase n is not a suffix"
        );
        assert_eq!(
            SuffixKind::from_char('r'),
            None,
            "lowercase r is not a suffix"
        );
        // A sweep of other plausible letters/digits is rejected — only `N`/`R` classify.
        for c in [
            'a', 'B', 'Z', 'x', '0', '9', 'i', 'I', 'f', 'F', 'u', 'U', 'L', ' ', '_',
        ] {
            assert_eq!(
                SuffixKind::from_char(c),
                None,
                "only N/R are suffixes; {c:?} must not classify"
            );
        }
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
    fn leaf_name_nfc_normalizes_so_canonically_equal_spellings_intern_as_one() {
        // A name is NFC-normalized before it becomes the dedup KEY (concierge-ruled 2026-07-21): two
        // Unicode-canonically-equal spellings of `café` — precomposed `é` (U+00E9) and decomposed
        // `e`+combining-acute (U+0301) — must intern to the SAME `Leaf::Name`. Before the fix they were
        // distinct leaves, so a decomposed reference failed to resolve against a precomposed def (silent
        // CDZ0101 unbound).
        let precomposed = "caf\u{00e9}";
        let decomposed = "cafe\u{0301}";
        assert_ne!(
            precomposed, decomposed,
            "the two byte spellings differ before normalization"
        );
        let mut b = Builder::new();
        let a1 = b.leaf_name(precomposed);
        let a2 = b.leaf_name(decomposed);
        assert_eq!(
            a1, a2,
            "canonically-equal name spellings intern to ONE leaf"
        );
        // And the interned text is the NFC (precomposed) form.
        assert_eq!(
            b.leaves[a1.0 as usize],
            Leaf::Name(precomposed.into()),
            "the interned name is NFC-normalized (precomposed)"
        );

        // PURE-ASCII no-op: an ASCII name (the hot common case) takes the is_ascii fast path — still
        // dedups correctly, no normalization applied (ASCII is already NFC).
        let mut c = Builder::new();
        let x1 = c.leaf_name("foo");
        let x2 = c.leaf_name("foo");
        assert_eq!(x1, x2, "an ASCII name still dedups on the fast path");
        assert_eq!(c.leaves.len(), 1, "one leaf for the repeated ASCII name");
    }

    #[test]
    fn leaf_and_leaf_name_share_one_name_index() {
        // A `Name` leaf interned via the general `leaf(Leaf::Name(..))` entry MUST land in the SAME
        // slot as one interned via the hot `leaf_name(&str)` path — `leaf` routes `Name` to `leaf_name`,
        // so there is exactly ONE dedup index for names. If they diverged, the same identifier could get
        // two leaf ids and structural equality / dedup would silently break.
        let mut b = Builder::new();
        let via_name = b.leaf_name("foo");
        let via_leaf = b.leaf(Leaf::Name("foo".into()));
        assert_eq!(via_name, via_leaf, "leaf(Name) must reuse leaf_name's id");
        // And a second `leaf_name` hit reuses it too — no new leaf appended.
        let again = b.leaf_name("foo");
        assert_eq!(again, via_name);
        let root = b.atom(via_name);
        let a = b.finish(root);
        assert_eq!(a.leaves.len(), 1, "one interned leaf for the single name");
    }

    #[test]
    fn same_text_across_leaf_kinds_stays_distinct() {
        // `Name("x")`, `Str("x")`, and `Sym("x")` carry the same text but are DIFFERENT values — the
        // name goes through `name_index`, the other two through the general `leaf_index`. They must NOT
        // collapse to one id (a name reference, a text value, and a symbol value are semantically apart).
        let mut b = Builder::new();
        let n = b.leaf(Leaf::Name("x".into()));
        let s = b.leaf(Leaf::Str("x".into()));
        let y = b.leaf(Leaf::Sym("x".into()));
        assert_ne!(n, s);
        assert_ne!(n, y);
        assert_ne!(s, y);
        // Re-interning each kind reuses its own id (dedup within a kind).
        assert_eq!(b.leaf(Leaf::Str("x".into())), s);
        assert_eq!(b.leaf(Leaf::Sym("x".into())), y);
    }

    #[test]
    fn cheap_clone_leaf_payloads_share_the_allocation_not_deep_copy() {
        // The cheap-clone arc's core invariant: the text/byte-carrying leaves hold a REFCOUNTED payload
        // (`Str`/`Sym`/`Name` = `Arc<str>`, `Bytes` = `Arc<[u8]>`), so cloning a `Leaf` is an O(1)
        // refcount bump that SHARES the underlying buffer — not a deep copy of the bytes. This pins that
        // property: if a future change reverts any of these variants to an owned `String`/`Vec<u8>`, the
        // clone silently becomes a deep copy and these `ptr_eq`/`strong_count` assertions fail (and the
        // `Arc`-typed bindings below stop compiling), catching the regression at gate time rather than in
        // a later profile. Guards the whole String->Arc<str> (increment 1) + Bytes Vec->Arc<[u8]> (2a) arc.
        let s: Arc<str> =
            Arc::from("a reasonably long string that would be costly to deep-copy per clone");
        let str_leaf = Leaf::Str(Arc::clone(&s));
        let cloned = str_leaf.clone();
        // The clone shares `s`'s allocation: extract each variant's Arc and assert pointer identity.
        if let (Leaf::Str(a), Leaf::Str(b)) = (&str_leaf, &cloned) {
            assert!(
                Arc::ptr_eq(a, b),
                "Str clone must share the Arc<str> allocation, not deep-copy"
            );
            assert!(
                Arc::ptr_eq(a, &s),
                "the leaf holds the same Arc it was built from"
            );
        } else {
            panic!("expected two Str leaves");
        }
        // Bytes: same refcount-share property over Arc<[u8]>.
        let raw: Arc<[u8]> = Arc::from(&b"\x00\xff a byte sequence long enough to matter"[..]);
        let before = Arc::strong_count(&raw);
        let bytes_leaf = Leaf::Bytes(Arc::clone(&raw));
        let bytes_clone = bytes_leaf.clone();
        assert_eq!(
            Arc::strong_count(&raw),
            before + 2,
            "each Bytes leaf holding the Arc bumps the refcount — a clone shares, never deep-copies"
        );
        if let (Leaf::Bytes(a), Leaf::Bytes(b)) = (&bytes_leaf, &bytes_clone) {
            assert!(
                Arc::ptr_eq(a, b),
                "Bytes clone must share the Arc<[u8]> allocation"
            );
        } else {
            panic!("expected two Bytes leaves");
        }
        // Sym + Name are the other two Arc<str> payloads — clone shares for them too.
        let name = Leaf::Name(Arc::clone(&s));
        let sym = Leaf::Sym(Arc::clone(&s));
        if let (Leaf::Name(n), Leaf::Sym(y)) = (&name.clone(), &sym.clone()) {
            assert!(
                Arc::ptr_eq(n, &s) && Arc::ptr_eq(y, &s),
                "Name/Sym clones share their Arc<str>"
            );
        } else {
            panic!("expected Name and Sym leaves");
        }
    }

    // Build a one-form arena `(head child…)` where `head` is either a Name or a Str atom.
    fn form(head: Leaf, children: &[Leaf]) -> Arenas {
        let mut b = Builder::new();
        let h = b.atom_leaf(head);
        let mut kids = vec![h];
        for c in children {
            kids.push(b.atom_leaf(c.clone()));
        }
        let root = b.list(kids);
        b.finish(root)
    }

    #[test]
    fn structurally_eq_collapses_ctor_head_name_and_string() {
        // The four compound ctors: a NAME-headed `(list …)` and a STRING-headed `("list" …)` denote the
        // SAME construct (the printer sugars the name form, the reader re-reads a string head), so
        // structural equality MUST treat the two head kinds as equal — in BOTH directions.
        for ctor in ["list", "tuple", "record", "map"] {
            let name_headed = form(
                Leaf::Name(ctor.into()),
                &[Leaf::Int {
                    value: BigInt::from(1),
                    radix: Radix::Dec,
                }],
            );
            let str_headed = form(
                Leaf::Str(ctor.into()),
                &[Leaf::Int {
                    value: BigInt::from(1),
                    radix: Radix::Dec,
                }],
            );
            assert!(
                name_headed.structurally_eq(&str_headed),
                "{ctor}: name head must equal string head"
            );
            assert!(
                str_headed.structurally_eq(&name_headed),
                "{ctor}: equality is symmetric"
            );
        }
    }

    #[test]
    fn structurally_eq_does_not_collapse_non_ctor_head() {
        // A non-ctor spelling has no head-kind normalization: `(foo 1)` name-headed vs string-headed are
        // DISTINCT (a bare application vs a string-headed form). Only the four ctors collapse.
        let name_headed = form(Leaf::Name("foo".into()), &[Leaf::Bool(true)]);
        let str_headed = form(Leaf::Str("foo".into()), &[Leaf::Bool(true)]);
        assert!(!name_headed.structurally_eq(&str_headed));
    }

    #[test]
    fn structurally_eq_collapse_is_head_position_only() {
        // The ctor collapse fires ONLY in head position. A ctor spelling appearing as a non-head CHILD
        // (`(f list)` with `list` a Name vs `(f "list")` with `"list"` a Str) must stay distinct — the
        // child falls through to exact leaf comparison, so Name("list") != Str("list") there.
        let name_child = form(Leaf::Name("f".into()), &[Leaf::Name("list".into())]);
        let str_child = form(Leaf::Name("f".into()), &[Leaf::Str("list".into())]);
        assert!(
            !name_child.structurally_eq(&str_child),
            "a ctor spelling as a non-head child must not collapse"
        );
    }

    #[test]
    fn structurally_eq_is_robust_to_interning_order() {
        // Structural equality compares the DENOTED tree, not the raw arena vectors — so two arenas that
        // intern the same leaves in different order (hence different leaf ids) are still equal.
        let mut b1 = Builder::new();
        let p1 = b1.name("pair");
        let x1 = b1.name("x");
        let y1 = b1.name("y");
        let r1 = b1.list(vec![p1, x1, y1]);
        let a1 = b1.finish(r1);

        let mut b2 = Builder::new();
        // Intern y before x (reversed) so the leaf ids differ from a1's.
        let _y = b2.leaf_name("y");
        let p2 = b2.name("pair");
        let x2 = b2.name("x");
        let y2 = b2.name("y");
        let r2 = b2.list(vec![p2, x2, y2]);
        let a2 = b2.finish(r2);

        assert!(a1.structurally_eq(&a2));
        // A different child count is not equal.
        let mut b3 = Builder::new();
        let p3 = b3.name("pair");
        let x3 = b3.name("x");
        let r3 = b3.list(vec![p3, x3]);
        let a3 = b3.finish(r3);
        assert!(!a1.structurally_eq(&a3));
    }

    #[test]
    fn structurally_eq_is_iterative_not_recursive_on_a_deep_arena() {
        // `node_eq` (backing `structurally_eq`) walks two arenas in parallel. An arena can originate
        // POST-DECODE, and `codec::decode` accepts arbitrarily-deep valid-tree arenas (no cap, unlike the
        // reader's MAX_NESTING_DEPTH), so the walk must be iterative — a native-recursive parallel walk
        // overflowed the native stack (SIGABRT) on a deep tree (last of the recursive-walk class, after
        // debug::print / sexpr::print_node / canon::visit). Build two independent 100k-deep chains (past
        // any native-stack limit) and assert equal-to-equal and a deep mismatch, both without overflow.
        let deep_chain = |leaf: &str, depth: usize| {
            let mut b = Builder::new();
            let mut cur = b.name(leaf);
            for _ in 0..depth {
                cur = b.list(vec![cur]);
            }
            b.finish(cur)
        };
        let depth = 100_000usize;
        let a = deep_chain("x", depth);
        let b = deep_chain("x", depth);
        assert!(
            a.structurally_eq(&b),
            "two equal deep chains compare equal (no overflow)"
        );
        // A mismatch only at the very BOTTOM (different leaf) — the walk must descend the full depth to
        // find it, exercising the stack to its deepest, and still return (false) without overflowing.
        let c = deep_chain("y", depth);
        assert!(
            !a.structurally_eq(&c),
            "a deep leaf mismatch is detected without overflow"
        );
    }

    #[test]
    fn head_and_form_accessors_distinguish_name_from_ctor() {
        // `head_name`/`as_form` read a NAME head; `head_ctor`/`as_ctor_form` read a STRING head. A
        // string-headed form has no name head (and vice-versa), so the accessors don't cross over.
        let str_headed = form(Leaf::Str("record".into()), &[Leaf::Bool(false)]);
        assert_eq!(str_headed.head_ctor(str_headed.root), Some("record"));
        assert_eq!(str_headed.head_name(str_headed.root), None);
        assert_eq!(
            str_headed
                .as_ctor_form(str_headed.root, "record")
                .map(<[_]>::len),
            Some(1)
        );
        assert_eq!(str_headed.as_form(str_headed.root, "record"), None);

        let name_headed = form(Leaf::Name("if".into()), &[Leaf::Bool(true)]);
        assert_eq!(name_headed.head_name(name_headed.root), Some("if"));
        assert_eq!(name_headed.head_ctor(name_headed.root), None);
        assert_eq!(name_headed.as_str(name_headed.root), None); // the root is a List, not a Str atom
    }

    #[test]
    fn builder_get_and_as_form_inspect_a_just_built_node() {
        // The Builder mirrors Arenas' read accessors so the parser can inspect a node it just pushed
        // (e.g. flattening a top-level `(do …)`) before `finish`. `get` returns the pushed entry and
        // `as_form` matches a name head — validated mid-build, not just post-finish.
        let mut b = Builder::new();
        let do_head = b.name("do");
        let stmt = b.name("stmt");
        let root = b.list(vec![do_head, stmt]);
        // `get` sees the list before finish.
        assert!(matches!(b.get(root), Struct::List(items) if items.len() == 2));
        // `as_form` peels the `do` head.
        assert_eq!(b.as_form(root, "do").map(<[_]>::len), Some(1));
        assert_eq!(b.as_form(root, "if"), None); // wrong head
        assert_eq!(b.structure_len(), 3); // do, stmt, root
    }

    /// A tiny deterministic PRNG (SplitMix64) — reproducible generation without a dependency (mirrors
    /// the unit-test PRNGs in `codec.rs`/`lexer.rs`).
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    /// A random leaf spanning EVERY `Leaf` variant — the shapes only a hand-built arena reaches (the
    /// reader never produces a `Bytes`/`Char`/`BadEscape`/`Sym` freely mixed with numbers), so this
    /// stresses the codec's per-kind serialization in combinations the corpus can't.
    fn gen_leaf(rng: &mut Rng) -> Leaf {
        match rng.below(11) {
            0 => Leaf::Int {
                value: BigInt::from(rng.next() as i64),
                radix: [Radix::Dec, Radix::Hex, Radix::Bin][rng.below(3)],
            },
            1 => Leaf::Float(Decimal {
                negative: rng.next() & 1 == 0,
                significand: BigInt::from(rng.next() % 10_000),
                exponent: (rng.next() % 9) as i64 - 4,
            }),
            2 => Leaf::Str(["", "hi", "a\nb", "λ中🎉"][rng.below(4)].into()),
            3 => Leaf::Char(['a', 'é', '\n', '🎉'][rng.below(4)]),
            4 => Leaf::Bytes(vec![(rng.next() & 0xff) as u8, (rng.next() & 0xff) as u8].into()),
            5 => Leaf::Bool(rng.next() & 1 == 0),
            6 => Leaf::Sym(["meter", "x", ""][rng.below(3)].into()),
            7 => Leaf::Name(["f", "x", "+", "list", "record"][rng.below(5)].into()),
            8 => Leaf::BadEscape(['q', 'z'][rng.below(2)]),
            9 => Leaf::BadChar("u+D800".into()),
            _ => Leaf::Suffixed {
                value: SuffixBody::Int {
                    value: BigInt::from(rng.next() % 1000),
                    radix: Radix::Dec,
                },
                kind: [SuffixKind::BigInt, SuffixKind::Rational][rng.below(2)],
            },
        }
    }

    /// Build a random subtree into `b` (atoms across all leaf kinds + lists of random arity), returning
    /// its root id. Bounded by `depth`.
    fn gen_node(rng: &mut Rng, b: &mut Builder, depth: usize) -> StructId {
        if depth == 0 || rng.below(3) == 0 {
            let leaf = gen_leaf(rng);
            b.atom_leaf(leaf)
        } else {
            // A list of 0..=4 children (an empty list is a shape the reader never makes, but a hand-built
            // or decoded arena can — the codec must handle it).
            let n = rng.below(5);
            let kids: Vec<StructId> = (0..n).map(|_| gen_node(rng, b, depth - 1)).collect();
            b.list(kids)
        }
    }

    #[test]
    fn builder_arena_survives_the_codec_and_structurally_eq_is_reflexive_over_generated_trees() {
        // The core invariant every surface reader rests on, exercised at the BUILDER level (not via a
        // surface parse): an arbitrary `Builder`-built arena — atoms across ALL `Leaf` variants (incl.
        // `Bytes`/`Char`/`BadEscape`/`Sym`/`Suffixed` freely mixed) + lists of arbitrary arity incl.
        // EMPTY — round-trips through the binary codec (`encode` → `decode`) to a STRUCTURALLY-EQUAL
        // arena, and `structurally_eq` is reflexive on the result. The corpus roundtrip only covers
        // reader-producible trees; this reaches the leaf-kind/arity combinations only a hand-built or
        // decoded arena takes, stressing the codec's per-kind serialization + the structurally_eq walk.
        let mut rng = Rng(0x00a5_7c0d_ea57_c0de);
        for _ in 0..4000 {
            let mut b = Builder::new();
            let depth = 1 + rng.below(4);
            let root = gen_node(&mut rng, &mut b, depth);
            let arena = b.finish(root);
            // Reflexive: an arena is structurally equal to itself.
            assert!(
                arena.structurally_eq(&arena),
                "structurally_eq not reflexive"
            );
            // Codec round-trip: encode → decode reproduces a structurally-equal arena.
            let bytes = crate::codec::encode(&arena);
            let decoded = crate::codec::decode(&bytes)
                .expect("a Builder-built arena always encodes to a decodable canonical form");
            assert!(
                arena.structurally_eq(&decoded),
                "Builder arena not preserved through the codec"
            );
            // And re-encoding the decoded arena is byte-identical (the encoding is canonical + stable).
            assert_eq!(
                bytes,
                crate::codec::encode(&decoded),
                "re-encode of the decoded arena is not byte-identical"
            );
        }
    }

    /// Copy a `src` node into `b`, OPTIONALLY flipping every compound-ctor HEAD between its `Name` and
    /// `Str` spelling (`record` ⇄ `"record"`, for the four ctors `list`/`tuple`/`record`/`map`). Since
    /// `structurally_eq` normalizes those two head kinds in head position, a flipped copy MUST stay
    /// structurally equal to the original — the property this exercises. A non-ctor head, and any leaf
    /// NOT in head position, is copied verbatim (so a bare `list` name / the string `"list"` elsewhere
    /// keeps its kind — the collapse is head-position-only).
    fn copy_flipping_ctor_heads(
        b: &mut Builder,
        src: &Arenas,
        id: StructId,
        flip: bool,
    ) -> StructId {
        match src.get(id) {
            Struct::Atom(l) => b.atom_leaf(src.leaf(*l).clone()),
            Struct::List(kids) => {
                let copied: Vec<StructId> = kids
                    .iter()
                    .enumerate()
                    .map(|(i, &k)| {
                        // Flip ONLY the head child (i == 0) and ONLY when it is one of the four ctors.
                        if flip
                            && i == 0
                            && let Struct::Atom(l) = src.get(k)
                            && let Leaf::Name(sp) | Leaf::Str(sp) = src.leaf(*l)
                            && matches!(&**sp, "list" | "tuple" | "record" | "map")
                        {
                            // Flip Name→Str / Str→Name for the ctor head.
                            let flipped = match src.leaf(*l) {
                                Leaf::Name(_) => Leaf::Str(sp.clone()),
                                _ => Leaf::Name(sp.clone()),
                            };
                            return b.atom_leaf(flipped);
                        }
                        copy_flipping_ctor_heads(b, src, k, flip)
                    })
                    .collect();
                b.list(copied)
            }
        }
    }

    #[test]
    fn structurally_eq_is_an_equivalence_with_head_collapse_over_generated_trees() {
        // `structurally_eq` is the workhorse EVERY round-trip/fidelity sweep in this crate rests on, yet
        // only REFLEXIVITY is swept generatively. Pin the rest of its contract over random trees:
        //   * SYMMETRY — `a.eq(b)` iff `b.eq(a)` (the ctor-head Name/Str collapse is head-position-only
        //     and looks asymmetric, so this is a real risk);
        //   * the HEAD COLLAPSE — an independent copy with EVERY compound-ctor head flipped between its
        //     `Name` and `Str` spelling is still equal (both directions);
        //   * DISCRIMINATION — a structurally-different tree (one leaf changed, or a child dropped) is
        //     NOT equal, and that inequality is also symmetric (no false-positive collapse).
        // Generation reuses `gen_node` (atoms across all leaf kinds + arbitrary arity), so the property is
        // checked over the whole shape space, not the few hand cases above.
        let mut rng = Rng(0xe01a_b1e5_c0de_0007);
        for _ in 0..4000 {
            let mut ba = Builder::new();
            let depth = 1 + rng.below(4);
            let root = gen_node(&mut rng, &mut ba, depth);
            let a = ba.finish(root);

            // An INDEPENDENT identical copy (fresh arena, same structure) — equality must not depend on
            // sharing the same arena/interning, and must be symmetric.
            let mut bb = Builder::new();
            let rb = copy_flipping_ctor_heads(&mut bb, &a, a.root, false);
            let a_copy = bb.finish(rb);
            assert!(a.structurally_eq(&a_copy), "equal to an independent copy");
            assert!(
                a_copy.structurally_eq(&a),
                "symmetric on an independent copy"
            );

            // A ctor-HEAD-FLIPPED copy (record ⇄ "record", …) — the collapse must make it equal, both ways.
            let mut bf = Builder::new();
            let rf = copy_flipping_ctor_heads(&mut bf, &a, a.root, true);
            let flipped = bf.finish(rf);
            assert!(
                a.structurally_eq(&flipped),
                "ctor-head Name/Str flip must stay equal (collapse)"
            );
            assert!(
                flipped.structurally_eq(&a),
                "ctor-head collapse must be symmetric"
            );

            // DISCRIMINATION: append one extra atom child at the root if it is a list (changes arity), or
            // wrap an atom root in a 1-list — either way a DIFFERENT structure that must NOT be equal.
            let mut bd = Builder::new();
            let rd = copy_flipping_ctor_heads(&mut bd, &a, a.root, false);
            let mutated_root = match bd.get(rd) {
                Struct::List(kids) => {
                    let mut k = kids.clone();
                    let extra = bd.atom_leaf(Leaf::Name("cdz-sentinel-xyz".into()));
                    k.push(extra);
                    bd.list(k)
                }
                Struct::Atom(_) => bd.list(vec![rd]), // wrap: an atom vs a 1-list are different shapes
            };
            let mutated = bd.finish(mutated_root);
            assert!(
                !a.structurally_eq(&mutated),
                "a structurally-different tree must NOT be equal"
            );
            assert!(!mutated.structurally_eq(&a), "inequality must be symmetric");
        }
    }

    #[test]
    fn schema_declared_name_reads_the_effect_head_name() {
        // A schema AST `(effect Weather (op get (-> Unit Reading)))` — the declared name is `Weather`,
        // the family/name effect routing keys on (DESIGN-userspace-effects envelope D14: resolve
        // schema-hash → schema AST → this name).
        let mut b = Builder::new();
        let effect = b.name("effect");
        let ename = b.name("Weather");
        let op = b.name("op");
        let get = b.name("get");
        let arrow = b.name("->");
        let unit = b.name("Unit");
        let reading = b.name("Reading");
        let sig = b.list(vec![arrow, unit, reading]);
        let op_get = b.list(vec![op, get, sig]);
        let root = b.list(vec![effect, ename, op_get]);
        let schema = b.finish(root);
        assert_eq!(schema.schema_declared_name(), Some("Weather"));

        // A non-effect-schema AST yields None (the resolver treats it as "not a schema").
        let mut b2 = Builder::new();
        let module = b2.name("module");
        let m = b2.name("m");
        let not_root = b2.list(vec![module, m]);
        let not_schema = b2.finish(not_root);
        assert_eq!(not_schema.schema_declared_name(), None);

        // A bare `(effect)` with no name slot (malformed) is None, not a panic.
        let mut b3 = Builder::new();
        let e3 = b3.name("effect");
        let bare_root = b3.list(vec![e3]);
        let bare = b3.finish(bare_root);
        assert_eq!(bare.schema_declared_name(), None);
    }

    #[test]
    fn effect_schema_tree_builds_the_canonical_shape_and_reads_back_its_name() {
        // The builder produces the SAME tree a hand-assembled `(effect Weather (op get SIG))` gives, and
        // `schema_declared_name` reads its name — so the one constructor is interchangeable with the shape
        // the reader was written against.
        let mut hand = Builder::new();
        let h_effect = hand.name("effect");
        let h_name = hand.name("Weather");
        let h_op = hand.name("op");
        let h_get = hand.name("get");
        let h_str = hand.name("string"); // a stand-in signature node (a string-head descriptor in practice)
        let h_sig = hand.list(vec![h_str]);
        let h_opget = hand.list(vec![h_op, h_get, h_sig]);
        let h_root = hand.list(vec![h_effect, h_name, h_opget]);
        let hand = hand.finish(h_root);

        let mut b = Builder::new();
        let sig = {
            let s = b.name("string");
            b.list(vec![s])
        };
        let root = b.effect_schema_tree("Weather", &[("get", sig)], None);
        let built = b.finish(root);

        assert_eq!(built.schema_declared_name(), Some("Weather"));
        assert!(
            built.structurally_eq(&hand),
            "the builder's tree matches the hand-assembled canonical shape"
        );
        // Identity is byte-exact: re-encoding the built tree is stable, and two builds of the SAME schema
        // hash-match (the head-kind is fixed in the builder, so no Name/Str drift splits identity).
        let bytes1 = crate::codec::encode(&built);
        let mut b2 = Builder::new();
        let sig2 = {
            let s = b2.name("string");
            b2.list(vec![s])
        };
        let root2 = b2.effect_schema_tree("Weather", &[("get", sig2)], None);
        let built2 = b2.finish(root2);
        assert_eq!(
            bytes1,
            crate::codec::encode(&built2),
            "two builds of the same schema encode byte-identically (stable identity)"
        );
    }

    #[test]
    fn effect_schema_tree_carries_ops_in_order_and_an_optional_authz() {
        // Multiple ops in caller order + a trailing `(authz …)` slot when provided; the structural heads
        // (effect/op/authz) are NAME atoms so `as_form`/`schema_declared_name` read them.
        let mut b = Builder::new();
        let (s1, s2, az) = (b.name("string"), b.name("u8"), b.name("public"));
        let root = b.effect_schema_tree("Fs", &[("read", s1), ("write", s2)], Some(az));
        let built = b.finish(root);
        assert_eq!(built.schema_declared_name(), Some("Fs"));
        // Two ops present as `(op read …)` / `(op write …)`, and an `(authz public)` tail.
        let tail = built.as_form(built.root, "effect").expect("effect form");
        assert_eq!(tail.len(), 4, "name + 2 ops + authz");
        assert!(
            built.as_form(tail[1], "op").is_some(),
            "first op is an (op …) form"
        );
        assert!(
            built.as_form(tail[2], "op").is_some(),
            "second op is an (op …) form"
        );
        assert!(
            built.as_form(tail[3], "authz").is_some(),
            "trailing (authz …)"
        );
        // No-authz omits the slot entirely.
        let mut b2 = Builder::new();
        let s = b2.name("string");
        let r2 = b2.effect_schema_tree("Fs", &[("read", s)], None);
        let no_authz = b2.finish(r2);
        let tail2 = no_authz
            .as_form(no_authz.root, "effect")
            .expect("effect form");
        assert_eq!(tail2.len(), 2, "name + 1 op, no authz slot");
    }
}
