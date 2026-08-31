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

// `alloc` (not std's prelude) so this file compiles under the `#![no_std]` minimal core as well as
// under std; `alloc::string::String` == `std::string::String`, etc.
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

/// A leaf primitive value. The value kinds plus one MARKER (`BadEscape`) the reader emits for a
/// lexically-malformed literal it cannot itself report.
///
/// `Int` is arbitrary-precision and `Float` is an exact width-free decimal: a literal's magnitude
/// or precision is never a well-formedness ceiling, and the concrete machine width (`Int64`,
/// `(Int N)`, `f32`, `f64`, …) is a *type* decision made downstream, not a representation choice
/// made here. `Float` always holds a FINITE decimal; a non-finite float VALUE (NaN, ±∞) — e.g. the
/// result of `Ast.encode` of a computed float — is a dedicated payloadless leaf ([`Leaf::FloatNan`] /
/// [`Leaf::FloatInf`]), since a decimal cannot represent it.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
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
    /// A native-compound-data CTOR-HEAD leaf — the HEAD child of a compound literal, one payloadless leaf
    /// per collection constructor (`("list" …)`'s head becomes `Atom(Leaf::Ctor(CompoundCtor::List))`).
    /// The compound KIND is recognized by this leaf's IDENTITY (a distinct codec byte), not by comparing
    /// head text against `"list"`/`"record"`/… — the native-compound-data migration
    /// (`DESIGN-native-ast-compound-data.md` D1). A distinct kind cannot collide with a user `#"record"`
    /// symbol value or a rebindable `record` name. Payloadless: the constructor is the whole value.
    Ctor(CompoundCtor),
    /// A record/map ENTRY head — the `=` of a `(= key value)` field pair. A dedicated payloadless leaf so
    /// the structural field-pair head is recognized by kind identity, distinct from the equality operator
    /// name `=` (which stays a `Name`), per `DESIGN-native-ast-compound-data.md` (the FIELD_PAIR tag).
    FieldPair,
    /// A member-access head — the `.` of a `(. obj key)` projection. A dedicated payloadless leaf so the
    /// structural member head is recognized by kind identity rather than head text
    /// (`DESIGN-native-ast-compound-data.md`, the MEMBER tag).
    Member,
    /// A native RATIONAL head — the tag of a `(RationalTag <num> <den>)` two-child node whose children are
    /// the numerator and denominator, each an ORDINARY `Leaf::Int` value leaf (deduped in the pool, shared
    /// with any equal integer). A distinct data type recognized by kind identity (operator seq-204/207: "a
    /// unique tag marking a tree as a rational + a two-child node pointing at the int value leaves"), the
    /// exact rational being num/den (NORMALIZED: lowest-terms, sign-on-numerator, denominator > 0).
    /// Payloadless like `FieldPair`/`Member` — the two integer components are the child value leaves, not
    /// carried in this leaf, so a rational reuses the arbitrary-precision `Int` machinery + leaf dedup.
    Rational,
}

/// The numeric body a type suffix decorates — an exact integer (with its display radix) or an exact
/// width-free decimal, the same two shapes the bare `Int`/`Float` leaves carry.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum SuffixBody {
    Int { value: IntValue, radix: Radix },
    Float(Decimal),
}

/// The DIRECTION of a WIT interface in a [`Builder::world_schema_tree`] — whether the world IMPORTS the
/// interface (the host provides it; the compiler emits an import marshal) or EXPORTS it (the guest
/// provides it; the compiler emits an export-side value-bridge). Direction is STRUCTURAL (a distinct
/// NAME-atom sub-head), not a member attribute, because the emitted bridge differs per direction. A
/// closed set — a WIT world member is imported or exported, nothing else.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum WitDir {
    /// The world IMPORTS this interface (host-provided) — the sub-head `import`.
    Import,
    /// The world EXPORTS this interface (guest-provided) — the sub-head `export`.
    Export,
}

impl WitDir {
    /// The NAME-atom sub-head this direction renders as in the world tree (`import`/`export`).
    pub fn head(self) -> &'static str {
        match self {
            WitDir::Import => "import",
            WitDir::Export => "export",
        }
    }
}

/// The type a numeric literal suffix selects: `N` → `BigInt` (unbounded integer), `R` → `Rational`
/// (exact rational). A closed set — the lexer accepts only these two suffix letters.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
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
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
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
/// is a record / tuple / list / map". Recognized from the reserved head via [`Arenas::compound_ctor`]
/// (the unshadowable STRING primitive) or, collapsing the shadowable NAME alias too, via
/// [`Arenas::ctor_head_key`] (head-kind normalization for [`Arenas::node_eq`]). Recognizing the kind by
/// this typed tag rather than by re-comparing head text at each consumer is the native-compound-data
/// migration; the seed compiler's `rcdzc::ast::CompoundCtor` is the twin. (`set` is not yet a primitive
/// constructor on this plane — held for operator decision D2 in
/// `implementation/design/DESIGN-native-ast-compound-data.md`.)
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum CompoundCtor {
    /// `("record" (= k v)…)` — a record.
    Record,
    /// `("tuple" e…)` — a tuple.
    Tuple,
    /// `("list" e…)` — a list.
    List,
    /// `("map" (k v)…)` — a map.
    Map,
    /// `("set" e…)` — a set (a first-class tagged construction; see rcdzc `CompoundCtor::Set`).
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
/// The significand is an arbitrary-precision non-negative magnitude; the sign lives in `negative`
/// so that `-0.0` (negative, zero significand) is preserved distinctly from `0.0`. This captures a
/// source float literal EXACTLY (no `f64` rounding), so a later type-directed rounding to a chosen
/// width happens once, from the exact value.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct Decimal {
    pub negative: bool,
    /// Big-endian non-negative magnitude of the significand (empty = zero) — a dependency-light byte
    /// magnitude (no num-bigint) so the codec-core is `no_std`. Matches the rcdzc codec twin.
    pub significand: Vec<u8>,
    /// Base-10 exponent.
    pub exponent: i64,
}

/// An arbitrary-precision integer value: a sign plus a big-endian magnitude — a dependency-light
/// bignum (no num-bigint) so the codec-core is `no_std`+alloc-only. Canonical form: the magnitude
/// carries no leading zero byte and is empty iff the value is zero (so equal values share one
/// representation, hence one leaf-pool entry and one encoding). The Int rep `Leaf::Int` / `Decimal`
/// converge on (mirrors the rcdzc codec twin); num-bigint is a `std`-side convenience via
/// [`IntValue::to_bigint`]/[`IntValue::from_bigint`].
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

/// The `IntValue` <-> `num_bigint::BigInt` bridge — a std-only convenience the all-std front-end uses to
/// keep num-bigint internal (arbitrary-precision arithmetic / decimal parsing), converting only at the
/// `Leaf`/`Decimal` boundary. Absent from the no_std minimal core, which carries no num-bigint.
#[cfg(feature = "std")]
impl IntValue {
    /// Convert to `num_bigint::BigInt`. Sign + big-endian magnitude map directly onto
    /// `BigInt::from_bytes_be`.
    pub fn to_bigint(&self) -> num_bigint::BigInt {
        use num_bigint::Sign;
        let sign = if self.magnitude.is_empty() {
            Sign::NoSign
        } else if self.negative {
            Sign::Minus
        } else {
            Sign::Plus
        };
        num_bigint::BigInt::from_bytes_be(sign, &self.magnitude)
    }

    /// Build from a `num_bigint::BigInt`, canonicalizing to the minimal big-endian magnitude (empty =
    /// zero, sign cleared on zero) — the inverse of [`Self::to_bigint`].
    pub fn from_bigint(b: &num_bigint::BigInt) -> IntValue {
        use num_bigint::Sign;
        let (sign, mut magnitude) = b.to_bytes_be();
        if sign == Sign::NoSign {
            magnitude.clear();
        }
        let start = magnitude
            .iter()
            .position(|&x| x != 0)
            .unwrap_or(magnitude.len());
        magnitude.drain(..start);
        IntValue {
            negative: sign == Sign::Minus,
            magnitude,
        }
    }
}

/// `Decimal` construction from a native float — the value-form `Leaf::Float` a native float value
/// encodes to. Part of the no_std minimal core (no num-bigint): the decimal->binary conversion is a
/// base-10 -> base-256 Horner loop, byte-identical to rcdzc's `ast::Decimal::from_f64` and the
/// cdz-runtime `float_leaf`, so all three codecs (compiler, native-rust-emit, runtime) produce
/// BYTE-IDENTICAL Float leaves (the 3-codec identity). cdz-runtime's `#[path]` include of this file
/// calls `from_f64` in its op93 float-encode path, so it must live in the no_std surface.
impl Decimal {
    /// The EXACT shortest-decimal `Decimal` for an `f64`: a WHOLE value uses its full expansion
    /// (`{f:.0}`), a non-whole its shortest round-tripping `{:e}` text; the (sign, digit string, base-10
    /// exponent) decomposition then folds the fractional digits into the exponent. `None` for a
    /// non-finite `f64` — nan/inf has no canonical value form, so the encode declines (the runtime's trap).
    pub fn from_f64(f: f64) -> Option<Decimal> {
        if !f.is_finite() {
            return None;
        }
        // `f.fract() == 0.0` (is `f` whole) WITHOUT the std-only `f64::fract`, so this compiles under
        // `#![no_std]`. Exact and identical to `fract() == 0.0` for every finite f64 (guarded above):
        // inspect the IEEE-754 fields — an unbiased exponent < 0 (|f| < 1) is whole only at ±0.0; an
        // exponent >= 52 leaves no fractional mantissa bits; otherwise f is whole iff its low `52 - exp`
        // mantissa bits are all zero.
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
        Self::from_sci(&s)
    }

    /// `from_f64`'s `Float32` twin: a promoted `f32`'s shortest decimal differs from the `f64`'s
    /// (`0.1f32` → `1e-1`, not `1.00000001…e-1`), so format the `f32` DIRECTLY. Matches the runtime
    /// `float_leaf_f32` (always `{:e}`, no whole-value branch). `None` for a non-finite `f32`.
    pub fn from_f32(f: f32) -> Option<Decimal> {
        if !f.is_finite() {
            return None;
        }
        Self::from_sci(&format!("{f:e}"))
    }

    /// Decompose a decimal/scientific text (`-2.5e-1`, `100`, `1.5e0`) into `(negative, significand,
    /// exponent)`: split the sign, the `e` exponent, and the fractional digits (folded into the exponent).
    /// The significand is the concatenated integer+fraction digit string converted to a big-endian
    /// base-256 magnitude by a Horner loop (no num-bigint). `None` on a malformed mantissa (a non-digit,
    /// or empty). Shared by `from_f64`/`from_f32` so both decompose identically.
    fn from_sci(s: &str) -> Option<Decimal> {
        let (negative, rest) = match s.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, s),
        };
        let (mantissa, exp10): (&str, i64) = match rest.split_once('e') {
            Some((m, e)) => (m, e.parse().ok()?),
            None => (rest, 0),
        };
        let (int_part, frac_part) = match mantissa.split_once('.') {
            Some((i, fr)) => (i, fr),
            None => (mantissa, ""),
        };
        let mut digits = String::from(int_part);
        digits.push_str(frac_part);
        let exponent = exp10 - frac_part.len() as i64;
        if digits.is_empty() {
            return None;
        }
        // Convert the base-10 digit string to a big-endian base-256 magnitude (Horner: acc = acc*10 + d,
        // carried in a little-endian byte vector), no num-bigint — a non-digit is malformed. Leading
        // zeros collapse to the empty magnitude (zero). Byte-identical to rcdzc's `from_f64`.
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

    /// The `f64` bits this decimal rounds to — the INVERSE of [`Self::from_f64`], used by both the
    /// compiler's float-arithmetic fold and the runtime's `Ast.encode` float path (which `#[path]`-shares
    /// this file), so all three codecs agree bit-for-bit. The base-256 significand is Horner'd into
    /// base-10 digits (`acc = acc*256 + byte`, carried in a little-endian decimal-digit vector), the
    /// reconstructed `[-]<digits>e<exponent>` is parsed by the standard library to the nearest double (the
    /// type-directed rounding `numeric-model.md` pins for `Float64`). An empty significand is zero, so
    /// `-0.0` keeps its sign through the `-` prefix; overflow rounds to `±inf`, underflow to `±0.0`.
    pub fn to_f64_bits(&self) -> u64 {
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

    /// Whether this decimal's value rounds to a FINITE `Float64` — a magnitude past the largest finite
    /// double rounds to `±inf` (a value with no written form), so a literal that fails this is malformed
    /// for the `Float64` default (`numeric-model.md` §A Floating-Point Literal That Denotes No
    /// Representable Value Is Malformed). A `Decimal` is always finite itself; this asks only whether the
    /// rounding overflows.
    pub fn is_finite_f64(&self) -> bool {
        f64::from_bits(self.to_f64_bits()).is_finite()
    }

    /// Whether this decimal fits `Float32` — its `f64` value, cast to `f32`, is still FINITE. A magnitude
    /// past the largest finite `f32` (`~3.4e38`) rounds to `±inf` in `Float32`, so `(: 1e40 Float32)` is
    /// malformed-for-the-width, the `Float32` analogue of [`Self::is_finite_f64`]. A value that already
    /// overflows `f64` fails this too.
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

/// The Builder's dedup index. FxHashMap under `std` (the hot front-end intern path — the dedup key is
/// the program's own leaf, never untrusted input, and `leaf` runs once per token during parse, so
/// SipHash is pure overhead; see `crate::fxhash`). BTreeMap in the no_std minimal core, so intern needs
/// no external hasher (`Leaf`/`String` derive `Ord`); the map is lookup-only and ids are
/// insertion-ordered, so the arena — and thus the encoded bytes — are identical regardless of map kind.
#[cfg(feature = "std")]
type InternMap<K, V> = crate::fxhash::FxHashMap<K, V>;
#[cfg(not(feature = "std"))]
type InternMap<K, V> = alloc::collections::BTreeMap<K, V>;

/// Builds `Arenas`: interns leaves on insert (dedup), appends structure occurrences (no dedup, so
/// each call is a distinct occurrence and spans stay 1:1). `root` is set once the top occurrence
/// is known via [`Builder::finish`].
#[derive(Default)]
pub struct Builder {
    leaves: Vec<Leaf>,
    leaf_index: InternMap<Leaf, LeafId>,
    // A SEPARATE dedup index for NAME leaves, keyed by the name STRING. `Name` is by far the most
    // common leaf (every identifier + construct head + qualified segment), and each occurrence arrives
    // as a `&str` slice of the source. Keying by `String` lets `leaf_name` look it up with a `&str`
    // (`String: Borrow<str>`) and allocate the owned `String` ONLY on a genuine cache miss — so a
    // repeated name (the norm in real code) costs zero allocation, instead of the old path that built a
    // `Leaf::Name(text.into())` for EVERY occurrence and discarded it on a dedup hit.
    name_index: InternMap<String, LeafId>,
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

    /// Push a leaf WITHOUT deduping — a fresh id every call, never coalesced with an equal leaf. The
    /// counterpart to [`Builder::leaf`] for a caller that needs a DISTINCT occurrence (e.g. a compiler
    /// pass emitting a leaf whose identity, not just value, must stay separate). Skips the `leaf_index`
    /// entirely, so it neither reads nor populates the dedup map.
    pub fn leaf_unique(&mut self, leaf: Leaf) -> LeafId {
        let id = LeafId(self.leaves.len() as u32);
        self.leaves.push(leaf);
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
        // Non-ASCII (std only): normalize to NFC first so canonically-equal spellings share a key.
        // `is_nfc_quick` avoids the `.nfc()` allocation when the name is already normalized (usual case).
        // ASCII is always NFC, so the pure-ASCII common case keeps the allocation-free `&str` dedup and
        // skips this entirely. The no_std minimal core has no NFC (unicode-normalization is std-only) —
        // it never builds names from unnormalized text (decode constructs `Leaf::Name` directly, not via
        // this intern), so canonical inputs are unaffected.
        #[cfg(feature = "std")]
        if !name.is_ascii() {
            use unicode_normalization::{IsNormalized, UnicodeNormalization, is_nfc_quick};
            return match is_nfc_quick(name.chars()) {
                IsNormalized::Yes => self.leaf_name_normalized(name),
                _ => {
                    let normalized: String = name.nfc().collect();
                    self.leaf_name_normalized(&normalized)
                }
            };
        }
        self.leaf_name_normalized(name)
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

    /// Convenience: an atom occurrence of a `Name`. The hot path — interns via `leaf_name` (no
    /// allocation on a dedup hit) and pushes the occurrence. Takes `impl AsRef<str>` so a caller can pass
    /// a `&str`, an owned `String`, or an `Arc<str>` without an explicit borrow; it goes through
    /// `leaf_name` by REFERENCE (`.as_ref()`), so the zero-alloc-on-hit intern is preserved regardless of
    /// the argument kind — a `String` is NOT eagerly materialized into an `Arc` before the dedup lookup
    /// (which `impl Into<Arc<str>>` would force, allocating on every occurrence, hit or miss).
    pub fn name(&mut self, name: impl AsRef<str>) -> StructId {
        let id = self.leaf_name(name.as_ref());
        self.atom(id)
    }

    /// Build a native-compound-data literal `(<ctor-leaf> child…)` — a `List` whose HEAD is an `Atom` of
    /// the reserved [`Leaf::Ctor`] leaf kind for `ctor`, followed by `children` in order. This is the
    /// M2 EMIT primitive: a compound literal's head is the ctor LEAF KIND (recognized by kind identity,
    /// [`Arenas::compound_ctor`]), NOT a `Name`/`Str` head text. `children` are the collection elements
    /// (positional for list/tuple/set; `field_pair`s for a record; entry pairs for a map). The dual of
    /// [`Arenas::compound_ctor`] + the child-tail readers.
    pub fn compound(&mut self, ctor: CompoundCtor, children: &[StructId]) -> StructId {
        let mut nodes = Vec::with_capacity(1 + children.len());
        nodes.push(self.atom_leaf(Leaf::Ctor(ctor)));
        nodes.extend_from_slice(children);
        self.list(nodes)
    }

    /// Build a record/map ENTRY `(= key value)` — a `List` whose HEAD is an `Atom` of the payloadless
    /// [`Leaf::FieldPair`] leaf kind (the `=` marker, recognized by kind, distinct from the equality
    /// operator `Name("=")`), then the key and value nodes. The M2 EMIT primitive for a record field or a
    /// map entry.
    pub fn field_pair(&mut self, key: StructId, value: StructId) -> StructId {
        let head = self.atom_leaf(Leaf::FieldPair);
        self.list(vec![head, key, value])
    }

    /// Build a member-access `(. obj key)` — a `List` whose HEAD is an `Atom` of the payloadless
    /// [`Leaf::Member`] leaf kind (the `.` marker, recognized by kind, distinct from any `Name(".")`),
    /// then the object and key nodes. The M2 EMIT primitive for a member-access projection.
    pub fn member(&mut self, obj: StructId, key: StructId) -> StructId {
        let head = self.atom_leaf(Leaf::Member);
        self.list(vec![head, obj, key])
    }

    /// Build a native RATIONAL `(RationalTag <num> <den>)` — a `List` whose HEAD is an `Atom` of the
    /// payloadless [`Leaf::Rational`] tag, then the numerator and denominator nodes (each an ordinary
    /// `Int` atom, so the integer components are first-class deduped value leaves). The two int nodes MUST
    /// carry the NORMALIZED components (lowest-terms, sign-on-numerator, denominator > 0). The read twin is
    /// [`Arenas::rational_parts`].
    pub fn rational(&mut self, numerator: StructId, denominator: StructId) -> StructId {
        let head = self.atom_leaf(Leaf::Rational);
        self.list(vec![head, numerator, denominator])
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

    /// The `Str`-LITERAL spelling of `id` if it is an `Atom(Str)`, else `None` — the `Str` sibling of
    /// [`Builder::as_name`], so a mid-build reader can recognize the unshadowable STRING-primitive
    /// compound head (`("record" …)`) as well as the `Name` alias (`(record …)`). Mirrors
    /// [`Arenas::as_str`].
    pub fn as_str(&self, id: StructId) -> Option<&str> {
        match self.get(id) {
            Struct::Atom(l) => match &self.leaves[l.0 as usize] {
                Leaf::Str(s) => Some(s),
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
    /// the caller sorts if it wants order-independent identity). The schema is the effect's DATA SHAPE only:
    /// there is NO authz node (operator directive — grants are dynamic and live OUTSIDE the schema; the
    /// schema-hash stays the collision-proof identity a grant keys on, but the authz contract is external).
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

    /// Build a WIT function-signature node `(func (param PName Desc)… (result Desc))` and return its root
    /// — the member-level shape a [`world_schema_tree`] interface member carries. A WIT func is named
    /// params PLUS a result, but a type descriptor (the kernel's `build_type`) encodes ONE type, so the
    /// func WRAPPER groups them. `params` is `(param_name, type_descriptor_node)` in declaration order
    /// (order is significant — WIT params are positional-and-named); `result` is the (ALWAYS-present)
    /// result type descriptor — a no-return member passes a `unit` descriptor, never an omitted slot, so
    /// the shape is uniform (no optional-slot presence marker that could drift the byte-exact identity).
    /// Each `Desc` is a node the caller emitted via the SAME type-descriptor emitter (`build_type`) it
    /// uses everywhere — one structural-encode path, never a parallel encoder. The `func`/`param`/`result`
    /// heads are NAME atoms (the head-kind-fixed discipline `effect_schema_tree` documents, so identical
    /// worlds encode byte-identically); the caller's descriptor nodes keep whatever heads their emitter
    /// chose. A param name participates in the identity (WIT treats it as part of the contract).
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

    /// Build a NAME-FREE operation-signature node `(func (param Desc)… (result Desc))` — the func shape an
    /// EFFECT-OPERATION schema uses, distinct from [`wit_func_sig`]'s named-param `(param PName Desc)`. The
    /// difference is deliberate and load-bearing for effect IDENTITY: a Cadenza effect operation is declared
    /// with a POSITIONAL, ANONYMOUS arrow `(op send (-> Bytes Unit))` — there is no param name to recover — so
    /// an effect op's schema-hash MUST NOT include param names, or a userspace effect could never
    /// content-address to a same-shape built-in (the whole schema-hash-identity premise; concierge ruling
    /// 2026-08-13, constraint-forced). Identity is `(effect-name, op-name, POSITIONAL param-type shape, result
    /// shape)` — the effect/op names ride [`effect_schema_tree`], the positional param + result types ride
    /// here. A param node is `(param <desc>)` (the `param` head is a NAME atom, head-kind-fixed like
    /// `wit_func_sig`), carrying ONLY the type descriptor, in declaration (positional) order. The `result` is
    /// always present (a no-return op passes a `unit` descriptor), so the shape is uniform.
    ///
    /// This is the SHARED convention for EVERY effect-op-schema producer: the kernel built-in/family schemas
    /// AND rcdzc's userspace `ty_to_wit_desc` both build op sigs through this, so the same declared shape
    /// content-addresses to the same schema-hash regardless of producer. [`wit_func_sig`] stays NAMED and is
    /// used ONLY by the WIT-WORLD path ([`world_schema_tree`]), where a member's param names ARE the `.wit`
    /// contract (e.g. `fold.apply(event)`, `kv.get(key)`) that a component reader reads from the declaration.
    pub fn wit_op_sig(&mut self, params: &[StructId], result: StructId) -> StructId {
        let mut children = Vec::with_capacity(1 + params.len() + 1);
        let func_head = self.name("func");
        children.push(func_head);
        for &desc in params {
            let param_head = self.name("param");
            let param_node = self.list(vec![param_head, desc]);
            children.push(param_node);
        }
        let result_head = self.name("result");
        let result_node = self.list(vec![result_head, result]);
        children.push(result_node);
        self.list(children)
    }

    /// Build a WIT interface node `(<dir> IfaceName (member MName FuncSig)…)` where `dir` is `import` or
    /// `export` — the direction is STRUCTURAL (a NAME-atom sub-head), not a member attribute, because the
    /// compiler emits a different value-bridge per direction (an export-side bridge for an exported member,
    /// an import marshal for an imported one). `members` is `(member_name, func_sig_node)` (each `func_sig`
    /// from [`wit_func_sig`]) in declaration order. `import`/`export`/`member` heads are NAME atoms
    /// (head-kind-fixed). Use [`WitDir`] so a caller cannot misspell the direction head.
    pub fn wit_interface(
        &mut self,
        dir: WitDir,
        iface_name: &str,
        members: &[(&str, StructId)],
    ) -> StructId {
        let mut children = Vec::with_capacity(2 + members.len());
        let dir_head = self.name(dir.head());
        children.push(dir_head);
        let iname = self.name(iface_name);
        children.push(iname);
        for &(member_name, func_sig) in members {
            let member_head = self.name("member");
            let mn = self.name(member_name);
            let member_node = self.list(vec![member_head, mn, func_sig]);
            children.push(member_node);
        }
        self.list(children)
    }

    /// Build the CANONICAL WIT-world tree `(world Name <interface>…)` and return its root — the single
    /// constructor for the target-world shape whose `Hash::of(codec::encode(root))` is the world identity,
    /// mirroring [`effect_schema_tree`]. This is the ONE structured-world representation THREE sources
    /// converge on (an external preparsed-binary-AST artifact, an inline `world …` module declaration, and
    /// v-compiler-ml's emit) so a target world means the same tree regardless of source. `interfaces` are
    /// the import/export interface nodes from [`wit_interface`], in caller order. The `world` head is a
    /// NAME atom (head-kind-fixed, matching [`Arenas::schema_declared_name`]'s `as_name` read), so two
    /// structurally-identical worlds encode byte-identically. v0 shape: interfaces + typed func members
    /// only (WIT resources and named type-aliases are deferred).
    pub fn world_schema_tree(&mut self, name: &str, interfaces: &[StructId]) -> StructId {
        let mut children = Vec::with_capacity(2 + interfaces.len());
        let world_head = self.name("world");
        children.push(world_head);
        let wname = self.name(name);
        children.push(wname);
        for &iface in interfaces {
            children.push(iface);
        }
        self.list(children)
    }

    /// Build a PRIMITIVE WIT type descriptor `(kind)` — a one-element list whose sole child is a NAME
    /// atom naming the primitive (`u8`, `string`, `bool`, `s64`, `f64`, …). One of the shared canonical
    /// WIT type-descriptor builders ([`wit_type_prim`]/[`wit_type_list`]/[`wit_type_option`]) that ALL
    /// three world sources emit through so a target world's per-member type descriptors are byte-identical
    /// regardless of source (the descriptor-form analogue of [`world_schema_tree`] for the world wrapper).
    /// The form is the LANDED one the kernel's `ast_marshal::build_type` produces and rcdzc's
    /// `parse_wit_type` already reads (v-agent-harness ruling 2026-08-12): a primitive is a NAME-head
    /// one-element list, a compound is a STRING-head form — so a `Name` vs `Str` head DISTINGUISHES prim
    /// from compound, and the codec's distinct Name/Str bytes make the choice load-bearing for identity.
    pub fn wit_type_prim(&mut self, kind: &str) -> StructId {
        let head = self.name(kind);
        self.list(vec![head])
    }

    /// Build a `list<T>` WIT type descriptor `("list" <elem>)` — a STRING-atom head `list` then the
    /// element type descriptor (itself built by a `wit_type_*`). String head (a compound), matching
    /// `build_type`'s landed form. See [`wit_type_prim`].
    pub fn wit_type_list(&mut self, elem: StructId) -> StructId {
        let head = self.atom_leaf(Leaf::Str("list".into()));
        self.list(vec![head, elem])
    }

    /// Build an `option<T>` WIT type descriptor `("option" <inner>)` — a STRING-atom head `option` then
    /// the inner type descriptor. The TYPE-side option (distinct from a value's `Some`/`None` ctor).
    /// See [`wit_type_prim`].
    pub fn wit_type_option(&mut self, inner: StructId) -> StructId {
        let head = self.atom_leaf(Leaf::Str("option".into()));
        self.list(vec![head, inner])
    }

    /// Build a `unit` WIT type descriptor `("unit")` — a STRING-atom head `unit`, no children. Unlike a
    /// scalar primitive (a NAME-head `(name)` via [`wit_type_prim`]), `unit` is a STR-head marker in the
    /// landed `build_type` form (it is the empty/no-value type, not a component-model scalar). See
    /// [`wit_type_prim`] for the head-kind discipline.
    pub fn wit_type_unit(&mut self) -> StructId {
        let head = self.atom_leaf(Leaf::Str("unit".into()));
        self.list(vec![head])
    }

    /// Build a `tuple<A, B, …>` WIT type descriptor `("tuple" <a> <b> …)` — a STRING-atom head `tuple`
    /// then each element type descriptor in positional order. Matches the landed `build_type` form.
    /// See [`wit_type_prim`].
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
                // MIGRATION BRIDGE (M2, transitional): the native MEMBER (`.`) and FIELD_PAIR (`=`)
                // structural head leaves report their legacy head SPELLING here, so every recognizer that
                // detects a member access / field pair by head text (`as_form(id, ".")`, the raw
                // `as_name(head) == Some("=")`/`Some(".")` checks, the field-key LABEL-position detection)
                // works uniformly whether the head is the native leaf (what the reader now emits) or the
                // legacy `Name(".")`/`Name("=")`. This bridge lived in rcdzc's own `ast.rs` before the
                // consolidation onto this crate (#5158) — restoring it here keeps the ~40 rcdzc member/
                // field-pair recognition sites working on native leaves (namespaced ctor patterns
                // `((. Sum V) x)`, record `(= k v)` patterns, …). Deliberately NOT the ctor-head leaves
                // (`Leaf::Ctor`): those are recognized by kind via `compound_ctor_leaf`/
                // `compound_form_of`, and keeping them `None` here preserves ctor-leaf-is-its-own-identity
                // (no name-collapse). M3 removes the legacy heads; these two bridge arms then just spell the
                // native head.
                Leaf::Member => Some("."),
                Leaf::FieldPair => Some("="),
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

    /// The value of an `Int` leaf as a `usize`, if `id` is an atom of a non-negative `Int` that fits.
    /// Used to read small index metadata (e.g. the SEC-F1 `(resource N)` param index) off the tree.
    pub fn as_int_usize(&self, id: StructId) -> Option<usize> {
        match self.get(id) {
            Struct::Atom(l) => match self.leaf(*l) {
                Leaf::Int { value, .. } => value.to_u128().and_then(|u| usize::try_from(u).ok()),
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

    /// The [`CompoundCtor`] this `List` node denotes via its native ctor-LEAF-KIND head — the read
    /// primitive, recognized by leaf-kind IDENTITY ([`Leaf::Ctor`]) rather than by head text. `None` if
    /// the head is not a ctor-leaf atom. The dual of the emit primitive [`Builder::compound`]. See
    /// `implementation/design/DESIGN-native-ast-compound-data.md`. (The transitional string-head
    /// recognizer `compound_ctor` this replaced was removed post-M3; native ctor-leaf recognition is the
    /// only compound-tag read.)
    pub fn compound_ctor_leaf(&self, id: StructId) -> Option<CompoundCtor> {
        match self.get(id) {
            Struct::List(items) => match self.leaf(self.atom_leaf_id(*items.first()?)?) {
                Leaf::Ctor(c) => Some(*c),
                _ => None,
            },
            _ => None,
        }
    }

    /// The `(key, value)` of a native `(= key value)` record/map ENTRY — a `List` of exactly three whose
    /// head is the [`Leaf::FieldPair`] leaf kind (the `=` marker, recognized by kind). `None` otherwise.
    /// The dual of [`Builder::field_pair`].
    pub fn field_pair_parts(&self, id: StructId) -> Option<(StructId, StructId)> {
        match self.get(id) {
            Struct::List(items) if items.len() == 3 => {
                match self.leaf(self.atom_leaf_id(items[0])?) {
                    Leaf::FieldPair => Some((items[1], items[2])),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The `(obj, key)` of a native `(. obj key)` MEMBER-ACCESS projection — a `List` of exactly three
    /// whose head is the [`Leaf::Member`] leaf kind (the `.` marker, recognized by kind). `None`
    /// otherwise. The dual of [`Builder::member`].
    pub fn member_parts(&self, id: StructId) -> Option<(StructId, StructId)> {
        match self.get(id) {
            Struct::List(items) if items.len() == 3 => {
                match self.leaf(self.atom_leaf_id(items[0])?) {
                    Leaf::Member => Some((items[1], items[2])),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// If `id` is a reader COMMENT wrapper — a leading `(comment "text" form)` (a `//`/`;` on its own line
    /// above `form`) or a trailing `(comment-after "text" form)` (a same-line comment) — the wrapped `form`.
    /// Both share the identical `[<string>, <form>]` tail (first tail element a `Str` leaf), so both peel by
    /// the one rule. `None` when `id` is not a well-formed comment wrapper. One layer only — [`peel_comments`]
    /// follows the whole chain.
    pub fn comment_wrapped_form(&self, id: StructId) -> Option<StructId> {
        let tail = self
            .as_form(id, "comment")
            .or_else(|| self.as_form(id, "comment-after"))?;
        let (&text, &form) = (tail.first()?, tail.get(1)?);
        // The first tail element must be a STRING (the comment text); else it is not a reader comment node.
        matches!(self.get(text), Struct::Atom(l) if matches!(self.leaf(*l), Leaf::Str(_)))
            .then_some(form)
    }

    /// Follow a chain of reader-produced comment wrappers ([`comment_wrapped_form`], leading and/or trailing
    /// in any mix) down to the form they annotate, returning the innermost non-comment form (or `id` itself
    /// when it is not a comment wrapper). Structural consumers that DISPATCH on a form's head (the corpus
    /// case/clause walks, the compiler's comment strip) call this so a comment annotating a form is
    /// transparent to them — the comment survives in the tree for printing but never hides the form it
    /// wraps. Read-only: unlike the compiler's in-place `strip_comments`, this does not mutate the arena.
    pub fn peel_comments(&self, mut id: StructId) -> StructId {
        while let Some(form) = self.comment_wrapped_form(id) {
            id = form;
        }
        id
    }

    /// The `(numerator, denominator)` of a native RATIONAL node — a `List` of exactly three whose head is
    /// the [`Leaf::Rational`] tag. `None` otherwise. The read twin of [`Builder::rational`]; the two
    /// returned ids are ordinary `Int` value-leaf atoms (normalized: lowest-terms, sign-on-numerator,
    /// denominator > 0).
    pub fn rational_parts(&self, id: StructId) -> Option<(StructId, StructId)> {
        match self.get(id) {
            Struct::List(items) if items.len() == 3 => {
                match self.leaf(self.atom_leaf_id(items[0])?) {
                    Leaf::Rational => Some((items[1], items[2])),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The [`LeafId`] of `id` if it is an `Atom` occurrence, else `None` — a small accessor the native
    /// ctor-head recognizers ([`compound_ctor_leaf`], [`field_pair_parts`], [`member_parts`]) use to reach
    /// a head node's leaf without re-borrowing.
    fn atom_leaf_id(&self, id: StructId) -> Option<LeafId> {
        match self.get(id) {
            Struct::Atom(l) => Some(*l),
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

    /// Find the collection REST/SPREAD marker `..` in `elems`, recognizing BOTH shapes during the
    /// operator's repo-wide `(.. v)` migration: the legacy FLAT form — a bare `Name("..")` element whose
    /// operand is the NEXT sibling (`(list a .. rest)`) — AND the WRAPPED form — a self-contained
    /// `(.. operand)` node, a list headed by `..` (`(list a (.. rest))`, `#list(1 (.. xs) 2)`). Returns
    /// `(marker_index, operand, trailing_start)`: the elements BEFORE the marker are `elems[..marker_index]`,
    /// the operand is `operand`, and the elements AFTER are `elems[trailing_start..]`. The two shapes differ
    /// only in how many slots the marker occupies: the flat form spans TWO (`..` + its operand sibling →
    /// `trailing_start = marker_index + 2`); the wrapped form spans ONE (the `(.. operand)` node itself →
    /// `trailing_start = marker_index + 1`). Every rest-marker scan reads through this so both shapes are
    /// accepted uniformly (Phase 1 of the `(.. v)` migration). `None` when there is no rest marker. A flat
    /// `..` with no following operand (malformed) yields the marker node itself as `operand` — the caller's
    /// existing shape validation handles it exactly as before.
    pub fn rest_marker(&self, elems: &[StructId]) -> Option<(usize, StructId, usize)> {
        for (i, &e) in elems.iter().enumerate() {
            // WRAPPED `(.. operand)` — a list headed by `..`, the operand its sole argument.
            if let Some(args) = self.as_form(e, "..") {
                return Some((i, args.first().copied().unwrap_or(e), i + 1));
            }
            // FLAT `..` marker — the operand is the next sibling.
            if self.as_name(e) == Some("..") {
                return Some((i, elems.get(i + 1).copied().unwrap_or(e), i + 2));
            }
        }
        None
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

    /// If `id` is an `Atom` of a decimal/float literal, its `Decimal`. The float analogue of [`Arenas::as_int`],
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

    /// The child occurrences of `id` if it is a `List` headed by the compound ctor `want`, accepting
    /// EITHER head spelling (a transitional dual-read) — the tag-typed twin of the
    /// `as_form(id, "…").or_else(|| as_ctor_form(id, "…"))` idiom for the four compound ctors.
    /// Like `as_form`/`as_ctor_form`, an empty tail (a head-only list) yields `Some(&[])`.
    pub fn compound_form_of(&self, id: StructId, want: CompoundCtor) -> Option<&[StructId]> {
        match self.get(id) {
            Struct::List(items) => {
                let &h = items.first()?;
                // Native ctor-LEAF-KIND head (M2, what the reader now emits) OR the legacy name/string head.
                if self.compound_ctor_leaf(id) == Some(want) {
                    return Some(&items[1..]);
                }
                // M3 reader-flip: recognize the native ctor-leaf head (above) OR the shadowable NAME alias
                // (`as_name`) ONLY — the legacy STRING head (`as_str`) is dropped (no more `("record" …)`).
                let spelling = self.as_name(h)?;
                (CompoundCtor::from_spelling(spelling) == Some(want)).then_some(&items[1..])
            }
            _ => None,
        }
    }

    /// The declared TYPE NAME from a `(type …)` decl's FIRST tail element `head_occ` (the element after the
    /// `type` keyword). Two spellings: a BARE atom `(type Box …)` — the atom IS the name — OR a
    /// PARENTHESIZED head `(type (Box a b…) …)` — a `(Name params…)` list whose HEAD atom is the name.
    /// `None` if `head_occ` is neither (a malformed decl). The ONE place both spellings are decoded, so every
    /// raw `(type …)`-tail name-reader agrees — a bare `head_occ.as_name()` returns `None` for a `(List)` head, so
    /// without this a parenthesized-head generic type was INVISIBLE to those readers (un-exported / not a
    /// known user type).
    pub fn type_decl_head_name(&self, head_occ: StructId) -> Option<&str> {
        match self.get(head_occ) {
            // Bare-atom name: `(type Box …)`.
            Struct::Atom(_) => self.as_name(head_occ),
            // Parenthesized `(Name params…)` head: the list's head atom is the name.
            Struct::List(kids) => kids.first().and_then(|&h| self.as_name(h)),
        }
    }

    /// The `(key, value)` of a canonical `(= key value)` FIELD PAIR node — a `List` of exactly three whose
    /// head is the NAME `=`. The transitional NAME-headed reader (distinct from [`Arenas::field_pair_parts`],
    /// which recognizes the M2 [`Leaf::FieldPair`] leaf-kind head); a consumer dual-reads
    /// `field_pair_parts(id).or_else(|| field_pair(id))` across the M2 flip. `None` for anything not a
    /// well-formed `(= k v)` triple.
    pub fn field_pair(&self, id: StructId) -> Option<(StructId, StructId)> {
        match self.get(id) {
            Struct::List(kv) if kv.len() == 3 && self.as_name(kv[0]) == Some("=") => {
                Some((kv[1], kv[2]))
            }
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

    /// The compound-ctor TAG an occurrence denotes as a LIST HEAD, collapsing ALL THREE head spellings of
    /// a compound ctor to one [`CompoundCtor`]: the native unshadowable ctor-LEAF ([`Leaf::Ctor`], the M2
    /// primitive), the shadowable NAME alias (`(record …)`), and the legacy unshadowable STRING primitive
    /// (`("record" …)`). So head-kind normalization in [`node_eq`] treats `#record(…)`, `(record …)`, and
    /// `("record" …)` as the same head — head-KIND never splits structural identity (consistent with the
    /// documented [`structurally_eq`] contract). This is what lets a native-head value and a still-legacy
    /// alias/string-head value (e.g. an un-migrated corpus record vs a `read_ml` native record) compare
    /// structurally equal across the M2 migration. It does NOT weaken byte-level content-addressing: the
    /// codec still emits DISTINCT bytes per head kind, so their hashes differ — this normalization is only
    /// for the lenient structural comparison. Only the five compound ctors qualify; the `=`/`.` marker
    /// leaves ([`Leaf::FieldPair`]/[`Leaf::Member`]) are NOT ctor heads and keep their own identity (they do
    /// not collapse with `Name("=")`/`Name(".")`). Every other name/string is left to exact leaf comparison.
    fn ctor_head_key(&self, id: StructId) -> Option<CompoundCtor> {
        match self.get(id) {
            Struct::Atom(l) => match self.leaf(*l) {
                // The native ctor-LEAF head IS the tag directly (M2 primitive).
                Leaf::Ctor(c) => Some(*c),
                // The shadowable NAME alias + legacy STRING primitive collapse to the tag by spelling.
                Leaf::Name(n) => CompoundCtor::from_spelling(n),
                Leaf::Str(s) => CompoundCtor::from_spelling(s),
                _ => None,
            },
            _ => None,
        }
    }
}

// `all(test, feature = "std")`: the libtest harness needs std, and these tests use the std-gated
// `IntValue<->BigInt` bridge — so they only ever ran under std. Making it explicit stops cdz-runtime's
// no_std `#[path]` include (mechanism B) from dragging this test module into its own test build.
#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use num_bigint::BigInt;
    use std::str::FromStr;

    #[test]
    fn rational_node_builds_recognizes_and_round_trips() {
        // The native RATIONAL node: Builder::rational(num, den) → a (RationalTag <num-int> <den-int>) List;
        // rational_parts reads the two Int children back; the whole thing survives a codec round-trip. The
        // num/den are ordinary Int value leaves (operator seq-204: "point at the int value leaves").
        let mut b = Builder::new();
        let num = b.atom_leaf(Leaf::Int {
            value: IntValue::from_i64(3),
            radix: Radix::Dec,
        });
        let den = b.atom_leaf(Leaf::Int {
            value: IntValue::from_i64(4),
            radix: Radix::Dec,
        });
        let rat = b.rational(num, den);
        let a = b.finish(rat);

        let (n, d) = a
            .rational_parts(a.root)
            .expect("rational_parts reads the tag+2-children node");
        assert_eq!(a.as_int(n).map(|v| v.to_i64_bits()), Some(3));
        assert_eq!(a.as_int(d).map(|v| v.to_i64_bits()), Some(4));
        // A non-rational List is not misread.
        assert_eq!(a.rational_parts(num), None);

        // Codec round-trip: decode∘encode preserves the rational node (tag + both Int children).
        let bytes = crate::codec::encode(&a);
        let back = crate::codec::decode(&bytes).expect("rational node decodes");
        let (n2, d2) = back
            .rational_parts(back.root)
            .expect("decoded rational still recognized");
        assert_eq!(back.as_int(n2).map(|v| v.to_i64_bits()), Some(3));
        assert_eq!(back.as_int(d2).map(|v| v.to_i64_bits()), Some(4));
    }

    #[test]
    fn arenas_leaf_accessors_and_compound_recognizers() {
        // Pins the leaf-atom accessors (`as_int`/`as_float`/`as_bool`) and the compound recognizers
        // (`compound_ctor_leaf`/`compound_form_of`) + `type_decl_head_name`
        // — the `Arenas` surface `rcdzc` re-exports from this crate once it consolidates off its own copy.
        let mut b = Builder::new();
        let iatom = b.atom_leaf(Leaf::Int {
            value: IntValue::from_i64(7),
            radix: Radix::Dec,
        });
        let fatom = b.atom_leaf(Leaf::Float(Decimal::from_f64(0.5).unwrap()));
        let batom = b.atom_leaf(Leaf::Bool(true));
        // Native ctor-leaf-headed `(list 7)` — `Builder::compound` emits the ctor-leaf head.
        let native_list = b.compound(CompoundCtor::List, &[iatom]);
        // Shadowable NAME-alias-headed `(tuple 0.5)` — a dual-read-only spelling.
        let alias_head = b.name("tuple");
        let alias = b.list(vec![alias_head, fatom]);
        // A parenthesized `(Box a)` type-decl head.
        let boxn = b.name("Box");
        let pa = b.name("a");
        let paren_head = b.list(vec![boxn, pa]);
        let root = b.list(vec![native_list, alias, paren_head, batom]);
        let a = b.finish(root);

        assert_eq!(a.as_int(iatom).map(|v| v.to_i64_bits()), Some(7));
        assert!(a.as_float(fatom).is_some());
        assert_eq!(a.as_bool(batom), Some(true));
        assert_eq!(a.as_int(batom), None);
        assert_eq!(a.as_bool(iatom), None);

        // Structural dispatch: the native ctor-leaf head is recognized; the shadowable NAME alias is NOT.
        assert_eq!(a.compound_ctor_leaf(native_list), Some(CompoundCtor::List));
        assert_eq!(a.compound_ctor_leaf(alias), None);
        assert_eq!(
            a.compound_form_of(native_list, CompoundCtor::List)
                .map(<[_]>::len),
            Some(1)
        );
        assert_eq!(a.compound_form_of(native_list, CompoundCtor::Map), None);
        // Dual-read (compound_form_of) accepts the shadowable NAME alias spelling too.
        assert_eq!(
            a.compound_form_of(alias, CompoundCtor::Tuple)
                .map(<[_]>::len),
            Some(1)
        );

        // `type_decl_head_name`: parenthesized `(Box a)` head → "Box"; a bare atom → itself.
        assert_eq!(a.type_decl_head_name(paren_head), Some("Box"));
        assert_eq!(a.type_decl_head_name(boxn), Some("Box"));
    }

    #[test]
    fn ctor_head_key_recognizes_the_reserved_vocabulary() {
        let mut b = Builder::new();
        // `("record" _)` — a STRING head occurrence.
        let rec_head = b.atom_leaf(Leaf::Str("record".into()));
        let payload = b.atom_leaf(Leaf::Str("_".into()));
        let rec = b.list(vec![rec_head, payload]);
        // `(record)` — the NAME alias head. `ctor_head_key` (the node_eq head-normalizer) collapses the
        // NAME and STRING head spellings to the same tag.
        let alias_head = b.name("record");
        let alias = b.list(vec![alias_head]);
        // One root keeps both subtrees reachable from `finish`.
        let root = b.list(vec![rec, alias]);
        let a = b.finish(root);

        // ctor_head_key operates on the HEAD occurrence and collapses NAME + STRING to one tag.
        assert_eq!(a.ctor_head_key(rec_head), Some(CompoundCtor::Record));
        assert_eq!(a.ctor_head_key(alias_head), Some(CompoundCtor::Record));
        // A non-ctor name/string head is not a tag.
        let other = b2_head_tag("if");
        assert_eq!(other, None);

        // All four spellings map to their tag, via either head kind.
        for (spelling, want) in [
            ("record", CompoundCtor::Record),
            ("tuple", CompoundCtor::Tuple),
            ("list", CompoundCtor::List),
            ("map", CompoundCtor::Map),
        ] {
            let mut b = Builder::new();
            let s = b.atom_leaf(Leaf::Str(spelling.into()));
            let n = b.name(spelling);
            // Keep both head atoms reachable from the root so neither is pruned by `finish`.
            let node = b.list(vec![s, n]);
            let a = b.finish(node);
            assert_eq!(
                a.ctor_head_key(s),
                Some(want),
                "ctor_head_key str `{spelling}`"
            );
            assert_eq!(
                a.ctor_head_key(n),
                Some(want),
                "ctor_head_key name `{spelling}`"
            );
        }
    }

    #[test]
    fn native_compound_value_golden_canonical_bytes() {
        // GOLDEN VECTORS: the CANONICAL binary bytes the compiler codec (Builder + canon + encode) produces
        // for representative Option-B compound VALUES. `encode` canonicalizes before serializing, so these
        // bytes ARE the content-address form. They serve TWO purposes: (1) a wire-contract regression guard
        // — a future canonical-form change that would silently move a compound value's content hash trips
        // here; (2) the authoritative reference the RUNTIME value codec (op62/90's DocBuilder) must match
        // byte-for-byte, or the value-wire forks from the AST-wire despite both being Option B (v-runtime +
        // v-static-data flagged that op62's shared name-index pool can order leaves differently from the
        // compiler Builder). Header is `cdzast\x00\x01`. Note the record/map share ONE payloadless
        // FIELD_PAIR (25) leaf across both entries (pool dedup) — Option B removes the `=` NAME leaf whose
        // ordering was the known op62/Builder divergence.
        fn int(b: &mut Builder, n: i64) -> StructId {
            b.atom_leaf(Leaf::Int {
                value: IntValue::from_i64(n),
                radix: Radix::Dec,
            })
        }
        // #record((= a 1) (= b 2)) — leaf pool: RECORD_CTOR=22, FIELD_PAIR=25, "a", 1, "b", 2.
        let mut b = Builder::new();
        let a = b.name("a");
        let one = int(&mut b, 1);
        let bn = b.name("b");
        let two = int(&mut b, 2);
        let fa = b.field_pair(a, one);
        let fb = b.field_pair(bn, two);
        let record = b.compound(CompoundCtor::Record, &[fa, fb]);
        let ra = b.finish(record);
        assert_eq!(
            crate::codec::encode(&ra),
            vec![
                99, 100, 122, 97, 115, 116, 0, 1, 6, 22, 25, 10, 1, 97, 0, 1, 1, 10, 1, 98, 0, 1,
                2, 10, 0, 0, 0, 1, 0, 2, 0, 3, 1, 3, 1, 2, 3, 0, 1, 0, 4, 0, 5, 1, 3, 5, 6, 7, 1,
                3, 0, 4, 8, 9
            ],
            "record golden bytes"
        );
        // #set(1 2 3) — leaf pool: SET_CTOR=24, 1, 2, 3.
        let mut b = Builder::new();
        let (s1, s2, s3) = (int(&mut b, 1), int(&mut b, 2), int(&mut b, 3));
        let set = b.compound(CompoundCtor::Set, &[s1, s2, s3]);
        let sa = b.finish(set);
        assert_eq!(
            crate::codec::encode(&sa),
            vec![
                99, 100, 122, 97, 115, 116, 0, 1, 4, 24, 0, 1, 1, 0, 1, 2, 0, 1, 3, 5, 0, 0, 0, 1,
                0, 2, 0, 3, 1, 4, 0, 1, 2, 3, 4
            ],
            "set golden bytes"
        );
        // #map((= 1 10) (= 2 20)) — leaf pool: MAP_CTOR=23, FIELD_PAIR=25, 1, 10, 2, 20.
        let mut b = Builder::new();
        let (k1, v1, k2, v2) = (
            int(&mut b, 1),
            int(&mut b, 10),
            int(&mut b, 2),
            int(&mut b, 20),
        );
        let e1 = b.field_pair(k1, v1);
        let e2 = b.field_pair(k2, v2);
        let map = b.compound(CompoundCtor::Map, &[e1, e2]);
        let ma = b.finish(map);
        assert_eq!(
            crate::codec::encode(&ma),
            vec![
                99, 100, 122, 97, 115, 116, 0, 1, 6, 23, 25, 0, 1, 1, 0, 1, 10, 0, 1, 2, 0, 1, 20,
                10, 0, 0, 0, 1, 0, 2, 0, 3, 1, 3, 1, 2, 3, 0, 1, 0, 4, 0, 5, 1, 3, 5, 6, 7, 1, 3,
                0, 4, 8, 9
            ],
            "map golden bytes"
        );
        // #tuple(1 2) — leaf pool [TUPLE_CTOR=21, 1, 2].
        let mut b = Builder::new();
        let (t1, t2) = (int(&mut b, 1), int(&mut b, 2));
        let tuple = b.compound(CompoundCtor::Tuple, &[t1, t2]);
        let ta = b.finish(tuple);
        assert_eq!(
            crate::codec::encode(&ta),
            vec![
                99, 100, 122, 97, 115, 116, 0, 1, 3, 21, 0, 1, 1, 0, 1, 2, 4, 0, 0, 0, 1, 0, 2, 1,
                3, 0, 1, 2, 3
            ],
            "tuple golden bytes"
        );
        // #list(1 2) — leaf pool [LIST_CTOR=20, 1, 2].
        let mut b = Builder::new();
        let (l1, l2) = (int(&mut b, 1), int(&mut b, 2));
        let list = b.compound(CompoundCtor::List, &[l1, l2]);
        let la = b.finish(list);
        assert_eq!(
            crate::codec::encode(&la),
            vec![
                99, 100, 122, 97, 115, 116, 0, 1, 3, 20, 0, 1, 1, 0, 1, 2, 4, 0, 0, 0, 1, 0, 2, 1,
                3, 0, 1, 2, 3
            ],
            "list golden bytes"
        );
        // NESTED #list(#record((= a 1)) #set(2 3)) — nested ctor heads; ALL four ctor kinds (LIST_CTOR,
        // RECORD_CTOR, FIELD_PAIR, SET_CTOR) are deduped ONCE in the shared leaf pool across nesting levels
        // — the cross-level dedup/order op62 must reproduce. Leaf pool [LIST_CTOR=20, RECORD_CTOR=22,
        // FIELD_PAIR=25, "a", 1, SET_CTOR=24, 2, 3].
        let mut b = Builder::new();
        let na = b.name("a");
        let n1 = int(&mut b, 1);
        let nfp = b.field_pair(na, n1);
        let nrec = b.compound(CompoundCtor::Record, &[nfp]);
        let (ns2, ns3) = (int(&mut b, 2), int(&mut b, 3));
        let nset = b.compound(CompoundCtor::Set, &[ns2, ns3]);
        let nested = b.compound(CompoundCtor::List, &[nrec, nset]);
        let na2 = b.finish(nested);
        assert_eq!(
            crate::codec::encode(&na2),
            vec![
                99, 100, 122, 97, 115, 116, 0, 1, 8, 20, 22, 25, 10, 1, 97, 0, 1, 1, 24, 0, 1, 2,
                0, 1, 3, 12, 0, 0, 0, 1, 0, 2, 0, 3, 0, 4, 1, 3, 2, 3, 4, 1, 2, 1, 5, 0, 5, 0, 6,
                0, 7, 1, 3, 7, 8, 9, 1, 3, 0, 6, 10, 11
            ],
            "nested list-of-(record,set) golden bytes"
        );
    }

    #[test]
    fn native_ctor_leaf_emit_api_round_trips_through_the_read_helpers_and_the_codec() {
        // The M2 emit primitives (`Builder::compound`/`field_pair`/`member`) build ctor-LEAF-KIND heads,
        // and the read primitives (`compound_ctor_leaf`/`field_pair_parts`/`member_parts`) recognize them
        // by leaf-kind identity — never by head text. Emit each shape, recognize it back, and confirm it
        // survives the binary codec unchanged (the wire carries the ctor-leaf head).
        let mut b = Builder::new();
        let one = b.atom_leaf(Leaf::Int {
            value: IntValue::from_bigint(&BigInt::from(1)),
            radix: Radix::Dec,
        });
        let two = b.atom_leaf(Leaf::Int {
            value: IntValue::from_bigint(&BigInt::from(2)),
            radix: Radix::Dec,
        });
        let key = b.name("x");
        // A `(= x 2)` field pair, a `(. x k)` member access, and one compound of each collection kind.
        let fp = b.field_pair(key, two);
        let mem = b.member(key, one);
        let list = b.compound(CompoundCtor::List, &[one, two]);
        let tuple = b.compound(CompoundCtor::Tuple, &[one, two]);
        let record = b.compound(CompoundCtor::Record, &[fp]);
        let map = b.compound(CompoundCtor::Map, &[fp]);
        let set = b.compound(CompoundCtor::Set, &[one, two]);
        let root = b.list(vec![list, tuple, record, map, set, fp, mem]);
        let a = b.finish(root);

        // Read side recognizes each ctor by LEAF KIND.
        assert_eq!(a.compound_ctor_leaf(list), Some(CompoundCtor::List));
        assert_eq!(a.compound_ctor_leaf(tuple), Some(CompoundCtor::Tuple));
        assert_eq!(a.compound_ctor_leaf(record), Some(CompoundCtor::Record));
        assert_eq!(a.compound_ctor_leaf(map), Some(CompoundCtor::Map));
        assert_eq!(a.compound_ctor_leaf(set), Some(CompoundCtor::Set));
        // A native ctor-leaf head is NOT a string/name head, so the string-head reader (`head_ctor`) and
        // the name-head reader (`head_name`) do not see it — the leaf-kind and text-head reads are disjoint.
        assert_eq!(a.head_ctor(list), None);
        assert_eq!(a.head_name(list), None);
        // Field-pair / member parts read back in order.
        assert_eq!(a.field_pair_parts(fp), Some((key, two)));
        assert_eq!(a.member_parts(mem), Some((key, one)));
        // A collection node is not a field pair / member, and vice-versa.
        assert_eq!(a.field_pair_parts(list), None);
        assert_eq!(a.member_parts(fp), None);
        assert_eq!(a.compound_ctor_leaf(fp), None);

        // The whole tree survives the binary codec (the ctor-leaf heads ride the wire).
        let back = crate::codec::decode(&crate::codec::encode(&a))
            .expect("decode of an arena with native ctor-leaf heads");
        assert!(
            a.structurally_eq(&back),
            "ctor-leaf arena survives the codec"
        );
    }

    #[test]
    fn native_ctor_leaf_heads_are_recognized_after_a_codec_round_trip() {
        // The full wire→recognition path the M2 flip depends on: a compound literal's ctor-leaf head must
        // still be recognized by KIND after an encode→decode round-trip (decode re-canonicalizes the leaf
        // pool + renumbers occurrences, but the head leaf's kind must survive so a consumer reading the
        // DECODED arena — the compiler's trusted path, or a re-reading platform consumer — still recognizes
        // it). Build one node of each shape (each with its OWN fresh children, so the tree needs no
        // de-sharing) as the root's children, round-trip, then recognize each on the DECODED arena.
        let mut b = Builder::new();
        let (la, lb) = (b.name("a"), b.name("b"));
        let list = b.compound(CompoundCtor::List, &[la, lb]);
        let (ta, tb) = (b.name("c"), b.name("d"));
        let tuple = b.compound(CompoundCtor::Tuple, &[ta, tb]);
        let (rk, rv) = (b.name("k"), b.name("v"));
        let rfp = b.field_pair(rk, rv);
        let record = b.compound(CompoundCtor::Record, &[rfp]);
        let (mk, mv) = (b.name("mk"), b.name("mv"));
        let mfp = b.field_pair(mk, mv);
        let map = b.compound(CompoundCtor::Map, &[mfp]);
        let (sa, sb) = (b.name("e"), b.name("f"));
        let set = b.compound(CompoundCtor::Set, &[sa, sb]);
        let (mo, mkey) = (b.name("obj"), b.name("key"));
        let member = b.member(mo, mkey);
        let (fk, fv) = (b.name("fk"), b.name("fv"));
        let fp = b.field_pair(fk, fv);
        // Fixed child order so the decoded nodes are findable by index (decode preserves tree shape).
        let root = b.list(vec![list, tuple, record, map, set, member, fp]);
        let a = b.finish(root);

        let back = crate::codec::decode(&crate::codec::encode(&a))
            .expect("decode of an arena with native ctor-leaf heads");
        let Struct::List(kids) = back.get(back.root) else {
            panic!("decoded root is a list");
        };
        assert_eq!(kids.len(), 7, "root has 7 children");
        assert_eq!(back.compound_ctor_leaf(kids[0]), Some(CompoundCtor::List));
        assert_eq!(back.compound_ctor_leaf(kids[1]), Some(CompoundCtor::Tuple));
        assert_eq!(back.compound_ctor_leaf(kids[2]), Some(CompoundCtor::Record));
        assert_eq!(back.compound_ctor_leaf(kids[3]), Some(CompoundCtor::Map));
        assert_eq!(back.compound_ctor_leaf(kids[4]), Some(CompoundCtor::Set));
        assert!(
            back.member_parts(kids[5]).is_some(),
            "member recognized after decode"
        );
        assert!(
            back.field_pair_parts(kids[6]).is_some(),
            "field pair recognized after decode"
        );
        // The record's entry is a recognizable FieldPair on the decoded arena too (its head-child [0] is
        // the RECORD_CTOR leaf; child [1] is the field pair).
        let Struct::List(rec_kids) = back.get(kids[2]) else {
            panic!("record is a list");
        };
        assert!(
            back.field_pair_parts(rec_kids[1]).is_some(),
            "a RecordCtor's child is a FieldPair after decode"
        );
    }

    /// Helper: the ctor_head_key of a lone NAME atom spelled `s` (for the negative case).
    fn b2_head_tag(s: &str) -> Option<CompoundCtor> {
        let mut b = Builder::new();
        let h = b.name(s);
        let root = b.list(vec![h]);
        let a = b.finish(root);
        a.ctor_head_key(h)
    }

    fn dec(neg: bool, sig: &str, exp: i64) -> Decimal {
        Decimal {
            negative: neg,
            significand: IntValue::from_bigint(&BigInt::from_str(sig).unwrap()).magnitude,
            exponent: exp,
        }
    }

    #[test]
    fn intvalue_bigint_bridge_round_trips_and_canonicalizes() {
        // The IntValue<->BigInt bridge (used while the front-end migrates off num-bigint) must be an
        // exact inverse and preserve IntValue's canonical minimal-magnitude form. Covers zero (empty
        // magnitude, non-negative), both signs, and a >u128 magnitude (arbitrary precision).
        for s in [
            "0",
            "1",
            "-1",
            "255",
            "256",
            "-256",
            "9223372036854775808",
            "-9223372036854775809",
            "340282366920938463463374607431768211456",
            "-123456789012345678901234567890",
        ] {
            let b = BigInt::from_str(s).unwrap();
            let iv = IntValue::from_bigint(&b);
            assert_eq!(iv.to_bigint(), b, "IntValue<->BigInt round-trip for {s}");
            // Canonical form: zero is a non-negative empty magnitude; a non-zero value has no leading
            // zero byte.
            if b == BigInt::from(0) {
                assert!(
                    !iv.negative && iv.magnitude.is_empty(),
                    "zero is empty + non-negative"
                );
            } else {
                assert_ne!(iv.magnitude.first(), Some(&0), "no leading zero byte: {s}");
            }
        }
        // from_bigint(iv.to_bigint()) is also identity on a hand-built IntValue.
        let iv = IntValue::from_i64(-42);
        assert_eq!(IntValue::from_bigint(&iv.to_bigint()), iv);
    }

    #[test]
    fn decimal_from_f64_is_the_exact_shortest_decimal_decomposition() {
        // The (sign, significand, exponent) each f64 decomposes to — matching rcdzc `Decimal::from_f64`
        // + the runtime `float_leaf` (`{f:.0}` whole / `{:e}` shortest), the basis of the 3-codec Float
        // byte-identity. A WHOLE value keeps its full integer expansion (100 → 100·10^0, not 1·10^2); a
        // non-whole folds the fraction digits into the exponent (1.5 → 15·10^-1). Sign is separate so
        // -0.0 stays distinct. nan/inf have no canonical form → None (the encode declines).
        assert_eq!(Decimal::from_f64(1.5), Some(dec(false, "15", -1)));
        assert_eq!(Decimal::from_f64(-0.25), Some(dec(true, "25", -2)));
        assert_eq!(Decimal::from_f64(100.0), Some(dec(false, "100", 0)));
        assert_eq!(Decimal::from_f64(0.0), Some(dec(false, "0", 0)));
        assert_eq!(Decimal::from_f64(-0.0), Some(dec(true, "0", 0)));
        assert_eq!(Decimal::from_f64(2.0), Some(dec(false, "2", 0)));
        assert_eq!(Decimal::from_f64(f64::NAN), None);
        assert_eq!(Decimal::from_f64(f64::INFINITY), None);
        assert_eq!(Decimal::from_f64(f64::NEG_INFINITY), None);
        // Round-trips to the same bits (the decomposition is exact, not lossy): reconstruct
        // `<sig>e<exp>` and re-parse.
        for &f in &[
            1.5f64, -0.25, 100.0, 1.23456, 1e20, 6.022e23, -7.0, 0.0, -0.0,
        ] {
            let d = Decimal::from_f64(f).unwrap();
            let s = format!(
                "{}{}e{}",
                if d.negative { "-" } else { "" },
                IntValue {
                    negative: false,
                    magnitude: d.significand.clone()
                }
                .to_decimal_string(),
                d.exponent
            );
            assert_eq!(
                f64::from_str(&s).unwrap().to_bits(),
                f.to_bits(),
                "from_f64({f}) reconstructed as {s} did not round-trip"
            );
        }
    }

    #[test]
    fn decimal_from_f32_formats_the_f32_directly_not_the_promoted_f64() {
        // A promoted f32's shortest decimal differs from the f64's: 0.1f32's shortest is `1e-1`, matching
        // the runtime `float_leaf_f32` (always `{:e}`), NOT the f64 expansion of `0.1f32 as f64`.
        assert_eq!(Decimal::from_f32(0.5f32), Some(dec(false, "5", -1)));
        assert_eq!(Decimal::from_f32(0.1f32), Some(dec(false, "1", -1)));
        assert_eq!(Decimal::from_f32(f32::NAN), None);
    }

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
    fn arc_payload_leaves_dedup_by_content_across_distinct_allocations() {
        // The general `leaf_index` is `FxHashMap<Leaf, LeafId>`, so leaf dedup relies on `Leaf`'s
        // `Eq`/`Hash` being CONTENT-based. For the Arc payloads (`Str`/`Sym` = Arc<str>, `Bytes` =
        // Arc<[u8]>) that holds because std's `Arc<T>: Eq/Hash` DELEGATES to `T` (deref to str/[u8]) —
        // NOT pointer identity. This pins that invariant: two leaves built from SEPARATE allocations of
        // the same content MUST intern to the SAME id. If a future change ever wrapped the payload in a
        // pointer-identity Eq/Hash (or reverted the deref-delegation assumption), dedup would silently
        // break — 500 `b"..."` literals would become 500 leaves instead of one, and the whole
        // interning/cheap-clone win would regress with no other test noticing (my cheap-clone test uses
        // Arc::clone = the SAME allocation, so it can't catch a content-vs-pointer dedup regression).
        let mut b = Builder::new();
        // Bytes: two DISTINCT Arc<[u8]> allocations with identical bytes.
        let a1: Arc<[u8]> = Arc::from(&b"\x00\xff payload"[..]);
        let a2: Arc<[u8]> = Arc::from(&b"\x00\xff payload"[..]);
        assert!(
            !Arc::ptr_eq(&a1, &a2),
            "the test needs two SEPARATE allocations to be meaningful"
        );
        let id1 = b.leaf(Leaf::Bytes(a1));
        let id2 = b.leaf(Leaf::Bytes(a2));
        assert_eq!(
            id1, id2,
            "equal-content Bytes leaves from distinct allocations must dedup to one id"
        );
        // A DIFFERENT byte content is a distinct leaf (dedup is by content, not blanket-collapse).
        assert_ne!(b.leaf(Leaf::Bytes(Arc::from(&b"other"[..]))), id1);
        // Str: same content-dedup across distinct Arc<str> allocations (String::from avoids any interning
        // shortcut a shared literal might take).
        let s1 = b.leaf(Leaf::Str(Arc::from(String::from("hello").as_str())));
        let s2 = b.leaf(Leaf::Str(Arc::from(String::from("hello").as_str())));
        assert_eq!(
            s1, s2,
            "equal-content Str leaves from distinct allocations must dedup to one id"
        );
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
                    value: IntValue::from_i64(1),
                    radix: Radix::Dec,
                }],
            );
            let str_headed = form(
                Leaf::Str(ctor.into()),
                &[Leaf::Int {
                    value: IntValue::from_i64(1),
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
    fn structurally_eq_collapses_a_native_ctor_leaf_head_with_the_name_and_string_spellings() {
        // node_eq NORMALIZES the head KIND for the five compound ctors (`ctor_head_key`): the native
        // ctor-LEAF head (`Leaf::Ctor`, the M2 primitive), the shadowable NAME alias, and the legacy
        // STRING primitive of the same spelling ALL collapse to one head. So two same-ctor trees are equal
        // regardless of which of the three head spellings each uses — head-KIND never splits structural
        // identity (the documented `structurally_eq` contract), which is what lets a native-head value and
        // a still-legacy alias/string-head value compare equal across the M2 migration (e.g. an un-migrated
        // corpus record vs a `read_ml` native record). Different ctor KINDS still differ. This does NOT
        // weaken byte content-addressing: the codec emits distinct bytes per head kind, so their HASHES
        // differ — only this lenient structural comparison normalizes. See
        // `DESIGN-native-ast-compound-data.md` (node_eq: head-kind is normalized for the compound ctors).
        let one = &[Leaf::Int {
            value: IntValue::from_i64(1),
            radix: Radix::Dec,
        }];
        let list_a = form(Leaf::Ctor(CompoundCtor::List), one);
        let list_b = form(Leaf::Ctor(CompoundCtor::List), one);
        let set = form(Leaf::Ctor(CompoundCtor::Set), one);
        let str_list = form(Leaf::Str("list".into()), one);
        let name_list = form(Leaf::Name("list".into()), one);
        // Same ctor kind → equal, and symmetric.
        assert!(
            list_a.structurally_eq(&list_b),
            "same ctor-leaf kind is equal"
        );
        assert!(list_b.structurally_eq(&list_a), "equality is symmetric");
        // Different ctor kinds → distinct.
        assert!(
            !list_a.structurally_eq(&set),
            "List-ctor and Set-ctor heads must differ"
        );
        // A native ctor-leaf head COLLAPSES with the legacy string/name head of the same ctor (head-kind
        // normalization), in BOTH directions.
        assert!(
            list_a.structurally_eq(&str_list),
            "ctor-leaf head collapses with the string-primitive head of the same ctor"
        );
        assert!(str_list.structurally_eq(&list_a), "collapse is symmetric");
        assert!(
            list_a.structurally_eq(&name_list),
            "ctor-leaf head collapses with the name-alias head of the same ctor"
        );
        // The two field-pair / member marker leaves are likewise their own identities.
        let fp = form(Leaf::FieldPair, one);
        let member = form(Leaf::Member, one);
        let eq_name = form(Leaf::Name("=".into()), one);
        assert!(
            !fp.structurally_eq(&member),
            "FieldPair and Member heads differ"
        );
        assert!(
            !fp.structurally_eq(&eq_name),
            "the FieldPair leaf head is distinct from a Name(\"=\") head"
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
        // `head_name`/`as_form` read a NAME head; `head_ctor` reads a STRING head. A string-headed form
        // has no name head (and vice-versa), so the accessors don't cross over.
        let str_headed = form(Leaf::Str("record".into()), &[Leaf::Bool(false)]);
        assert_eq!(str_headed.head_ctor(str_headed.root), Some("record"));
        assert_eq!(str_headed.head_name(str_headed.root), None);
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
                value: IntValue::from_i64(rng.next() as i64),
                radix: [Radix::Dec, Radix::Hex, Radix::Bin][rng.below(3)],
            },
            1 => Leaf::Float(Decimal {
                negative: rng.next() & 1 == 0,
                significand: IntValue::from_i64((rng.next() % 10_000) as i64).magnitude,
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
                    value: IntValue::from_i64((rng.next() % 1000) as i64),
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
        let root = b.effect_schema_tree("Weather", &[("get", sig)]);
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
        let root2 = b2.effect_schema_tree("Weather", &[("get", sig2)]);
        let built2 = b2.finish(root2);
        assert_eq!(
            bytes1,
            crate::codec::encode(&built2),
            "two builds of the same schema encode byte-identically (stable identity)"
        );
    }

    #[test]
    fn effect_schema_tree_carries_ops_in_order_data_shape_only() {
        // Multiple ops in caller order; the structural heads (effect/op) are NAME atoms so
        // `as_form`/`schema_declared_name` read them. The schema is DATA SHAPE ONLY — there is NO authz
        // node (operator directive: grants are dynamic, external to the schema).
        let mut b = Builder::new();
        let (s1, s2) = (b.name("string"), b.name("u8"));
        let root = b.effect_schema_tree("Fs", &[("read", s1), ("write", s2)]);
        let built = b.finish(root);
        assert_eq!(built.schema_declared_name(), Some("Fs"));
        // Two ops present as `(op read …)` / `(op write …)`, and NO authz tail.
        let tail = built.as_form(built.root, "effect").expect("effect form");
        assert_eq!(tail.len(), 3, "name + 2 ops, no authz slot");
        assert!(
            built.as_form(tail[1], "op").is_some(),
            "first op is an (op …) form"
        );
        assert!(
            built.as_form(tail[2], "op").is_some(),
            "second op is an (op …) form"
        );
        // A single-op schema is name + 1 op, no authz slot.
        let mut b2 = Builder::new();
        let s = b2.name("string");
        let r2 = b2.effect_schema_tree("Fs", &[("read", s)]);
        let one_op = b2.finish(r2);
        let tail2 = one_op.as_form(one_op.root, "effect").expect("effect form");
        assert_eq!(tail2.len(), 2, "name + 1 op, no authz slot");
    }

    // Build the reducer world `(world Reducer (export fold (member apply (func (param event (list (u8)))
    // (result (list (u8)))))) (import kv (member get (func (param key (string)) (result (string))))))`
    // through the canonical builders — the concrete v-ah reducer example. Returns the built arena.
    fn reducer_world(b: &mut Builder) -> StructId {
        // Signature descriptor stand-ins (in practice the kernel's `build_type` emits these). A `(list
        // (u8))` and a `(string)` — arbitrary descriptor nodes; the builder does not interpret them.
        let bytes_desc = |b: &mut Builder| {
            let (l, u8h) = (b.name("list"), b.name("u8"));
            let u8n = b.list(vec![u8h]);
            b.list(vec![l, u8n])
        };
        let str_desc = |b: &mut Builder| {
            let s = b.name("string");
            b.list(vec![s])
        };
        // export fold { apply: (event: bytes) -> bytes }
        let (ev, res) = (bytes_desc(b), bytes_desc(b));
        let apply_sig = b.wit_func_sig(&[("event", ev)], res);
        let fold = b.wit_interface(WitDir::Export, "fold", &[("apply", apply_sig)]);
        // import kv { get: (key: string) -> string }
        let (k, r) = (str_desc(b), str_desc(b));
        let get_sig = b.wit_func_sig(&[("key", k)], r);
        let kv = b.wit_interface(WitDir::Import, "kv", &[("get", get_sig)]);
        b.world_schema_tree("Reducer", &[fold, kv])
    }

    #[test]
    fn world_schema_tree_builds_the_canonical_shape_with_import_export_directions() {
        // The locked WIT-world node shape (converged w/ v-agent-harness, 2026-08-11): `(world Name
        // <interface>…)` where each interface is `(import|export IfaceName (member MName (func (param
        // PName Desc)… (result Desc)))…)`. All structure heads (world/import/export/member/func/param/
        // result) are NAME atoms (head-kind-fixed like `effect_schema_tree`, so identical worlds encode
        // byte-identically). Direction is STRUCTURAL (import vs export sub-head), not a member attribute.
        let mut b = Builder::new();
        let root = reducer_world(&mut b);
        let built = b.finish(root);

        // `(world Reducer <export-iface> <import-iface>)` — head is a NAME, name reads back, 2 interfaces.
        let world = built.as_form(built.root, "world").expect("world form");
        assert_eq!(world.len(), 3, "name + 2 interfaces");
        assert_eq!(built.as_name(world[0]), Some("Reducer"));

        // Interface 0: `(export fold (member apply <func>))`.
        let fold = built.as_form(world[1], "export").expect("export interface");
        assert_eq!(built.as_name(fold[0]), Some("fold"));
        let apply = built.as_form(fold[1], "member").expect("member form");
        assert_eq!(built.as_name(apply[0]), Some("apply"));
        // The member's signature is a `(func (param event …) (result …))` — param present, result present.
        let func = built.as_form(apply[1], "func").expect("func form");
        assert_eq!(
            func.len(),
            2,
            "one param sub-node + the (always-present) result"
        );
        let param = built.as_form(func[0], "param").expect("param form");
        assert_eq!(built.as_name(param[0]), Some("event"));
        assert!(
            built.as_form(func[1], "result").is_some(),
            "result sub-node present"
        );

        // Interface 1: `(import kv (member get …))` — direction is the sub-head, structurally distinct.
        let kv = built.as_form(world[2], "import").expect("import interface");
        assert_eq!(built.as_name(kv[0]), Some("kv"));
        assert!(built.as_form(kv[1], "member").is_some(), "kv has a member");
    }

    #[test]
    fn world_schema_tree_identity_is_byte_stable_across_independent_builds() {
        // Two independent builds of the SAME world encode BYTE-identically — the world identity is
        // `Hash::of(codec::encode(root))` (mirroring the effect-schema identity), and the head-kind-fixed
        // NAME atoms mean no Name/Str drift can split an otherwise-identical world's content address. This
        // is the property the THREE world sources (external artifact, inline decl, v-cml emit) rely on to
        // agree a target world is the same tree regardless of who produced it.
        let mut b1 = Builder::new();
        let r1 = reducer_world(&mut b1);
        let w1 = b1.finish(r1);
        let mut b2 = Builder::new();
        let r2 = reducer_world(&mut b2);
        let w2 = b2.finish(r2);
        assert_eq!(
            crate::codec::encode(&w1),
            crate::codec::encode(&w2),
            "two builds of the same world must encode byte-identically (stable identity)"
        );
    }

    #[test]
    fn distinct_worlds_encode_to_distinct_bytes() {
        // The DISCRIMINATION half of the world-identity contract (symmetric to
        // `distinct_effect_schemas_encode_to_distinct_bytes` for effect schemas): worlds that differ in
        // ANY identity-bearing position must encode to DIFFERENT bytes, so the world content-hash never
        // COLLIDES distinct worlds (an import-world with an export-world, or two worlds differing only by
        // a param name). Without this, a future encode/builder change that dropped an identity-bearing
        // field would pass every same->same test while silently collapsing distinct worlds to one hash.
        // Each of world name / direction / interface name / member name / param name / result type
        // occupies a distinct encodable position; perturb each and assert the bytes change.
        //
        // One tiny descriptor per position — the builder does not interpret it, so a bare `(t)` suffices.
        let ty = |b: &mut Builder, n: &str| {
            let h = b.name(n);
            b.list(vec![h])
        };
        // Build `(world <wname> (<dir> <iface> (member <member> (func (param <param> (pty)) (result (rty))))))`.
        let build = |wname, dir, iface, member, param, pty, rty| {
            let mut b = Builder::new();
            let (p, r) = (ty(&mut b, pty), ty(&mut b, rty));
            let sig = b.wit_func_sig(&[(param, p)], r);
            let i = b.wit_interface(dir, iface, &[(member, sig)]);
            let root = b.world_schema_tree(wname, &[i]);
            crate::codec::encode(&b.finish(root))
        };
        let base = build("W", WitDir::Export, "iface", "m", "p", "A", "B");
        // Each variation perturbs exactly one identity-bearing position.
        assert_ne!(
            base,
            build("W2", WitDir::Export, "iface", "m", "p", "A", "B"),
            "world name"
        );
        assert_ne!(
            base,
            build("W", WitDir::Import, "iface", "m", "p", "A", "B"),
            "direction (import vs export)"
        );
        assert_ne!(
            base,
            build("W", WitDir::Export, "iface2", "m", "p", "A", "B"),
            "interface name"
        );
        assert_ne!(
            base,
            build("W", WitDir::Export, "iface", "m2", "p", "A", "B"),
            "member name"
        );
        assert_ne!(
            base,
            build("W", WitDir::Export, "iface", "m", "p2", "A", "B"),
            "param name"
        );
        assert_ne!(
            base,
            build("W", WitDir::Export, "iface", "m", "p", "A2", "B"),
            "param type"
        );
        assert_ne!(
            base,
            build("W", WitDir::Export, "iface", "m", "p", "A", "B2"),
            "result type"
        );
    }

    #[test]
    fn world_schema_tree_nullary_member_has_an_explicit_present_result() {
        // A no-parameter, no-meaningful-return member is `(func (result <unit>))` — ZERO param sub-nodes
        // but the result sub-node is ALWAYS present (a `unit` descriptor, never an omitted slot), so the
        // func shape is uniform (no optional-slot presence marker that could drift the byte-exact
        // identity). Pin that a nullary func is exactly `(func (result …))`.
        let mut b = Builder::new();
        let unit = {
            let u = b.name("unit");
            b.list(vec![u])
        };
        let sig = b.wit_func_sig(&[], unit);
        let iface = b.wit_interface(WitDir::Export, "clock", &[("now", sig)]);
        let root = b.world_schema_tree("W", &[iface]);
        let built = b.finish(root);
        let iface_form = built
            .as_form(built.as_form(built.root, "world").unwrap()[1], "export")
            .unwrap();
        let member = built.as_form(iface_form[1], "member").unwrap();
        let func = built.as_form(member[1], "func").expect("func form");
        assert_eq!(
            func.len(),
            1,
            "zero params, just the (always-present) result sub-node"
        );
        assert!(
            built.as_form(func[0], "result").is_some(),
            "the sole sub-node is the result"
        );
    }

    #[test]
    fn wit_type_descriptors_match_the_canonical_str_head_build_type_form() {
        // The shared WIT type-descriptor builders emit the LANDED `ast_marshal::build_type` form (the
        // anchor rcdzc's `parse_wit_type` already reads; v-agent-harness ruling 2026-08-12): a PRIMITIVE
        // is a NAME-head one-element list `(u8)`; a COMPOUND (`list`/`option`) is a STRING-head form
        // `("list" <elem>)` / `("option" <inner>)`. A Name-vs-Str head is what distinguishes prim from
        // compound, so the head KIND is load-bearing — pin it explicitly, not just the spelling.
        let mut b = Builder::new();
        // Primitive: `(u8)` — head is a NAME `u8`, exactly one child.
        let u8 = b.wit_type_prim("u8");
        // list<u8>: `("list" (u8))` — head is a STRING `list`, child is the u8 prim descriptor.
        let list_u8 = {
            let e = b.wit_type_prim("u8");
            b.wit_type_list(e)
        };
        // option<list<u8>>: `("option" ("list" (u8)))`.
        let opt = {
            let inner = {
                let e = b.wit_type_prim("u8");
                b.wit_type_list(e)
            };
            b.wit_type_option(inner)
        };
        let built = b.finish(u8); // finish needs a root; reuse the arena via accessors below
        // Primitive: a 1-element list whose head is a NAME atom.
        let Struct::List(prim_kids) = built.get(u8) else {
            panic!("prim is a list")
        };
        assert_eq!(
            prim_kids.len(),
            1,
            "a primitive descriptor is a one-element list"
        );
        assert_eq!(
            built.as_name(prim_kids[0]),
            Some("u8"),
            "prim head is a NAME atom"
        );

        // list: a 2-element list whose head is a STRING atom `list` (NOT a name), child is the element.
        let Struct::List(list_kids) = built.get(list_u8) else {
            panic!("list is a list")
        };
        assert_eq!(list_kids.len(), 2, "list<T> is head + element");
        assert_eq!(
            built.as_str(list_kids[0]),
            Some("list"),
            "list head is a STRING atom (compound marker), not a name"
        );
        assert!(
            built.as_name(list_kids[0]).is_none(),
            "list head is NOT a name atom"
        );

        // option: STRING head `option` + inner.
        let Struct::List(opt_kids) = built.get(opt) else {
            panic!("option is a list")
        };
        assert_eq!(opt_kids.len(), 2, "option<T> is head + inner");
        assert_eq!(
            built.as_str(opt_kids[0]),
            Some("option"),
            "option head is a STRING atom"
        );
    }

    #[test]
    fn wit_type_record_and_variant_match_the_canonical_str_head_form() {
        // The aggregate WIT type-descriptor builders emit the LANDED `ast_marshal::build_type` form the
        // compiler's `parse_wit_type` reads: a RECORD is a STRING-head `("record" (fname <ty>)…)` whose
        // every field is a `(name-atom ty)` 2-list; a VARIANT is a STRING-head `("variant" (Case <ty>?)…)`
        // whose payload-bearing case is a `(CaseName ty)` 2-list and payload-less case a `(CaseName)`
        // 1-list. The compound head is a STRING atom (distinct from a NAME-head primitive), and the
        // field/case names ride as NAME atoms — pin the head KIND and the sub-shape, both load-bearing for
        // the byte-exact identity these descriptors' content-hash keys on.
        let mut b = Builder::new();
        // record { x: u8, y: string } — fields in caller (name-sorted) order.
        let (xt, yt) = (b.wit_type_prim("u8"), b.wit_type_prim("string"));
        let rec = b.wit_type_record(&[("x", xt), ("y", yt)]);
        // variant { Some(u8), None } — one payload-bearing case, one payload-less.
        let some_ty = b.wit_type_prim("u8");
        let var = b.wit_type_variant(&[("Some", Some(some_ty)), ("None", None)]);
        let built = b.finish(rec);

        // record: STRING head `record`, then one 2-list `(name ty)` per field.
        let Struct::List(rec_kids) = built.get(rec) else {
            panic!("record is a list")
        };
        assert_eq!(rec_kids.len(), 3, "record head + 2 field entries");
        assert_eq!(
            built.as_str(rec_kids[0]),
            Some("record"),
            "record head is a STRING atom (compound marker)"
        );
        assert!(
            built.as_name(rec_kids[0]).is_none(),
            "record head is NOT a name atom"
        );
        let Struct::List(field0) = built.get(rec_kids[1]) else {
            panic!("field is a 2-list")
        };
        assert_eq!(field0.len(), 2, "a field is a (name ty) 2-list");
        assert_eq!(
            built.as_name(field0[0]),
            Some("x"),
            "field name rides as a NAME atom"
        );

        // variant: STRING head `variant`; payload-bearing case is a 2-list, payload-less a 1-list.
        let Struct::List(var_kids) = built.get(var) else {
            panic!("variant is a list")
        };
        assert_eq!(var_kids.len(), 3, "variant head + 2 case entries");
        assert_eq!(
            built.as_str(var_kids[0]),
            Some("variant"),
            "variant head is a STRING atom"
        );
        let Struct::List(some_case) = built.get(var_kids[1]) else {
            panic!("case is a list")
        };
        assert_eq!(
            some_case.len(),
            2,
            "a payload-bearing case is a (Case ty) 2-list"
        );
        assert_eq!(built.as_name(some_case[0]), Some("Some"));
        let Struct::List(none_case) = built.get(var_kids[2]) else {
            panic!("case is a list")
        };
        assert_eq!(none_case.len(), 1, "a payload-less case is a (Case) 1-list");
        assert_eq!(built.as_name(none_case[0]), Some("None"));
    }

    #[test]
    fn wit_type_record_variant_identity_is_byte_stable_and_discriminating() {
        // The aggregate descriptors obey the same content-address contract as the world tree: two
        // independent builds of the SAME descriptor encode BYTE-identically, and any identity-bearing
        // perturbation (field/case name, field/case ORDER, a case's payload presence) changes the bytes —
        // so a descriptor's content-hash never collides two structurally-distinct types. A prim per field
        // slot suffices; the builder does not interpret it.
        let prim = |b: &mut Builder, n: &str| b.wit_type_prim(n);
        // record { a: u8, b: u8 }
        let build_rec = |fa: &str, fb: &str| {
            let mut b = Builder::new();
            let (ta, tb) = (prim(&mut b, "u8"), prim(&mut b, "u8"));
            let r = b.wit_type_record(&[(fa, ta), (fb, tb)]);
            crate::codec::encode(&b.finish(r))
        };
        let rec_base = build_rec("a", "b");
        assert_eq!(
            rec_base,
            build_rec("a", "b"),
            "same record encodes byte-identically"
        );
        assert_ne!(
            rec_base,
            build_rec("a", "c"),
            "field name is identity-bearing"
        );
        assert_ne!(
            rec_base,
            build_rec("b", "a"),
            "field ORDER is identity-bearing (encode is positional)"
        );

        // variant { X(u8)?, Y } — vary case name and payload presence.
        let build_var = |cx: &str, x_payload: bool, cy: &str| {
            let mut b = Builder::new();
            let x_ty = if x_payload {
                Some(prim(&mut b, "u8"))
            } else {
                None
            };
            let v = b.wit_type_variant(&[(cx, x_ty), (cy, None)]);
            crate::codec::encode(&b.finish(v))
        };
        let var_base = build_var("X", true, "Y");
        assert_eq!(
            var_base,
            build_var("X", true, "Y"),
            "same variant encodes byte-identically"
        );
        assert_ne!(
            var_base,
            build_var("Z", true, "Y"),
            "case name is identity-bearing"
        );
        assert_ne!(
            var_base,
            build_var("X", false, "Y"),
            "a case's payload PRESENCE is identity-bearing ((Case ty) vs (Case))"
        );
    }

    #[test]
    fn wit_type_enum_flags_result_match_the_pinned_reader_form() {
        // The tagged aggregate builders mirror rcdzc's landed `parse_wit_type` (wit_world.rs) byte-for-byte:
        // - enum : `("enum" Case…)`  — STR head, each case a BARE NAME leaf (payload-less by construction).
        // - flags: `("flags" Name…)` — SAME node shape as enum but a DISTINCT type (the str head separates).
        // - result: `("result" <ok-slot> <err-slot>)` — EXACTLY two slots; a present arm is its descriptor,
        //   an omitted arm is the absent-marker `("none")` (a STR-head 1-list, distinct from `("unit")`).
        let mut b = Builder::new();
        let en = b.wit_type_enum(&["Red", "Green"]);
        let fl = b.wit_type_flags(&["Read", "Write"]);
        // result<u8, string>, result<u8> (no err), result (no arms).
        let (okt, errt) = (b.wit_type_prim("u8"), b.wit_type_prim("string"));
        let res_full = b.wit_type_result(Some(okt), Some(errt));
        let ok_only = b.wit_type_prim("u8");
        let res_no_err = b.wit_type_result(Some(ok_only), None);
        let res_bare = b.wit_type_result(None, None);
        let built = b.finish(en);

        // enum: STR head `enum`, then bare NAME cases (NOT wrapped in a list).
        let Struct::List(en_kids) = built.get(en) else {
            panic!("enum is a list")
        };
        assert_eq!(
            built.as_str(en_kids[0]),
            Some("enum"),
            "enum head is a STRING atom"
        );
        assert_eq!(
            built.as_name(en_kids[1]),
            Some("Red"),
            "enum case is a bare NAME leaf"
        );
        // flags: STR head `flags`, same bare-name shape.
        let Struct::List(fl_kids) = built.get(fl) else {
            panic!("flags is a list")
        };
        assert_eq!(
            built.as_str(fl_kids[0]),
            Some("flags"),
            "flags head is a STRING atom"
        );
        assert_eq!(
            built.as_name(fl_kids[1]),
            Some("Read"),
            "flags name is a bare NAME leaf"
        );

        // result: EXACTLY 3 children (head + 2 slots), regardless of arm presence.
        for r in [res_full, res_no_err, res_bare] {
            let Struct::List(r_kids) = built.get(r) else {
                panic!("result is a list")
            };
            assert_eq!(
                r_kids.len(),
                3,
                "result is head + exactly two slots (fixed arity)"
            );
            assert_eq!(
                built.as_str(r_kids[0]),
                Some("result"),
                "result head is a STRING atom"
            );
        }
        // A present arm is its descriptor; an omitted arm is the `("none")` marker (distinct from unit).
        let Struct::List(full_kids) = built.get(res_full) else {
            panic!()
        };
        assert_eq!(
            built.head_name(full_kids[1]),
            Some("u8"),
            "present ok arm is its descriptor"
        );
        assert_ne!(
            built.head_ctor(full_kids[1]),
            Some("none"),
            "a present arm is NOT the none-marker"
        );
        let Struct::List(no_err_kids) = built.get(res_no_err) else {
            panic!()
        };
        assert_eq!(
            built.head_ctor(no_err_kids[2]),
            Some("none"),
            "an omitted err arm is the ('none') absent-marker"
        );
        assert_ne!(
            built.head_ctor(no_err_kids[2]),
            Some("unit"),
            "the absent-marker is distinct from ('unit') (a present unit-typed arm)"
        );
    }

    #[test]
    fn wit_type_enum_flags_result_identity_is_byte_stable_and_discriminating() {
        // The tagged aggregates obey the content-address contract: same input → byte-identical encoding, and
        // any identity-bearing perturbation (case ORDER, enum-vs-flags of the same names, a result arm's
        // presence, or swapping its ok/err) changes the bytes.
        let enc_enum = |cases: &[&str]| {
            let mut b = Builder::new();
            let e = b.wit_type_enum(cases);
            crate::codec::encode(&b.finish(e))
        };
        assert_eq!(
            enc_enum(&["A", "B"]),
            enc_enum(&["A", "B"]),
            "same enum encodes byte-identically"
        );
        assert_ne!(
            enc_enum(&["A", "B"]),
            enc_enum(&["B", "A"]),
            "case ORDER is identity-bearing"
        );

        // enum { A } vs flags { A } — SAME names, DIFFERENT type: the str head must discriminate.
        let enc_flags = |names: &[&str]| {
            let mut b = Builder::new();
            let f = b.wit_type_flags(names);
            crate::codec::encode(&b.finish(f))
        };
        assert_ne!(
            enc_enum(&["A"]),
            enc_flags(&["A"]),
            "enum and flags of the same names encode distinctly (head discriminates)"
        );

        // result: arm presence and ok/err ORDER are identity-bearing.
        let enc_result = |ok: Option<&str>, err: Option<&str>| {
            let mut b = Builder::new();
            let o = ok.map(|n| b.wit_type_prim(n));
            let e = err.map(|n| b.wit_type_prim(n));
            let r = b.wit_type_result(o, e);
            crate::codec::encode(&b.finish(r))
        };
        let full = enc_result(Some("u8"), Some("string"));
        assert_eq!(
            full,
            enc_result(Some("u8"), Some("string")),
            "same result encodes byte-identically"
        );
        assert_ne!(
            full,
            enc_result(Some("u8"), None),
            "err-arm PRESENCE is identity-bearing"
        );
        assert_ne!(
            full,
            enc_result(Some("string"), Some("u8")),
            "ok/err ORDER is identity-bearing (positional slots)"
        );
        assert_ne!(
            enc_result(Some("u8"), None),
            enc_result(None, Some("u8")),
            "result<T> and result<_, E> with the same T are distinct (which arm is absent matters)"
        );
    }

    #[test]
    fn decimal_f64_bits_round_trips_from_f64() {
        // `to_f64_bits` is the inverse of `from_f64` for every finite f64 — the property both the
        // compiler fold and the runtime op93 float path rely on (they share this file). A whole value,
        // a fraction, a negative, a negative zero, and a subnormal-ish magnitude all round-trip bit-exact.
        for f in [
            0.0f64,
            -0.0,
            1.0,
            -1.0,
            1.5,
            0.1,
            300.0,
            -2.5e-1,
            1e300,
            1e-300,
            0.30000000000000004,
        ] {
            let d = Decimal::from_f64(f).expect("finite f64 has a Decimal");
            assert_eq!(
                d.to_f64_bits(),
                f.to_bits(),
                "from_f64({f}).to_f64_bits() must equal {f}'s bits"
            );
            assert!(d.is_finite_f64(), "{f} is finite");
        }
        // A non-finite f64 has no Decimal form (the encode declines) — matches the runtime trap.
        assert!(Decimal::from_f64(f64::INFINITY).is_none());
        assert!(Decimal::from_f64(f64::NAN).is_none());
        // fits_f32: a small value fits; a magnitude past the largest finite f32 (~3.4e38) does not.
        assert!(Decimal::from_f64(1.5).unwrap().fits_f32());
        assert!(!Decimal::from_f64(1e40).unwrap().fits_f32());
    }
}
