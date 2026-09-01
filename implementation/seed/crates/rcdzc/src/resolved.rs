//! The resolved node form — one entry of the resolved column, keyed by AST `StructId`.
//!
//! This is the tree-above-the-core rung, but it is NOT a separate arena: a node's resolved form is a
//! *column* over the AST's own identity (`query-engine.md` §The Compiler's State Is Columns Indexed
//! By Node Identity). `resolved_of(id)` fills the slot for one node; the node references its children
//! by their AST `StructId`, so descending into a child is the same lazy column read on a different
//! id. The source's nesting is therefore preserved (a child is reached through the parent) without
//! copying the tree into a second arena.
//!
//! Resolving a node is per-node and does NOT recurse: it classifies the AST occurrence at `id` and
//! records what it denotes, leaving the children as ids for a later demand to resolve. A "no" is a
//! value here — an unrecognized or malformed construct is [`Resolved::Poison`] — so the resolved
//! column is total over every node a query reaches.
//!
//! A bare name resolves — by the lexical-scope walk (`resolve::scope`) then the prelude map — to what
//! it denotes: a [`Resolved::Ref`] to the value occurrence it is bound to, or a `Poison` if unbound.
//! A member KEY is never resolved this way: it is a [`Symbol`] label (its spelling), read without any
//! scope/prelude lookup (`prelude-and-resolution.md` §A Member Key Is A Label, Not A Value).

use crate::ast::{IntValue, StructId};
use crate::diag::Reject;
use std::collections::BTreeMap;

/// A field/variant/member label — a name taken as data, NOT resolved to a value. A member access's
/// key and a record literal's field names are symbols: the projection finds a field BY this label and
/// never inspects a bound value for it.
///
/// A symbol carries an optional NAMESPACE so a name the language defines and a name a macro introduces
/// cannot collide (`contracts/ast-encoding.md` §A Prelude Symbol Is Namespaced). Ordering is by
/// (namespace, name), so a `BTreeMap` keyed by `Symbol` has a canonical field order — which is what
/// makes record equality and projection order-independent (a record's fields are a SET).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Symbol {
    /// The namespace this label belongs to, or `None` for an unqualified source name. Source names
    /// are unqualified today; the field exists so a macro-introduced label can carry its origin
    /// namespace when hygienic macros are added.
    ///
    /// `Arc<str>` (not `String`): a `Symbol` is the KEY of a record-field `BTreeMap<Symbol, _>`, which
    /// `Subst::apply`/`rename`/`freshen` REBUILD field-by-field on the hot type-checking path — cloning
    /// every key. As `String` that was a heap alloc per field per pass (~5% of a realistic compile,
    /// resolve.rs doc); as `Arc<str>` a clone is a refcount bump. `Arc<str>` orders + compares + hashes
    /// exactly like its `str` contents, so the name-lexicographic CANONICAL field order the record SET
    /// semantics + codec + value-heap cell layout depend on is preserved with no other change. Mirrors
    /// the `ConstStr`/`HostCall` `String`→`Rc<str>` cheap-clone slices. (cheap-clone audit, operator-
    /// greenlit; concierge: outcome>mechanism — kills the measured clone cost, not the literal "intern".)
    pub namespace: Option<std::sync::Arc<str>>,
    pub name: std::sync::Arc<str>,
}

impl Symbol {
    /// An unqualified label from a source spelling (no namespace).
    pub fn plain(name: impl Into<std::sync::Arc<str>>) -> Symbol {
        Symbol {
            namespace: None,
            name: name.into(),
        }
    }
}

/// A NATIVE primitive — the irreducible bottom a `Meta.apply` (or a leaf value) names. Everything
/// user-facing is a record; a `Prim` is where the compiler's own machinery takes over ("bottom out on
/// an intrinsic, don't bloat the general node types"). There are two families:
///  - arithmetic operations (`+`/`-`/`*`) — `Meta.apply` of the operator records; folded/emitted in
///    `lower`/`select` by the width read off the solved type;
///  - type CONSTRUCTORS (`Int`/`UInt`) — `Meta.apply` builders the evaluator applies to a width to
///    build a concrete integer MODULE record, and the function-type constructor `->`.
///
/// A prelude `(intrinsic NAME)` node names one of these; the name→prim table is the ONE place a prim
/// spelling lives (the prelude authors it), so nothing downstream matches a source name.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Prim {
    Add,
    Sub,
    Mul,
    /// Truncating integer division `/` (toward zero) and remainder `%` (sign of the dividend). Both
    /// trap on a zero divisor and on the `MIN / -1` overflow (`numeric-model.md` §Overflow Is Defined),
    /// so a provable trap folds to CDZ0304 like `*`.
    Div,
    Rem,
    /// Left shift `<<` — exact multiplication by `2^count`, so an overflowing shift traps like `*`, and
    /// a shift count outside `0..width` traps rather than masking (`numeric-model.md` §A Shift Is Not
    /// Exempt From Overflow Is Defined). Right shift `>>` is ARITHMETIC (sign-extending), also trapping
    /// on an out-of-range count.
    Shl,
    Shr,
    /// Bitwise `&` / `|` / `^` — total on the two's-complement value, never trap.
    BitAnd,
    BitOr,
    BitXor,
    /// The ordering comparisons `<` / `>` / `<=` / `>=` and equality `=` — each `∀a. a → a → Bool`.
    /// Unlike arithmetic (which is `∀a. (Int a) → (Int a) → (Int a)`), a comparison's result is `Bool`,
    /// and its operand is a BARE type variable — so it relates `Bool` as well as an integer (`(< false
    /// true)` = `true`), and STRUCTURALLY any value (a tuple, a map, a type-value). The I1 fold decides
    /// two constant SCALARS (`Int`/`Bool`); a compound or runtime operand declines (structural
    /// comparison over the value heap is a later stage) — the generic type stays, coverage grows behind
    /// a decline.
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    /// `compare` — the THREE-WAY comparison `∀a. a → a → Ordering` (core-semantics.md §A Total Order Is
    /// Observed Through A Three-Way Comparison). The primitive `<`/`>`/`=` agree with; its result is the
    /// built-in `Ordering` sum (Less=0/Equal=1/Greater=2). A constant SCALAR/string pair FOLDS to the
    /// matching `Ordering` variant (a `Core::SumNew` at the Ordering discs, like `List.at` builds Option);
    /// a compound or runtime operand declines (as the comparison prims do).
    Compare,
    /// The FLOATING-POINT arithmetic prims — each `∀a. (Float a) → (Float a) → (Float a)`, the
    /// width-generic float analogue of the integer `Add`/`Sub`/`Mul`/`Div`. These are INTERNAL — NOT a
    /// distinct surface operator. The author writes the ONE arithmetic operator `+`/`-`/`*`/`/`; when its
    /// operands solve to `Float`, `infer::apply_type` types the application `Float` and `lower` remaps the
    /// integer prim to the matching `F*` here (routing to `lower_float_arith`) — the same solved-type
    /// dispatch a `BigInt`/`Rational`/`Qty` operand rides (numeric-model.md §An Arithmetic Operator
    /// Requires Both Operands To Be One Numeric Type). A mixed integer/float application is CDZ0301 from
    /// that dispatch (the operands disagree), not a coercion. UNLIKE the integer arithmetic these NEVER
    /// trap on overflow — an IEEE result that leaves the finite range is an infinity, division by zero is
    /// ±inf/NaN. A constant pair FOLDS (round-to-nearest-even at the width); a runtime operand emits the
    /// machine `f64.add`/… (F4).
    FAdd,
    FSub,
    FMul,
    FDiv,
    /// Runtime FLOAT EQUALITY `== : Float^w → Float^w → Bool` under the CANONICAL BYTE FORM
    /// (`core-semantics.md` §Floating-Point Equality Follows The Canonical Byte Form): `nan == nan` TRUE,
    /// `-0.0 != +0.0`, all NaN equal. Distinct from the integer/bool `Eq` prim because the machine emit is
    /// a NaN-canonicalizing BIT compare, not IEEE `f64.eq`. A constant pair folds in `lower`; a runtime
    /// operand emits `Core::FloatCompare`.
    FEq,
    /// Runtime FLOAT ORDERING `< <= > >= : Float^w → Float^w → Bool` under IEEE PARTIAL order (operator
    /// ruling 2026-07-16): a NaN operand yields `false` (unordered — NaN is neither less, greater, nor
    /// equal to anything), and `-0.0` compares EQUAL-under-ordering to `+0.0` (`-0.0 <= +0.0` true, neither
    /// strictly less). This is RAW IEEE (`f64.lt/le/gt/ge`), NOT the canonical-byte compare `FEq` uses —
    /// the two relations DISAGREE on NaN + signed zero (bit-equality vs numeric-ordering, inherent to
    /// float). A constant pair folds in `lower` (NaN → false, not a decline); a runtime operand emits
    /// `Core::FloatCompare` with the ordering op → the raw IEEE machine compare.
    FLt,
    FLe,
    FGt,
    FGe,
    /// The TRUNCATING integer conversion `T.wrap : ∀(w,s). Int^s_w → T` — keeps the low `N` bits of the
    /// source's two's-complement value and interprets them at the TARGET width `N` and signedness. The
    /// source is a fully-polymorphic integer (any width/sign, via the operator record's type-lambda); the
    /// target is the MODULE's own width, read off the application's solved type at lowering. So there is
    /// ONE `Wrap` prim, not one per source type — no pair-explosion. It never traps and never returns an
    /// `Option`: truncation is total (`(UInt8.wrap 256) = 0`, `(UInt8.wrap -1) = 255`). Its CHECKED
    /// companion is `CheckedOf`.
    Wrap,
    /// The CHECKED integer conversion `T.of : ∀(w,s). Int^s_w → T` — the range-checked counterpart of
    /// `Wrap`. Same fully-polymorphic source + target-is-the-module's-own-width shape, but where `Wrap`
    /// truncates totally, `Of` TRAPS when the source value is outside the target type's range and
    /// otherwise returns the value UNCHANGED at the target type (`(UInt8.of 200) = 200`, `(UInt8.of 256)`
    /// traps, `(UInt8.of -1)` traps). It returns the bare `T`, NOT an `Option` — the overflow-FALLIBLE
    /// forms are the separate `checked-add`/`checked-mul` ops (numeric-model.md §A Conversion Between
    /// Integer Types Is Explicit: "a range-checked conversion that traps on a value outside the target
    /// type's range"). A CONSTANT operand FOLDS — in range → `Core::ConstInt`, out of range → `Core::Trap`.
    CheckedOf,
    /// The UNARY integer negation `T.neg : T → T` — the first-class named form of prefix `(- e)`, for a
    /// SIGNED integer module `T = (Int width)`. `(Int64.neg x)` = `0 - x` at the target width; a CONSTANT
    /// operand FOLDS to a negated `Core::ConstInt`, and negating the width's minimum (`0 - Int64.min`)
    /// TRAPS as a compile-provable overflow (CDZ0304) — the same fold/trap the prefix `(- e)` uses, since
    /// it lowers through `lower_negate` (`0 - e`). Offered only on the SIGNED widths; an unsigned negate
    /// would underflow-trap on every nonzero input, so it is not built.
    Neg,
    /// `Int : Nat → Module` — applied to a width, builds the signed integer module of that width.
    IntCtor,
    /// `UInt : Nat → Module` — the unsigned integer module builder.
    UIntCtor,
    /// `Float : Nat → Module` — applied to an ADMITTED width ({32,64}), builds the float module of that
    /// width (its `(meta t) = (Float N)` type-value + `of-int`/… fields). The float analogue of `IntCtor`;
    /// a width outside the admitted set reduces to the sentinel width 0 → CDZ0302, exactly as `(UInt 65)`.
    FloatCtor,
    /// `Float64.of-int` / `Float32.of-int` — the explicit INT→FLOAT conversion `Int64 → Float N` (the
    /// `(meta apply)` of a float module's `of-int` field). The float analogue of the integer `T.of`, but
    /// TOTAL (an integer always has a float image; a large magnitude ROUNDS to the nearest representable
    /// float under the fixed mode, it does not trap). A CONSTANT integer FOLDS to a `Core::ConstFloat`
    /// (the value as f64/f32); a runtime integer emits `f64.convert_i64_s`/`f32.convert_i64_s`. The
    /// TARGET width is the module's own, read off the application's solved `Ty::Float`. No implicit
    /// promotion — the conversion is always written (numeric-model.md §A Conversion Involving A
    /// Floating-Point Type Is Explicit).
    FloatOfInt,
    /// `Float64.of` / `Float32.of` — the explicit FLOAT-WIDTH conversion `Float M → Float N` (the `(meta
    /// apply)` of a float module's `of` field). `Float64.of` from a narrower float PROMOTES (widening,
    /// exact, `f64.promote_f32`); `Float32.of` from a wider float DEMOTES (narrowing, rounds to nearest
    /// under the fixed mode, `f32.demote_f64`); a same-width conversion is the identity. TARGET width =
    /// the module's own (read off the solved `Ty::Float`); SOURCE = the operand's own float width. A
    /// CONSTANT float FOLDS (round the exact `Decimal` at the target width); a runtime float emits the
    /// demote/promote op. The float-width companion of the integer `T.of`, but TOTAL (a float always has
    /// an image at another float width — no trap). No implicit promotion (numeric-model.md §A Conversion
    /// Involving A Floating-Point Type Is Explicit): a float↔float-width conversion (this `FloatOf`) and a
    /// float↔other-numeric conversion (`FloatOfInt` / a checked `Int.of` from a float) are each WRITTEN —
    /// the seed inserts none implicitly (a cross-width/cross-kind numeric mix is CDZ0301, no promotion).
    //= spec/capabilities/numeric-model.md#a-conversion-involving-a-floating-point-type-is-explicit
    //# A conversion between a floating-point type and any other numeric type, or between two floating-point types of different width, MUST be written explicitly rather than performed as an implicit promotion or narrowing.
    FloatOf,
    /// `nan` — the canonical not-a-number Float VALUE (a bare prelude name, like `unit`). NOT a literal
    /// (`Decimal` holds only finite values); it resolves to this prim and lowers to `Core::ConstFloatNan`,
    /// the ONE canonical NaN byte form. Every NaN equals every NaN and NaN differs from every finite float
    /// under the canonical-byte-form equality (`core-semantics.md` §Floating-Point Equality Follows The
    /// Canonical Byte Form) — the fold compares by `to_f64_bits`, so a single canonical NaN bit pattern
    /// makes `(= nan nan)` true. Types as `Ty::Float` (a bare `nan` grounds to Float64). A negative zero
    /// keeps its sign in the `Decimal` (`negative` bit), so `-0.0` serializes distinctly from `0.0`.
    //= spec/contracts/deterministic-value-form.md#numeric-values-serialize-deterministically
    //# Every floating-point not-a-number value MUST serialize to one canonical byte form, consistent with structural equality treating all not-a-number values as equal.
    //= spec/contracts/deterministic-value-form.md#numeric-values-serialize-deterministically
    //# A floating-point negative zero MUST serialize distinctly from a positive zero, consistent with structural equality treating them as distinct.
    // The NaN and negative-zero rules above are the two edge cases of the general canonical-byte-form
    // equality this fold enforces via `to_f64_bits`: same bits ⇒ same serialization, different bits ⇒
    // different serialization.
    //= spec/contracts/deterministic-value-form.md#numeric-values-serialize-deterministically
    //# Two floating-point values equal under structural equality MUST have identical canonical byte encodings.
    //= spec/contracts/deterministic-value-form.md#numeric-values-serialize-deterministically
    //# Two floating-point values that are not equal under structural equality MUST have distinct canonical byte encodings.
    //= spec/capabilities/numeric-model.md#floating-point-follows-the-determinism-contract
    //# A floating-point value MUST serialize under the canonical form fixed by the deterministic-value-form contract.
    //= spec/contracts/determinism-and-fuel.md#floating-point-emission-is-determinism-constrained
    //# The compiler MUST emit floating-point operations such that a not-a-number result has a canonical bit pattern rather than a runtime-dependent one.
    FloatNan,
    /// `Infinity` — the positive-INFINITY Float VALUE (a bare prelude name, like `nan`). NOT a literal
    /// (`Decimal` holds only finite values); it resolves to this prim and lowers to `Core::ConstFloatInf`,
    /// the canonical `f64::INFINITY` byte form. Unlike NaN it is fully ORDERED: `+∞ = +∞`, `+∞ > x` and
    /// `+∞ ≠ x` for every finite `x` — the fold compares by `to_f64_bits` for `=` and by IEEE order for
    /// `<`/`>`. Types as `Ty::Float` (a bare `Infinity` grounds to Float64; the float modules annotate it
    /// to their width, exactly as `nan`).
    FloatInf,
    /// `-> : (Type, Type) → Type` — the function-type constructor.
    FnCtor,
    /// `Tuple : (Type…) → Type` — the tuple-type constructor, VARIADIC over its element types. `(Tuple
    /// Int64 Bool)` builds the type-value `(Tuple Int64 Bool)`; a different arity or element type is a
    /// different type. Used in type position (an annotation `(: e (Tuple …))`), the arity/element check
    /// the annotation needs.
    TupleCtor,
    /// `Record : ((name Type)…) → Type` — the record-type constructor, VARIADIC over its `(name type)`
    /// field pairs. `(Record (a Int64) (b Bool))` builds the type-value `(Record (a Int64) (b Bool))`; the
    /// field-name SET and per-field types ARE the type. Used in type position (an annotation `(: e (Record
    /// …))`), giving the field-name/type check the annotation needs — the record companion of `TupleCtor`.
    RecordCtor,
    /// The ground type-values — nullary "constructors" that ARE a type-value directly (`Bool`/`Unit`/
    /// `String` resolve to a record whose `(meta t)` holds one of these).
    BoolTy,
    UnitTy,
    /// The ground `String` type-value — held in the `String` module record's `(meta t)`, so `(: x
    /// String)` reduces `String` (the record) to `Ty::String` via `typeval_of`. (A NULLARY type, like
    /// `Bool`/`Unit`; the `String` module ALSO carries operation fields, but its type role is this.)
    StringTy,
    /// The ground `Char` type-value — held in the `Char` module record's `(meta t)`, so bare `Char` in
    /// type position reduces to `Ty::Char` (a NULLARY type, like `String`; the `Char` module also carries
    /// `to-int`/`from-int` operation fields, but its type role is this).
    CharTy,
    /// `Char.to-int` — the TOTAL conversion of a char to its integer scalar value (`Char → Int64`,
    /// `collections-and-text.md` §A Char Converts To And From An Integer Totally). Folds a constant char
    /// to a `Core::ConstInt` of its code point.
    CharToInt,
    /// `Char.from-int` — the FALLIBLE conversion of an integer to a char (`Int64 → (Option Char)`): `Some`
    /// for a value that is a Unicode scalar, `None` for a surrogate / out-of-range integer. Folds a
    /// constant integer to `Some`/`None`.
    CharFromInt,
    /// `Value.encode` — the in-fold binary-AST value-form encode (R2): `∀a. a → Bytes`, TOTAL. Walks any
    /// value to its canonical binary-AST document bytes (the same value-form the export boundary emits),
    /// callable mid-fold. Lowers to `Core::ValueEncode` (runtime `value-encode` op) or a constant fold.
    ValueEncode,
    /// `Value.decode` — the inverse (R2): `∀a. Bytes → (Option a)`, PARTIAL. Parses a binary-AST document
    /// back into a value of the call-site expected type (`None` on shape/type mismatch). Lowers to
    /// `Core::ValueDecode`. The target type is grounded by typing (an unsolved `a` declines at the decode node).
    ValueDecode,
    /// The ground `Symbol` type-value — held in the `Symbol` module record's `(meta t)`, so bare `Symbol`
    /// in type position reduces to `Ty::Symbol` (a NULLARY type, like `String`/`Char`; the `Symbol` module
    /// also carries `of`/`to-string` operation fields, but its type role is this).
    SymbolTy,
    /// The ground `BigInt` type-value — held in the `BigInt` module record's `(meta t)`, so bare `BigInt`
    /// in type position reduces to `Ty::BigInt` (a NULLARY type, like `String`/`Symbol`; the `BigInt`
    /// module also carries the `of` conversion field, but this is its type role). `ground_type` maps it.
    BigIntTy,
    /// `BigInt.of` — the WIDENING conversion from a fixed-width integer to `BigInt`: `∀a. (Int a) →
    /// BigInt` (`options/numeric-model/explicit-checked.md` §Arbitrary-precision integer, "Construction").
    /// EXACT and never traps — every fixed-width value fits the unbounded type. A CONSTANT source FOLDS to
    /// the same `Core::ConstInt` (whose `IntValue` is already `num-bigint`-backed and unbounded), retyped
    /// `Ty::BigInt`: only the STATIC type changes, the value is unchanged — exactly as `Symbol.of` keeps
    /// its `Core::ConstStr`. A runtime source declines until the runtime limb ops (B3). The reverse
    /// (`Int64.of`/`(UInt N).of` from a `BigInt`, checked/trapping) is the existing `CheckedOf` extended to
    /// a `BigInt` source, not a new prim.
    ///
    /// `BigInt` is a DISTINCT numeric type (`Ty::BigInt`): a fixed-width integer never silently becomes a
    /// `BigInt` (nor the reverse) — the widening `BigInt.of` here and the checked narrowing `Int64.of` are
    /// the only crossings, each written explicitly (a `BigInt`/fixed-width mix is CDZ0301, no promotion).
    /// The narrowing direction cannot preserve exactness (a `BigInt` may not fit the fixed width), so it
    /// is exactly the WRITTEN, checked/trapping `Int64.of` — never an implicit conversion.
    //= spec/capabilities/numeric-model.md#an-arbitrary-precision-integer-is-a-distinct-type-opted-into-explicitly
    //# An arbitrary-precision integer MUST be a distinct numeric type, so that it does not silently convert to or from a fixed-width integer without an explicit conversion.
    //= spec/capabilities/numeric-model.md#exact-arithmetic-is-exact
    //# A conversion that cannot preserve a value's exactness MUST be written explicitly rather than performed implicitly.
    BigIntOf,
    /// The ground `Rational` type-value — held in the `Rational` module record's `(meta t)`, so bare
    /// `Rational` in type position reduces to `Ty::Rational` (a NULLARY type, like `BigInt`; the
    /// `Rational` module also carries the `of`/`of-int`/`value` operation fields, but this is its type
    /// role). `ground_type` maps it. B4-0 adds the type-position prim (byte-neutral, no ops yet).
    RationalTy,
    /// `Rational.of n d` — CONSTRUCT an exact rational from a numerator and denominator (`(Int a) → (Int
    /// b) → Rational`). Normalizes immediately: gcd-reduce to lowest terms, sign onto the numerator,
    /// denominator strictly positive. A ZERO denominator has no value → TRAPS ("rational with zero
    /// denominator"). A CONSTANT pair FOLDS to a normalized `Core::ConstRational`; a runtime operand emits
    /// `Core::RationalOfInts` (widen each to a BigInt + the runtime `rational-of` op, R3b). B4-1.
    ///
    /// The canonical normalized form (lowest terms + fixed sign placement) makes two rationals denoting
    /// the same number share one representation, and it is a deterministic function of the numerator and
    /// denominator; a zero denominator fails at this construction point rather than producing a value.
    //= spec/capabilities/numeric-model.md#an-exact-rational-has-a-canonical-normalized-form
    //# An exact rational value MUST be maintained in a canonical normalized form, so that two rationals denoting the same number have one representation and one canonical byte form.
    //= spec/capabilities/numeric-model.md#an-exact-rational-has-a-canonical-normalized-form
    //# The normalization of an exact rational MUST reduce it to lowest terms and fix the placement of its sign, so that its canonical form is a deterministic function of the number it denotes rather than of how it was constructed.
    //= spec/capabilities/numeric-model.md#a-rational-with-a-zero-denominator-is-not-a-value
    //# Constructing an exact rational with a zero denominator MUST fail at a defined point rather than produce a value, because a rational with a zero denominator denotes no number.
    RationalOf,
    /// `Rational.of-int n` — the WHOLE rational `n/1` from a fixed-width integer (`(Int a) → Rational`).
    /// The explicit integer→rational crossing (never an implicit promotion — a `Rational`/integer mix is
    /// CDZ0301). A constant folds to `Core::ConstRational(n, 1)`. B4-1.
    RationalOfInt,
    /// `Rational.value r` — a no-op identity that just NAMES `r`'s type `Rational` (the rational IS its own
    /// erased value; there is no narrower type to extract, unlike `Qty.value` erasing a unit). Present for
    /// surface symmetry; folds to its operand unchanged. B4-1.
    RationalValue,
    /// `Rational.numerator r` / `Rational.denominator r` — read the numerator / denominator of a Rational as
    /// a `BigInt` (`Rational → BigInt`). Result is `BigInt`, not `Int64`: a Rational is a numerator/
    /// denominator pair of BIG-INTEGERS (numeric-model.md), and either can exceed i64 — so an `Int64` surface
    /// would silently narrow/trap, violating rationals-everywhere exactness. Floor/round/integer-projection
    /// are written in Cadenza on top (BigInt divmod, then `Int64.of` the small final value). A CONSTANT
    /// Rational (`Core::ConstRational(n, d)`) folds to a constant BigInt of `n` / `d`; a runtime Rational
    /// emits `Core::RationalNum`/`RationalDen` (the runtime `rational-num`/`rational-den` ops, which BORROW
    /// the Rational and return a fresh BigInt handle). The denominator is always strictly positive + the pair
    /// is in lowest terms (the normalized canonical form), so num/den observe the reduced fraction.
    RationalNum,
    RationalDen,
    /// `Rational.truncate r` — the integer part of a rational TOWARD ZERO, narrowed to `Int64` (`7/2 → 3`,
    /// `-7/2 → -3`). NOT a runtime op: it lowers as a DERIVATION synthesizing `(Int64.of (/ (numerator r)
    /// (denominator r)))` over EXISTING nodes — `Rational.numerator`/`denominator` (→ BigInt), BigInt `/`
    /// (truncating, toward zero), then the checked `Int64.of` narrowing. A CONSTANT `Core::ConstRational(n,
    /// d)` folds to `Core::ConstInt(n / d)` (num-bigint truncating division) at compile time; a runtime
    /// Rational lowers the synthesized derivation subtree. The `Int64.of` narrowing TRAPS on overflow. The
    /// base of the Rational→Int64 projection surface; floor/ceil/round add a remainder-sign `±1` on top in
    /// later increments (numeric-model.md: integer projections of a rational are written on BigInt divmod).
    RationalTruncate,
    /// `Rational.floor r` (toward −∞) / `Rational.ceil r` (toward +∞) — the other two exact integer
    /// projections of a rational, narrowed to `Int64`. Like `RationalTruncate`, DERIVATIONS (no runtime op):
    /// `floor` = `truncate − 1` iff (numerator < 0 AND remainder ≠ 0) else `truncate`; `ceil` = `truncate +
    /// 1` iff (numerator > 0 AND remainder ≠ 0) else `truncate` — the ±1 adjustment off the truncating
    /// division's remainder sign (numeric-model.md: the integer projections are written on BigInt divmod).
    /// A CONSTANT `Core::ConstRational` folds to the adjusted `Core::ConstInt` (via `IntValue::divmod`); a
    /// runtime Rational synthesizes the derivation subtree (a remainder-sign `Core::If` over the truncate).
    /// `floor(-7/2) = -4` (vs `truncate`'s −3); `ceil(7/2) = 4`. The `Int64.of` narrowing TRAPS on overflow.
    RationalFloor,
    RationalCeil,
    /// `Rational.round r` — round to the NEAREST integer, ties HALF-AWAY-FROM-ZERO (`1/2 → 1`, `-1/2 → -1`,
    /// `3/2 → 2`, `5/2 → 3`), narrowed to `Int64`. Like the sibling projections, a DERIVATION (no runtime
    /// op): the toward-zero quotient `q` adjusted by the sign of `n` when `2·|remainder| ≥ denominator` (the
    /// fractional part is ≥ ½, so it rounds AWAY from zero). A CONSTANT `Core::ConstRational` folds to the
    /// adjusted `Core::ConstInt` (`IntValue::divmod` + the tie test); a runtime Rational synthesizes the
    /// derivation subtree. Half-away is the settled ruling (symmetric snapping for MIDI/tick boundaries).
    /// The `Int64.of` narrowing TRAPS on overflow. Completes the floor/ceil/truncate/round surface.
    RationalRound,
    /// `Symbol.of` — INTERN a String into a Symbol (`String → Symbol`, 17-symbols). A constant string
    /// FOLDS to a constant symbol (represented as the underlying `Core::ConstStr` at type `Ty::Symbol` —
    /// the identity is content-derived), so `(= (Symbol.of "a") (Symbol.of "a"))` folds via the shared
    /// constant-string equality. A runtime string interns at run time (a later increment).
    SymbolOf,
    /// `Symbol.to-string` — recover a Symbol's underlying content String (`Symbol → String`, the inverse
    /// of `Symbol.of`). A constant symbol FOLDS to its `Core::ConstStr` content (retyped `String`).
    SymbolToString,
    /// A SUM VARIANT CONSTRUCTOR — the `(meta apply)` of a variant field on a synthesized sum record
    /// (`crate::sums`). Applying it (`(Option.Some 5)`) builds the sum value `sum-new(disc, payload)`:
    /// the DISCRIMINANT is read off the variant record's `(meta variant)` channel at lowering (NOT
    /// baked into this prim — one `SumNew` serves every variant, like the one `Wrap` serves every target
    /// width, reading the target off the solved type). A NULLARY variant used bare is this prim applied
    /// to no arguments. The result is the owning sum type, read off the ctor's `(meta t)`.
    SumNew,
    /// A generic SUM TYPE CONSTRUCTOR — the `(meta apply)` of a GENERIC sum record (`crate::sums`).
    /// Applying it in TYPE position (`(Option Int64)`) builds the type-value `Ty::Sum { decl, args }`:
    /// the owning declaration is read off the record's `(meta sum-decl)` channel, the args are the
    /// applied type-values. One prim serves every generic sum (the decl is metadata, like `SumNew`'s
    /// discriminant), so `Option`/`Result`/… need no per-type prim — the same "type constructor's
    /// `(meta apply)` builds a type" model as `Int`/`Tuple`/`->`.
    SumCtor,
    /// A TUPLE VALUE CONSTRUCTOR — the `(meta apply)` of the prelude `tuple` alias. Applying it (`(tuple
    /// 1 2)` written with the shadowable name) builds the tuple value, exactly as the STRING-head
    /// primitive `("tuple" 1 2)` does: it lowers to `Core::Tuple{elems=args}` and types as `Ty::Tuple(elem
    /// types)`. VARIADIC over its elements — its type is the tuple of the argument types, so (unlike a
    /// sum variant's fixed arrow) it needs its own `apply_type` arm rather than a `(meta t)` scheme.
    TupleNew,
    /// A RECORD VALUE CONSTRUCTOR — the `(meta apply)` of the prelude `record` alias. Applying it
    /// (`(record (x 1) (y 2))`) builds the record value, exactly as the STRING-head primitive `("record"
    /// (x 1) (y 2))` does: each argument is a `(key value)` pair. VARIADIC over its fields; the record
    /// companion of `TupleNew`.
    RecordNew,
    /// The RECORD ROW-PROJECTION operation — `(Record.project r (a c))` narrows `r` to exactly the named
    /// fields, each bound to the value `r` holds for it (`type-system.md` §A Record Is Restricted To A
    /// Named Set Of Its Fields). Its SECOND operand is a LITERAL field-name LIST `(a c)` — labels, not an
    /// evaluated value (like a `record` literal's field names, read via [`crate::resolve::read_key`]) — so
    /// the projection's result shape is fixed statically. Folds over a constant `Core::Record` to a new
    /// `Core::Record` with only the named fields; a named field absent from the operand is CDZ0212. The
    /// narrowing member of the record row-operation surface (`without`/`merge`/`with`/`pop`/`extend` follow).
    RecordProject,
    /// The RECORD ROW-DROP operation — `(Record.without r (b))` derives `r` MINUS the named fields, i.e.
    /// the complement of `project` (`type-system.md` §A Record Is Reduced By Dropping A Named Set Of Its
    /// Fields). Same LITERAL field-name-list second operand as `project`; folds a constant `Core::Record`
    /// to a new one with the named fields removed. A named field absent from the operand is CDZ0212 (a
    /// drop of a field never held is a static error, not a silent no-op).
    RecordWithout,
    /// The RECORD ROW-MERGE operation — `(Record.merge a b)` combines two records into one whose field set
    /// is the UNION (`type-system.md` §Two Records Are Combined Only When Their Field Sets Are Disjoint).
    /// UNLIKE `project`/`without`, BOTH operands are ordinary record VALUES (no label list). The field sets
    /// MUST be DISJOINT: a shared field name is CDZ0211 (the combined record never chooses which operand's
    /// value a shared field takes). Folds two constant `Core::Record`s to their union.
    RecordMerge,
    /// The RECORD ROW-EXTEND operation — `(Record.extend r #z v)` adds a field ABSENT from `r`
    /// (`type-system.md` §A Field Is Added To Or Replaced In A Record By A Derived Operation, 1st
    /// sentence), the meaning-preserving rewrite of `(Record.merge r (record (z v)))`. Three operands
    /// (DESIGN-record-update-syntax.md): the record `r`, a `#z` field LABEL (static, read at resolve time),
    /// and the value `v` (evaluated, unlike `project`/`without`'s label list). An already-present field is
    /// CDZ0211 (never a silent overwrite — the author means `with`).
    RecordExtend,
    /// The RECORD ROW-UPDATE operation — `(Record.with r #z v)` REPLACES a field PRESENT in `r` with a
    /// new value of a possibly-different type (`type-system.md` §…2nd sentence), the rewrite of
    /// `(Record.merge (Record.without r (z)) (record (z v)))`. Same three operands as `extend` (record,
    /// `#z` LABEL, value). An absent field is CDZ0212 (stays distinct from `extend`, which ADDS).
    RecordWith,
    /// The RECORD ROW-POP operation — `(Record.pop r z)` takes a field OFF `r`, yielding `(tuple (. r z)
    /// (Record.without r (z)))` — the field's value paired with the record of the remaining fields
    /// (`type-system.md` §A Record Is Reduced By Dropping A Named Set Of Its Fields). Its second operand
    /// is a BARE field NAME (a label). An absent field is CDZ0212 — a record field name is a static label,
    /// never a runtime `None` (contrast `List.at` on a runtime index).
    RecordPop,
    /// The TUPLE positional CONCATENATE — `(Tuple.concat a b)` appends `b`'s elements after `a`'s, yielding
    /// a tuple of the combined arity, each element keeping its source position's type (`type-system.md`
    /// §Two Tuples Are Concatenated Into One Of Their Combined Length). Both operands are tuple VALUES (no
    /// disjointness — positions are anonymous). Folds two constant `Core::Tuple`s to their concatenation.
    TupleCat,
    /// The TUPLE positional SPLIT — `(Tuple.split-at t k)` splits `t` at compile-time literal position `k`
    /// into a PAIR `(tuple <prefix> <suffix>)` (`type-system.md` §A Tuple Is Split At A Position Into A
    /// Prefix And A Suffix): the prefix is a tuple of the first `k` elements, the suffix a tuple of the
    /// rest. `k=0` → the empty-tuple prefix IS `unit` (core-semantics.md §The Empty Tuple Is The Unit
    /// Value). `k` outside `0..=arity` is CDZ0201 (the static-bounds rule `(. x N)` uses). The second
    /// operand is a compile-time integer LITERAL (like a tuple index).
    TupleSplitAt,
    /// The TUPLE positional POP — `(Tuple.remove t)` takes element 0 off, yielding `(tuple (. t 0) <rest>)`
    /// — the positional analogue of `Record.pop`, `(Tuple.split-at t 1)` with the singleton prefix
    /// unwrapped to its element. A one-operand op over a tuple of arity ≥ 1.
    TuplePop,
    /// `Tuple.size t` — the tuple's ARITY as an `Int64`. A tuple's arity is a STATIC property of its
    /// type (a tuple carries no runtime length — `type-system.md` §The Arity Of A Tuple Positional
    /// Operation's Result Must Be Determined Statically), so this ALWAYS folds to a constant `Int` in
    /// `lower` from the operand's `Ty::Tuple` — even for a runtime-valued tuple. It never emits a
    /// runtime op (there is no runtime tuple-length). One operand: the tuple.
    TupleSize,
    /// A LIST VALUE CONSTRUCTOR — the `(meta apply)` of the prelude `list` alias. Applying it (`(list 1 2
    /// 3)`) builds the list value, exactly as the STRING-head primitive `("list" 1 2 3)` does. VARIADIC,
    /// but HOMOGENEOUS: every element unifies to ONE element type (a mixed list is ill-typed), so its
    /// type is `Ty::List(elem)` not a per-position product. Lowers to `Core::ListNew{elems}` (built on
    /// the persistent `vec-*` heap). The tuple/record companion for the homogeneous sequence.
    ListNew,
    /// A SET VALUE CONSTRUCTOR — the `(meta apply)` of the prelude `set` NAME alias (operator ruling
    /// 2026-08-31: keep a SHADOWABLE name constructor for set as the string-form `("set" …)` is dropped at
    /// the M3 reader-flip). Applying it (`(set 1 2 3)`) reduces to the native `#set(1 2 3)` form, resolving
    /// to `Resolved::Set` → `Core::SetOf` (constant elements dedup at build) — IDENTICAL to the native
    /// literal, so a shadowable `(set …)` and the unshadowable `#set(…)` denote the same set. VARIADIC,
    /// homogeneous. The set companion of `ListNew`/`TupleNew`/`RecordNew`/`MapNew` (set uniquely lacked a
    /// name alias until now — it had only the native leaf + the dropped string form).
    SetNew,
    /// `List.len` — the length of a list, an `Int64`. The `(meta apply)` of the `len` field of the `List`
    /// prelude module. Lowers to the runtime `vec-len` op.
    ListLen,
    /// `List.push` — append an element to a list, returning the new list. `∀a. (List a) → a → (List a)`.
    /// Lowers to the runtime `vec-push` op (persistent — returns a new handle, does not mutate).
    ListPush,
    /// `List.prepend` — insert an element at the FRONT of a list, returning the new list. Receiver-first,
    /// like `push`/`at`/`update`: `∀a. (List a) → a → (List a)` (operator ruling — consistent arg order,
    /// pipeline-friendly). Phase-1 is a DERIVED lowering — `concat(list-new(elem), list)` (a singleton list
    /// prepended, reusing the persistent `vec-concat`), so NO dedicated runtime op and NO frozen-hash bump.
    /// The front-insertion companion of `List.push` (which appends at the tail).
    ListPrepend,
    /// `List.concat` — concatenate two lists of the same element type into one. `∀a. (List a) → (List a)
    /// → (List a)`. Lowers to the runtime `vec-concat` op.
    ListConcat,
    /// `List.update` — replace the element at an index, returning the new list. `∀a. (List a) → Int64 →
    /// a → (List a)`. Lowers to the runtime `vec-update` op (persistent — returns a new handle; an
    /// out-of-bounds index TRAPS). The functional-construction companion of `List.push`.
    ListUpdate,
    /// `List.at` — the FALLIBLE indexed read. `∀a. (List a) → Int64 → (Option a)`: `Some` of the element
    /// when the index is in bounds (`0 <= i < len`), `None` otherwise (collections-and-text.md #Indexing
    /// And Lookup Are Fallible, Not Trapping — an out-of-range index yields `None`, never traps nor reads
    /// an unspecified value). A CONSTANT list + constant index FOLDS to `(Some elem)` / `(None unit)`;
    /// a runtime list emits a bounds-checked `vec-get` (runtime companion of `arr-get` for the flat
    /// product). The list-reader half of the fallible-access family (`Bytes.at`/`String.at` mirror it).
    ListAt,
    /// `List : Type → Type` — the list-TYPE constructor. `(List Int64)` in type position builds the
    /// type-value `Ty::List(Int64)` (used in annotations `(: e (List Int64))` and in `List.len`'s scheme
    /// `∀a. (List a) → Int64`). One element type, unlike `Tuple`'s variadic — the list companion of the
    /// type constructors `Int`/`Tuple`/`Record`.
    ListCtor,
    /// `ast-splice-lift` — a COMPILER-INTERNAL operation (`(List Int64) → (List Ast)`) emitted only by the
    /// quasiquote splice desugar (`quote::reify_active`): it LIFTS each element of a list into an `Ast.Int`
    /// node, so an active `,@args` splice's elements enter the parent `Ast.List` already wrapped. Applied
    /// via `(intrinsic "ast-splice-lift")` (never a user-spellable surface). CONSTANT-fold only this
    /// increment — a constant `Core::ListNew` of Int64 folds to a `ListNew` of `(Ast.Int e)` `Core::SumNew`
    /// nodes; a runtime list operand declines (the runtime map is a later increment). Int-only lift, the
    /// splice companion of the active-unquote `(Ast.Int e)` wrap.
    AstSpliceLift,
    /// `ast-lift` — a COMPILER-INTERNAL operation (`∀a. a → Ast`) emitted only by the quasiquote
    /// desugar (`quote::reify_active`) around a RUNTIME active-unquote operand (a name `,n` / a computed
    /// `,(f x)`) whose value type is unknown at reify time (reify runs pre-typecheck). It LIFTS the
    /// operand's value into the `Ast` leaf its INFERRED type denotes — resolved at LOWER: IDENTITY when
    /// the operand is already `Ast` (the compiler's own `(quasiquote (+ (unquote sub) 1))` with `sub :
    /// Ast`), else wrap in `Ast.Int`/`Ast.Bool`/`Ast.Str` by the operand's scalar type. Applied via
    /// `(intrinsic "ast-lift")` (never user-spellable). This replaces the old unconditional `(Ast.Int e)`
    /// wrap of a runtime operand, which type-errored a non-Int operand against `Ast.Int`'s Int64 payload
    /// (`[[unquote-computed-ast-needs-inferred-type-lift]]`). A LITERAL operand still dispatches by kind
    /// in reify (no `ast-lift` needed — its kind is known structurally); only a runtime operand takes this.
    AstLift,
    /// `Ast.encode` — serialize an AST value to its canonical binary form: `Ast → Bytes` (the `(meta
    /// apply)` of the `encode` field of the `Ast` sum record). Realizes `ast-encoding.md` §The Encoding
    /// Is A Bijection With One Canonical Byte Form: each tree has EXACTLY ONE encoding, equal trees encode
    /// identically. Constant-fold ONLY this increment — a compile-time-visible `Ast` value (a `Core::SumNew`
    /// tree of the `Int`/`Name`/`List` variants) folds to a `Core::BytesOf` of the canonical bytes; a
    /// runtime `Ast` declines. The canonical byte format is a DECLARED-DEFAULT choice (the contract pins
    /// the bijection, not the concrete bytes) — see `lower::encode_ast_value` for the tag layout.
    AstEncode,
    /// `Ast.decode` — the TOTAL inverse of `Ast.encode`: `Bytes → (Result Ast e)` (the `(meta apply)` of
    /// the `decode` field of the `Ast` sum record). Total over POSSIBLY-EXTERNAL bytes (`value-interchange.md`
    /// §A Decode Over External Bytes Is Total): a byte sequence that is the canonical encoding of an AST →
    /// `(Ok ast)`; anything else (ill-formed, truncated, or valid-prefix-plus-trailing) → `(Err …)`, NEVER
    /// a trap. Constant-fold ONLY this increment — a compile-time-visible `Core::BytesOf` parses to `(Ok
    /// <SumNew tree>)` / `(Err unit)`; a runtime `Bytes` declines (the runtime deserializer is a later
    /// increment). The decode companion of `AstEncode`; the format is `lower::decode_ast_value`.
    AstDecode,
    /// `Blake3.of` — the blake3 content hash: `Bytes → Bytes` (a 32-byte unkeyed BLAKE3-256 digest as a
    /// `Bytes` value; the `(meta apply)` of the `Blake3` module's `of` field). NAMES THE ALGORITHM
    /// (design-compiler-primitives.md D5): a future digest is a DIFFERENT named function, never a silent
    /// change to a generic `Hash`. ENTIRELY GENERIC — raw `bytes → digest`, no tag/prefix/domain
    /// separation (that is userspace's job, D7). Constant-fold this increment: a compile-time-visible
    /// `Bytes` (a `Core::ConstBytes` / a `Core::BytesOf` of constants) folds to the `Core::ConstBytes` of
    /// its `blake3::hash`; a runtime `Bytes` declines until the runtime lowering to heap op 91
    /// (`hash-blake3`) lands. Byte-identical to that runtime op — both call the one `blake3` crate over the
    /// same bytes (design §9). The compile-time half of the third contract-agnostic primitive.
    Blake3Of,
    /// `Ast.module` — the enclosing module's own source as an `Ast` value: the `(meta apply)` of the `module`
    /// field on the built-in `Ast` record, a bare VALUE `Ast.module : Ast` (magic-constant style — never
    /// applied; the `() -> Ast` is its internal shape). The
    /// TYPE-DIRECTED, prelude-derived self-reflection (operator redesign of the retired `(. Ast self)`
    /// syntax-rewrite): it resolves through ordinary member access on the built-in `Ast` (so a user
    /// `type Ast` shadows it — the reflection field lives ONLY on the built-in Ast record) and types as the
    /// built-in `Ast` sum. ALWAYS folds at compile time (no runtime "reflect the current module") — the
    /// lowerer fills it from the enclosing module's canonical SOURCE via `file_of(occ)` + a pre-resolve
    /// per-file source snapshot (v-compiler-primitives' fill), so it never reaches a backend as a distinct
    /// op. Byte-identical to `quote`/`__ast__` over the same module.
    ReflectModule,
    /// `print` — render an `Ast` value as its canonical re-readable TEXT: `Ast → String` (self-hosting-
    /// surface.md §A Printer Renders The Canonical Representation As Re-Readable Text). The text analogue of
    /// `AstEncode` (which produces canonical BYTES): a compile-time-visible `Ast` value (a `Core::SumNew`
    /// tree of the `Int`/`Name`/`List` variants) folds to the `Core::ConstStr` of the s-expression the tree
    /// denotes (`(Ast.List (list (Ast.Name "+") (Ast.Int 1) (Ast.Int 2)))` → `"(+ 1 2)"`); a runtime `Ast`
    /// declines. Paired with `Read` so `read(print(v)) == v` under structural equality.
    //= spec/capabilities/self-hosting-surface.md#a-printer-renders-the-canonical-representation-as-re-readable-text
    //# A printer MUST render a program's canonical representation as text that a reader converts back to the same canonical representation.
    Print,
    /// `read` — the inverse of `print`: parse canonical TEXT back to the `Ast` value it denotes: `String →
    /// Ast` (self-hosting-surface.md §A Reader Converts Text To The Canonical Representation). The text
    /// analogue of `AstDecode`: a compile-time-visible `Core::ConstStr` is parsed as one s-expression and
    /// reified into the `Ast` `Core::SumNew` tree (`"(+ 1 2)"` → `(Ast.List …)`); a runtime `String`
    /// declines. `read(print(v))` reproduces `v` exactly (the round-trip the corpus witnesses).
    //= spec/capabilities/self-hosting-surface.md#a-reader-converts-text-to-the-canonical-representation
    //# A reader MUST convert the text of a program to the program's canonical representation, so that a program can be written as text before a surface syntax exists.
    Read,
    /// `Ast.gensym` — mint a FRESH, collision-free `Ast.Name` for MANUAL macro hygiene (macros are
    /// non-hygienic by default; DESIGN-macro-system.md): `String → Ast`. A compile-time-visible base
    /// string folds to `Ast.Name "<base> $<node-id>"` — the call node's id makes the name distinct per
    /// call site and DETERMINISTIC across compiles (reproducible expansion), and the embedded SPACE makes
    /// it unreadable (a reader can never produce a name with a space) so it cannot collide with a source
    /// name or another gensym. A runtime (non-constant) base declines. Folds at compile time — no backend
    /// arm (like `Print`/`Read`/`ReflectModule`).
    Gensym,
    /// `Bytes.of` — construct a byte sequence from a list of integers in `0..=255`: `(List Int64) →
    /// Bytes`. The `(meta apply)` of the `of` field of the `Bytes` module. A CONSTANT list literal folds
    /// to the baked byte value (range-checking each element: `< 0` or `> 255` is a compile-time trap,
    /// CDZ0304); a runtime list emits the `bytes-alloc`+`bytes-set` build. The one bytes CONSTRUCTOR.
    BytesOf,
    /// `Bytes.len` — the length of a byte sequence, an `Int64` (`Bytes → Int64`). The `(meta apply)` of
    /// the `len` field of the `Bytes` module. A compile-time-visible `Bytes.of` literal folds to its byte
    /// count; a runtime bytes emits `bytes-len` (+ i32→i64 extend). The bytes companion of `List.len`.
    BytesLen,
    /// The ground type-value `Bytes` — the `(meta t)` of the `Bytes` module, so bare `Bytes` in type
    /// position IS the type `Ty::Bytes` (the leaf companion of `BoolTy`/`UnitTy`; `ground_type` maps it).
    BytesTy,
    /// `String.scalar-len` — the number of Unicode SCALAR VALUES in a string, an `Int64`
    /// (`collections-and-text.md` §A String Offers Both A Scalar Length And A Byte Length). `String →
    /// Int64`. On a CONSTANT string it FOLDS to the char count (`"café"` → 4, distinct from its 5 bytes);
    /// a runtime string declines (the byte-rope length op arrives later).
    StrScalarLen,
    /// `String.byte-len` — the number of BYTES in a string's UTF-8 encoding, an `Int64` (`String →
    /// Int64`). On a CONSTANT string it FOLDS to the UTF-8 byte count (`"café"` → 5); the byte companion
    /// of `scalar-len`, differing exactly on a multi-byte string.
    StrByteLen,
    /// `Bytes.at` — the FALLIBLE indexed byte read `Bytes → Int64 → (Option Int64)`. The `(meta apply)`
    /// of the `at` field of the `Bytes` module. A constant `Bytes.of` indexed by a constant folds to
    /// `(Some byte)` / `(None unit)`; a runtime read emits `Core::BytesAt` (a bounds-checked `bytes-get`
    /// boxed into `Some`, else `None`). The byte companion of `List.at`; monomorphic (a byte is Int64).
    BytesAt,
    /// `Bytes.concat` — append two byte sequences `Bytes → Bytes → Bytes`. A constant pair folds to a
    /// single `Core::BytesOf`; a runtime pair emits `Core::BytesConcat` (`bytes-concat`). Byte companion
    /// of `List.concat`.
    BytesConcat,
    /// `Bytes.slice` — the FALLIBLE sub-range read `Bytes → Int64 → Int64 → (Option Bytes)`. In range
    /// (`start >= 0`, `len >= 0`, `start + len <= bytes-len`) → `Some(bytes-slice)`, else `None` (the emit
    /// bounds-checks first — the runtime `bytes-slice` would TRAP on OOB). Folds a constant.
    BytesSlice,
    /// `Bytes.compact` — `Bytes → Bytes`, a content-equal sequence with independent storage (rope
    /// collapse). Total; a constant folds to itself, a runtime value emits `bytes-compact`.
    BytesCompact,
    /// `String.at` — the FALLIBLE scalar-indexed read. `String → Int64 → (Option String)`: `Some` of the
    /// ONE-scalar string at that Unicode SCALAR position when in bounds (`0 <= i < scalar-len`), `None`
    /// otherwise (collections-and-text.md #Indexing And Lookup Are Fallible, Not Trapping + #A String Is
    /// A Sequence Of Unicode Scalar Values — indexed by SCALAR, not byte). A CONSTANT string + constant
    /// index FOLDS to `(Some "<char>")` / `(None unit)` (`chars().nth(i)`); a runtime string declines
    /// (the byte-rope indexed read arrives later). The string companion of `List.at`.
    StrAt,
    /// `String.scalar-at` — the fallible read of the CHAR at a scalar position (`String → Int64 →
    /// (Option Char)`, the char-typed companion of `StrAt` which yields a one-scalar `Option String`).
    /// A CONSTANT string + constant index FOLDS to `(Some #\c)` (the scalar via `chars().nth(i)`) / `(None
    /// unit)` out of bounds; a runtime string declines. Addresses SCALAR values, not bytes.
    StrScalarAt,
    /// `String.concat` — the TOTAL binary join `String → String → String` (the `(meta apply)` of the
    /// `concat` field of the `String` module). On two CONSTANT strings it FOLDS to their concatenation
    /// (`(String.concat "hello" " world")` → `"hello world"`); a runtime operand declines (the byte-rope
    /// join arrives with the runtime string heap). The compiler builds error messages and export names
    /// this way (collections-and-text.md #Strings Concatenate).
    StrConcat,
    /// `String.slice` — the FALLIBLE sub-range read `String → Int64 → Int64 → (Option String)` by SCALAR
    /// offsets (`start`, `end`, half-open `[start, end)`). In range (`0 <= start <= end <= scalar-len`) →
    /// `Some substring`, else `None` (a reversed, over-long, or negative bound). A CONSTANT string +
    /// constant bounds FOLD to `(Some "<substr>")` / `(None unit)` (indexed by Unicode scalar, NOT byte);
    /// a runtime string declines (the byte-rope slice arrives later). The string companion of
    /// `Bytes.slice`, but cut by scalar offset and with an `(start, end)` — not `(start, len)` — range.
    StrSlice,
    /// `String.to-bytes` — the UTF-8 encoding `String → Bytes`. A CONSTANT string FOLDS to a constant
    /// `Core::BytesOf` of its UTF-8 bytes (each a synthesized `UInt8`), consumed by `Bytes.len`/`Bytes.at`
    /// (`(Bytes.len (String.to-bytes "run"))` → 3, `"café"` → 5); a runtime string declines (the byte-rope
    /// materialization arrives with the runtime string heap). A String IS its UTF-8 bytes.
    StrToBytes,
    /// `String.from-bytes` — the TOTAL UTF-8 DECODE `Bytes → (Option String)` (the inverse of `to-bytes`):
    /// a well-formed byte sequence → `Some string`, ill-formed → `None`, never a trap (collections-and-
    /// text.md #Decoding Bytes To A String Is Total, Not Trapping). A CONSTANT `Bytes.of` FOLDS via strict
    /// UTF-8 (`std::str::from_utf8`, which rejects invalid/overlong/surrogate exactly as the spec pins) →
    /// `(Some "<decoded>")` / `(None unit)` at the result Option's discs (like `List.at`/`String.at`); a
    /// runtime Bytes declines.
    StrFromBytes,
    /// `Option.expect` / `Result.expect` — the unwrap-or-trap accessor `∀a. Sum<a> → String → a`. The
    /// `(meta apply)` of the `expect` field on the synthesized Option/Result records. Applying it to a
    /// present variant (`Some`/`Ok`, discriminant 0) yields the payload; the absent variant TRAPS
    /// (core-semantics.md §Requiring The Value Of An Optional Traps On Absence). A constant present
    /// variant FOLDS to its payload; a runtime sum emits `Core::SumExpect` (disc probe → payload / trap).
    /// The message argument is a `String` (for a human), dropped by the pure core (the wasm trap is
    /// textless). ONE prim for both Option and Result — present is discriminant 0 in each.
    ///
    /// This IS the value-requiring operation a program uses to obtain the present value of a fallible
    /// read's optional (a `List.at`/`Map.lookup` result): it carries a mandatory message argument, so the
    /// boundary between handling absence as data (matching the `Option`) and halting on absence (this
    /// `expect`) is explicit at the point the program crosses it, not hidden inside the access operation.
    //= spec/capabilities/collections-and-text.md#indexing-and-lookup-are-fallible-not-trapping
    //# A program that requires the present value of such an optional MUST obtain it through the optional's value-requiring operation carrying a mandatory message (core-semantics.md §"Requiring The Value Of An Optional Traps On Absence"), so that the boundary between handling absence as data and halting on absence is explicit at the point the program crosses it, not hidden inside the access operation.
    ///
    /// The message is a MANDATORY parameter of the operation, not optional: `expect`'s scheme is `∀a.
    /// Sum<a> → String → a` and `lower` requires the full two-argument application (`args.len() == 2`), so
    /// a program cannot require a present value without stating, at that point, why it expects presence.
    /// (The runtime does not carry the text — that is a SEPARATE, currently-declined obligation; here only
    /// the "MUST require a message argument" signature obligation is realized.)
    //= spec/capabilities/core-semantics.md#requiring-the-value-of-an-optional-traps-on-absence
    //# This operation MUST require a message argument, so that requiring a present value states, at the point it does so, why the value is expected to be present.
    SumExpect,
    /// `trap` — the DIVERGING primitive: `∀a. String → a`, an expression that never produces a value but
    /// HALTS the program at a defined point (core-semantics.md §A Trap Occurs Only Where Its Computation
    /// Is Observed; type-system.md §Never Is The Empty Sum — a diverging expression has type `Never`,
    /// which unifies with any expected type). Realized as a bare-name prelude intrinsic whose scheme's
    /// RESULT is a fresh unquantified type variable `a` — so ordinary Hindley-Milner makes `(trap "x")`
    /// fit ANY position (`(if b 1 (trap "x"))`, `(+ 1 (bomb))`), no dedicated `Never` type needed. Lowers
    /// to `Core::Trap`, which emits `unreachable` (wasm) / `unreachable!()` (rust); the `String` message
    /// argument is DROPPED (the wasm trap carries no text, exactly as `SumExpect`'s absent branch does).
    Trap,
    /// `Int64.checked-add` / `checked-mul` — the FALLIBLE arithmetic companions of the trapping `+`/`*`:
    /// `T → T → (Option T)`, the exact result wrapped in `Some` when it fits the width, `None` on overflow
    /// (numeric-model.md §Overflow Is Defined — the defined value outcome alongside the trap). The `(meta
    /// apply)` of the `checked-add`/`checked-mul` field of an integer module. A CONSTANT operand pair
    /// FOLDS (`i64::checked_add`/`checked_mul` → `Some result` / `None`); a runtime operand emits the
    /// overflow-detecting `Some`/`None` build (a later increment). The target width is the module's own,
    /// read off the solved type.
    CheckedAdd,
    CheckedSub,
    CheckedMul,
    /// `Int64.wrapping-add` / `wrapping-mul` — two's-complement wraparound modulo 2^width: `T → T → T`,
    /// NEVER trapping (numeric-model.md §Overflow Is Defined — the modular value outcome). The `(meta
    /// apply)` of the `wrapping-add`/`wrapping-mul` field of an integer module. A CONSTANT operand pair
    /// FOLDS (`i64::wrapping_add`/`wrapping_mul`); a runtime operand emits the RAW machine `i64.add`/
    /// `i64.mul` (wasm's add/mul already wrap — no overflow guard, unlike the trapping `+`/`*`). The
    /// target width is the module's own, read off the solved type (a narrow width masks after the op).
    WrappingAdd,
    WrappingSub,
    WrappingMul,
    /// A MAP TYPE CONSTRUCTOR — the `(meta apply)` of the `Map` prelude module. Applying it in TYPE
    /// position (`(Map Int64 Int64)`) builds the type-value `Ty::Map(key, value)` (used in annotations
    /// and in the map ops' schemes `∀k v. (Map k v) → …`). TWO type parameters (key then value), unlike
    /// `List`'s single — the two-parameter companion of `List`/`Tuple`/`Record` type constructors.
    MapCtor,
    /// A MAP VALUE CONSTRUCTOR — the `(meta apply)` of the `map` prelude alias, and the lowering target of
    /// the `(map (k v) …)` literal. Builds a map value from `(key, value)` entry pairs, each key an
    /// ORDINARY VALUE expression resolved in scope (NOT a compile-time label like a record field): `(let
    /// ((a 5)) (map (a 1)))` keys by the VALUE 5. HOMOGENEOUS on two axes — all keys unify to one type,
    /// all values to one — so its type is `Ty::Map(k, v)`. Lowers to `map-empty` + a consuming
    /// `map-insert` per entry (or const-folds a constant map). Keys are compared by value, so a
    /// runtime-computed duplicate key overwrites; a compile-time-constant duplicate is a CDZ0201 reject.
    MapNew,
    /// `Map.empty` — the empty map VALUE `∀k v. (Map k v)`. The `(meta apply)` of the `empty` field of
    /// the `Map` module. A value, not a function (like a nullary sum ctor's bare use); lowers to the
    /// runtime `map-empty` op (or folds to an empty `Core::MapNew`).
    MapEmpty,
    /// `Map.insert` — add-or-replace an association, returning the NEW map (functional construction):
    /// `∀k v. (Map k v) → k → v → (Map k v)`. Lowers to the runtime `map-insert` op (persistent —
    /// consumes the map handle, returns a new one). Enforces key/value homogeneity against the map's
    /// solved key/value types (CDZ0201 on a mismatch). Inserting a present key replaces its value.
    MapInsert,
    /// `Map.merge` — the UNION of two maps, LAST-WRITER (right operand) wins on an overlapping key: `∀k v.
    /// (Map k v) → (Map k v) → (Map k v)`. Lowers to the runtime `map-merge` op (CHAMP union — consumes
    /// both map handles, returns a new one). The map analogue of `List.concat`/`Record.merge`; backs the
    /// value-position map construction spread.
    MapMerge,
    /// `Map.lookup` — the FALLIBLE keyed read `∀k v. (Map k v) → k → (Option v)`: `Some v` when the map
    /// contains the key, `None` otherwise (collections-and-text.md §Indexing And Lookup Are Fallible —
    /// the map clause). Lowers to the runtime `map-lookup` op (returns a NULL handle when absent, which
    /// the backend wraps as the Option, exactly as `List.at` wraps a bounds-checked read). Keys compared
    /// by value.
    MapLookup,
    /// `Map.remove` — drop a key's association, returning the NEW map: `∀k v. (Map k v) → k → (Map k v)`.
    /// Lowers to the runtime `map-remove` op (persistent — consumes the map handle). Removing an absent
    /// key yields a map equal to the operand (removal is total, no trap).
    MapRemove,
    /// `Map.size` — the number of DISTINCT keys the map associates, an `Int64` (`∀k v. (Map k v) →
    /// Int64`). Lowers to the runtime `map-size` op (O(1) from the CHAMP root) + an i32→i64 extend. The
    /// map companion of `List.len`.
    MapSize,
    /// `Map.swap` — the value-yielding add: `∀k v. (Map k v) → k → v → (Tuple (Option v) (Map k v))`,
    /// pairing the prior value (present when the key was associated) with the new map. Lowers as a
    /// borrow-`map-lookup` (→ Option) paired with a consuming `map-insert` (collections-and-text.md §A Map
    /// Is Built By Functional Construction — the two-form rule).
    MapSwap,
    /// `Map.take` — the value-yielding remove: `∀k v. (Map k v) → k → (Tuple (Option v) (Map k v))`,
    /// pairing the removed value (present when the key was associated) with the new map. Lowers as a
    /// borrow-`map-lookup` paired with a consuming `map-remove`. The remove companion of `Map.swap`.
    MapTake,

    // ---- Units of measure (the optional, compile-time-only dimensional-analysis layer) ----
    // A UNIT is a compile-time value (an element of the free abelian group over base dimensions); these
    // prims BUILD one and are reduced away by `eval` before emission. A unit indexes `Ty::Qty` and
    // never reaches the backend (`units-of-measure.md` §Dimensions Are Checked Then Erased).
    /// `Unit.one` — the dimensionless unit, the group identity (the empty exponent map). Reduces to a
    /// canonical `(unit)` node. Applying it is a no-op (it takes no arguments).
    UnitOne,
    /// `Unit.base` — a base dimension named by a symbol: `(Unit.base #"meter")` reduces to the unit
    /// `{meter: 1}`. The one-unit-per-dimension case (Layer 1); the symbol's TEXT is read directly off
    /// its `Leaf::Sym` (resolved to a `Str`).
    UnitBase,
    /// `Unit.*` — the product of two units (pointwise exponent add, dropping zeros): `(Unit.* meter
    /// meter)` = `{meter: 2}`. The `*` dimensional rule's builder.
    UnitMul,
    /// `Unit./` — the quotient of two units (pointwise exponent subtract, dropping zeros): `(Unit./
    /// meter second)` = `{meter: 1, second: -1}` (a velocity). The `/` dimensional rule's builder.
    UnitDiv,
    /// `Unit.^` — a unit raised to a compile-time integer power (each exponent scaled, dropping zeros):
    /// `(Unit.^ meter 2)` = `{meter: 2}` (area). May be negative (`(Unit.^ second -1)` = frequency).
    UnitPow,
    /// `Unit.prefix` — SCALE a unit by a prefix's exact factor, producing another unit of the SAME
    /// dimension differing only by that factor (`units-of-measure.md` §A Scaled Unit Is A Unit Scaled By
    /// An Exact Factor). `(Unit.prefix kilo meter)` = meter at scale 1000; `(Unit.prefix mebi byte)` =
    /// byte at scale 2²⁰. The prefix argument (`kilo`/`milli`/`mebi`/…) is a prelude record carrying its
    /// scale ratio on a `(meta scale)` channel (`(num den)`), which this reads and applies via
    /// `Unit::scaled`. The scale is compile-time metadata (a machine-int ratio), NOT a runtime Rational.
    UnitPrefix,
    /// `Unit.of` — name a FAMILY unit: `(Unit.of #"foot")` is a unit of the `length` dimension at foot's
    /// exact scale to meter (381/1250). Consults a prelude FAMILY REGISTRY (a record mapping each unit
    /// name to its reference-dimension symbol + scale `(num den)`), so the vocabulary is prelude DATA,
    /// not a privileged in-compiler list (`units-of-measure.md` #A Dimension Groups Interconvertible
    /// Units). Builds `Unit.base(dim).scaled(num, den)`. Scales are machine-int metadata (foot 381/1250,
    /// mile 201168/125 - all small), so a family unit auto-converts over Float/Int with NO bignum.
    UnitOf,
    /// `Unit.define` — DECLARE a family unit: `(Unit.define #"furlong" (Unit.of #"foot") 660 1)` names
    /// `furlong` as 660 feet. As a VALUE it reduces to the defined unit itself (`base` scaled by
    /// `num/den`), so it can be bound or used inline; its DECLARATION effect (registering the name so
    /// `(Unit.of #"furlong")` resolves) is captured by a load-time scan (`db::scan_unit_defines`). A
    /// redeclaration conflicting with the built-in table or another declaration is CDZ0502
    /// (`units-of-measure.md` §A Named Unit's Conversion Is Unique). The user family-declaration surface.
    UnitDefine,
    /// `Unit.in` — EXPLICIT conversion of a quantity to a chosen unit: `(Unit.in meter (Qty.of 3.0 km))`
    /// = `(Qty Float64 meter)` with value 3000 (`units-of-measure.md` #A Unit Conversion Is The
    /// Arithmetic The Source Denotes; the way a program pins a specific result unit rather than the
    /// auto-chosen reference). Takes a TARGET unit and a quantity of the SAME dimension (else CDZ0501);
    /// the magnitude is multiplied by `source.scale / target.scale` in the inner type T (Float rounds,
    /// Int exact/truncates). The result unit is the TARGET.
    UnitIn,
    /// `Qty.of` - attach a unit to a numeric value: `∀(T,u). T → u → (Qty T u)`. The result's inner type
    /// is the value argument's type; the result's UNIT is the VALUE of the second argument (a
    /// compile-time unit read by `unit_of`). Erases to the value argument's lowering (the unit is
    /// compile-time-only). The one quantity CONSTRUCTOR.
    QtyOf,
    /// `Qty.value` — recover the underlying numeric value, DISCARDING the unit: `∀(T,u). (Qty T u) → T`.
    /// The explicit exit from the dimensional layer (the widening that requires no check). Erases to its
    /// argument's lowering (the quantity's inner value IS its erased value).
    QtyValue,
    /// `Qty.pow q n` — raise a quantity to a compile-time NON-NEGATIVE integer power, composing the unit
    /// the same way `Unit.^` does: `(Qty.pow (Qty.of 3.0 meter) 2)` = `9.0 : (Qty Float64 meter²)`. The
    /// unit's exponents (and scale) are raised to the `n`th power (`Unit::pow`); the numeric magnitude
    /// erases to `value * value * … ` (`n` factors) — `n = 0` is the dimensionless `1`. The exponent `n`
    /// is a compile-time `Int` literal read off arg1 (not an HM variable), exactly as `Unit.^` reads its
    /// power and `Qty.of` reads its unit. A negative exponent DECLINES for now (needs a reciprocal).
    QtyPow,
    /// `Qty.unit q` — extract a quantity's UNIT as a compile-time unit value (a `ty::Unit`): `(Qty.of new
    /// (Qty.unit y))` builds a new quantity in `y`'s unit WITHOUT re-spelling it. Reduces (via `unit_of`,
    /// which reads `q`'s solved `Ty::Qty` for its unit) exactly like `(Unit.base …)` does — it IS a unit
    /// expression, flowing through the unit-reading path (`unit_of`/`typeval_of`), never a runtime value.
    /// Compile-time-only (units erase), so `Qty.unit` is usable only in a UNIT position, like any unit
    /// value. The value-level companion of `Type.of`: `Type.of` gives the whole type (for annotations);
    /// `Qty.unit` gives just the unit (to construct another quantity of the same unit).
    QtyUnit,
    /// `Qty : (Type, Unit) → Type` — the QUANTITY-TYPE constructor. `(Qty Float64 u)` in TYPE position
    /// builds the type-value `Ty::Qty { inner, unit }` (used in an annotation `(: e (Qty T u))`), reading
    /// the first argument as the inner numeric type and the second as a compile-time unit (`unit_of`).
    /// The `(meta apply)` of the `Qty` module — the quantity companion of `Int`/`List`/`Tuple`'s type
    /// constructors, so a `(Qty …)` annotation reduces through the ordinary `typeval_of` path.
    QtyCtor,
    /// `Type.of e` — COMPILE-TIME TYPE REFLECTION: reduce to the type-VALUE of `e`'s inferred type, so
    /// `(: x (Type.of y))` gives `x` the same type as `y`. The `(meta apply)` of the `Type` module.
    /// Reduces (via `reduce_ctor`/`typeval_of`) to `encode_typeval(type_of(e))` — the value→type
    /// direction that composes the two existing halves (`type_of` computes the `Ty`; `encode_typeval`
    /// makes it a first-class type-value). Its own type is `Ty::Type`. Compile-time-ONLY: a `Type` value
    /// is erased before the runtime boundary (like any type-value / a `Ty::Qty`), so `Type.of` is usable
    /// in TYPE positions (annotations, further type-level computation), never returned at runtime.
    TypeOf,
    /// `Type.eq a b` — COMPILE-TIME TYPE EQUALITY: reduce both arguments to their `Ty` (each a type-value —
    /// a `(Type.of e)` result OR a written type like `Int64`/`(Qty Float64 meter)`, via `typeval_of`) and
    /// fold to the constant `Bool` of their EXACT STRUCTURAL equality (`Ty`'s `PartialEq`: `meter` ≠
    /// `second`, `Int64` ≠ `Int32`, `(Qty T u)` compares inner AND unit). Because it folds to a constant,
    /// `(if (Type.eq (Type.of x) Int64) …)` selects a branch AT COMPILE TIME — reflection that lets a
    /// program branch on types. The `(meta apply)` of the `Type` module's `eq` field. Its result `Bool` is
    /// an ordinary runtime value (unlike a `Type` value, which is erased); the type COMPARISON is what is
    /// compile-time.
    TypeEq,
    /// `Type.ast e` / `Type.ast-generic e` — COMPILE-TIME TYPE→AST REFLECTION: reduce `e` to its type-VALUE
    /// (a `(Type.of x)` result or a written type), then fold to the `Ast` VALUE of that type's DEFINITION —
    /// the verbatim `(type Name …)` declaration form (`DESIGN-type-to-ast-reflection.md`). The missing dual
    /// of `encode_ty` (which emits only the type REFERENCE, not the shape). The `(meta apply)` of the `Type`
    /// module's `ast` / `ast-generic` fields. `instantiated = false` (`ast-generic`) yields the GENERIC decl
    /// verbatim (type params intact); `instantiated = true` (`ast`) substitutes the type's concrete args
    /// into the decl (increment 3). Reuses the existing `Ast` sum variants (no encoding change), so the
    /// result flows through `Ast.print`/`Ast.encode`/`Ast.decode` unchanged. Total over concrete types;
    /// declines on a non-concrete type (an unresolved type variable). One prim with a bool (design OQ#2), so
    /// the RUST-backend arm covers one new `Prim`.
    TypeAst {
        instantiated: bool,
    },
    /// `Type.try-as x` — the compile-time "view this value at the EXPECTED type" op: `∀a b. a → (Option
    /// b)`, where the target `b` is INFERRED from usage (the expected `(Option b)` context; ascribe to
    /// force it). Folds at lower time: `Some x` iff `x`'s inferred type STRUCTURALLY equals `b` (the same
    /// strict equality as `Type.eq` — no subtype widening), else `None`. Because `b` is fixed by inference
    /// and `x`'s type is static, this ALWAYS folds to a definite `Some`/`None` — no runtime type tag, no
    /// runtime branch. A target `b` left a free type variable (not inferrable) is a compile-time decline
    /// asking for an annotation. On the `Type` module (`DESIGN-variable-arity-functions.md` §5).
    TryAsType,
    /// A SET TYPE CONSTRUCTOR — the `(meta apply)` of the `Set` prelude module. `(Set Int64)` in type
    /// position builds `Ty::Set(elem)` (ONE parameter, like `List`). The set analogue of `ListCtor`.
    SetCtor,
    /// `Set.of` — construct a set from a LIST of its elements: `∀a. (List a) → (Set a)`, DEDUPLICATING
    /// (each element at most once). Lowers to `set-empty` + a `set-insert` per list element (a constant
    /// list folds to a canonical `Core::SetOf`). The one set CONSTRUCTOR (the set analogue of `Bytes.of`).
    SetOf,
    /// `Map.to-list` — enumerate the map's entries as a `List (Tuple k v)` in CANONICAL KEY order:
    /// `∀k v. (Map k v) → (List (Tuple k v))` (collections-and-text.md §A Map Renders As Its Entries In
    /// Canonical Key Order). Lowers to the runtime `map-to-list` op with a compiler-baked map shape
    /// descriptor. The map companion of `Set.to-list`.
    MapToList,
    /// `Set.to-list` — enumerate the set's elements as a `List` in CANONICAL element-value order:
    /// `∀a. (Set a) → (List a)` (the inverse of `Set.of`; realizes collections-and-text.md §Map/Set
    /// iteration is deterministic). Lowers to the runtime `set-to-list` op with a compiler-baked element
    /// shape descriptor.
    SetToList,
    /// `Set.contains` — the TOTAL membership predicate `∀a. (Set a) → a → Bool` (never traps; no positional
    /// access — a set is unordered). Lowers to the runtime `set-contains` op (returns a `bool` directly,
    /// UNLIKE `Map.lookup`'s Option). A constant set + constant element folds to `ConstBool`.
    SetContains,
    /// `Set.len` — the count of DISTINCT elements, an `Int64` (`∀a. (Set a) → Int64`). Lowers to the
    /// runtime `set-size` op (+ i32→i64 extend). The set analogue of `List.len`/`Map.size`.
    SetLen,
    /// `Set.insert` — add an element, returning the new set: `∀a. (Set a) → a → (Set a)` (functional
    /// construction; inserting a present element is a no-op value). Lowers to the runtime `set-insert`.
    SetInsert,
    /// `Set.remove` — drop an element, returning the new set: `∀a. (Set a) → a → (Set a)` (total — removing
    /// an absent element yields an equal set). Lowers to the runtime `set-remove`.
    SetRemove,
    /// `Set.union` — the set of elements in EITHER set: `∀a. (Set a) → (Set a) → (Set a)`. Lowers to the
    /// runtime `set-union` op (consumes both). A constant pair folds.
    SetUnion,
    /// `Set.intersection` — the set of elements in BOTH sets: `∀a. (Set a) → (Set a) → (Set a)`. Lowers to
    /// the runtime `set-intersection` op.
    SetIntersection,
    /// `Set.difference` — the set of elements in the first set but NOT the second: `∀a. (Set a) → (Set a)
    /// → (Set a)`. Lowers to the runtime `set-difference` op.
    SetDifference,
}

impl Prim {
    /// The primitive a prelude `(intrinsic NAME)` node names, or `None` if unrecognized. The one place
    /// a prim's source spelling is matched — the prelude authors these nodes, so no other pass sees a
    /// name.
    pub fn from_name(name: &str) -> Option<Prim> {
        match name {
            "+" => Some(Prim::Add),
            "-" => Some(Prim::Sub),
            "*" => Some(Prim::Mul),
            "/" => Some(Prim::Div),
            "%" => Some(Prim::Rem),
            "<<" => Some(Prim::Shl),
            ">>" => Some(Prim::Shr),
            "&" => Some(Prim::BitAnd),
            "|" => Some(Prim::BitOr),
            "^" => Some(Prim::BitXor),
            "<" => Some(Prim::Lt),
            ">" => Some(Prim::Gt),
            "<=" => Some(Prim::Le),
            ">=" => Some(Prim::Ge),
            "=" => Some(Prim::Eq),
            "compare" => Some(Prim::Compare),
            "wrap" => Some(Prim::Wrap),
            "checked-of" => Some(Prim::CheckedOf),
            "neg" => Some(Prim::Neg),
            "Int" => Some(Prim::IntCtor),
            "UInt" => Some(Prim::UIntCtor),
            "Float" => Some(Prim::FloatCtor),
            "float-of-int" => Some(Prim::FloatOfInt),
            "float-nan" => Some(Prim::FloatNan),
            "float-inf" => Some(Prim::FloatInf),
            "float-of" => Some(Prim::FloatOf),
            "->" => Some(Prim::FnCtor),
            "Tuple" => Some(Prim::TupleCtor),
            "Record" => Some(Prim::RecordCtor),
            "Bool" => Some(Prim::BoolTy),
            "Unit" => Some(Prim::UnitTy),
            "String" => Some(Prim::StringTy),
            "BigInt" => Some(Prim::BigIntTy),
            "Rational" => Some(Prim::RationalTy),
            "rational-of" => Some(Prim::RationalOf),
            "rational-of-int" => Some(Prim::RationalOfInt),
            "rational-value" => Some(Prim::RationalValue),
            "rational-num" => Some(Prim::RationalNum),
            "rational-den" => Some(Prim::RationalDen),
            "rational-truncate" => Some(Prim::RationalTruncate),
            "rational-floor" => Some(Prim::RationalFloor),
            "rational-ceil" => Some(Prim::RationalCeil),
            "rational-round" => Some(Prim::RationalRound),
            "Char" => Some(Prim::CharTy),
            "char-to-int" => Some(Prim::CharToInt),
            "char-from-int" => Some(Prim::CharFromInt),
            "value-encode" => Some(Prim::ValueEncode),
            "value-decode" => Some(Prim::ValueDecode),
            "Symbol" => Some(Prim::SymbolTy),
            "bigint-of" => Some(Prim::BigIntOf),
            "symbol-of" => Some(Prim::SymbolOf),
            "symbol-to-string" => Some(Prim::SymbolToString),
            "sum-new" => Some(Prim::SumNew),
            "sum-ctor" => Some(Prim::SumCtor),
            "tuple-new" => Some(Prim::TupleNew),
            "record-new" => Some(Prim::RecordNew),
            "record-project" => Some(Prim::RecordProject),
            "record-without" => Some(Prim::RecordWithout),
            "record-merge" => Some(Prim::RecordMerge),
            "record-extend" => Some(Prim::RecordExtend),
            "record-with" => Some(Prim::RecordWith),
            "record-pop" => Some(Prim::RecordPop),
            "tuple-cat" => Some(Prim::TupleCat),
            "tuple-split-at" => Some(Prim::TupleSplitAt),
            "tuple-pop" => Some(Prim::TuplePop),
            "tuple-size" => Some(Prim::TupleSize),
            "list-new" => Some(Prim::ListNew),
            "set-new" => Some(Prim::SetNew),
            "list-len" => Some(Prim::ListLen),
            "list-push" => Some(Prim::ListPush),
            "list-prepend" => Some(Prim::ListPrepend),
            "list-concat" => Some(Prim::ListConcat),
            "list-update" => Some(Prim::ListUpdate),
            "list-at" => Some(Prim::ListAt),
            "ast-splice-lift" => Some(Prim::AstSpliceLift),
            "ast-lift" => Some(Prim::AstLift),
            "ast-encode" => Some(Prim::AstEncode),
            "ast-decode" => Some(Prim::AstDecode),
            "blake3-of" => Some(Prim::Blake3Of),
            "reflect-module" => Some(Prim::ReflectModule),
            "print" => Some(Prim::Print),
            "read" => Some(Prim::Read),
            "gensym" => Some(Prim::Gensym),
            "List" => Some(Prim::ListCtor),
            "bytes-of" => Some(Prim::BytesOf),
            "bytes-len" => Some(Prim::BytesLen),
            "bytes-ty" => Some(Prim::BytesTy),
            "str-scalar-len" => Some(Prim::StrScalarLen),
            "str-byte-len" => Some(Prim::StrByteLen),
            "bytes-at" => Some(Prim::BytesAt),
            "bytes-concat" => Some(Prim::BytesConcat),
            "bytes-slice" => Some(Prim::BytesSlice),
            "bytes-compact" => Some(Prim::BytesCompact),
            "str-at" => Some(Prim::StrAt),
            "str-scalar-at" => Some(Prim::StrScalarAt),
            "str-concat" => Some(Prim::StrConcat),
            "str-slice" => Some(Prim::StrSlice),
            "str-to-bytes" => Some(Prim::StrToBytes),
            "str-from-bytes" => Some(Prim::StrFromBytes),
            "sum-expect" => Some(Prim::SumExpect),
            "trap" => Some(Prim::Trap),
            "checked-add" => Some(Prim::CheckedAdd),
            "checked-sub" => Some(Prim::CheckedSub),
            "checked-mul" => Some(Prim::CheckedMul),
            "wrapping-add" => Some(Prim::WrappingAdd),
            "wrapping-sub" => Some(Prim::WrappingSub),
            "wrapping-mul" => Some(Prim::WrappingMul),
            "Map" => Some(Prim::MapCtor),
            "map-new" => Some(Prim::MapNew),
            "map-empty" => Some(Prim::MapEmpty),
            "map-insert" => Some(Prim::MapInsert),
            "map-merge" => Some(Prim::MapMerge),
            "map-lookup" => Some(Prim::MapLookup),
            "map-remove" => Some(Prim::MapRemove),
            "map-size" => Some(Prim::MapSize),
            "map-to-list" => Some(Prim::MapToList),
            "map-swap" => Some(Prim::MapSwap),
            "map-take" => Some(Prim::MapTake),
            "unit-one" => Some(Prim::UnitOne),
            "unit-base" => Some(Prim::UnitBase),
            "unit-mul" => Some(Prim::UnitMul),
            "unit-div" => Some(Prim::UnitDiv),
            "unit-pow" => Some(Prim::UnitPow),
            "unit-prefix" => Some(Prim::UnitPrefix),
            "unit-of" => Some(Prim::UnitOf),
            "unit-define" => Some(Prim::UnitDefine),
            "unit-in" => Some(Prim::UnitIn),
            "qty-of" => Some(Prim::QtyOf),
            "qty-value" => Some(Prim::QtyValue),
            "qty-pow" => Some(Prim::QtyPow),
            "qty-unit" => Some(Prim::QtyUnit),
            "Qty" => Some(Prim::QtyCtor),
            "type-of" => Some(Prim::TypeOf),
            "type-eq" => Some(Prim::TypeEq),
            "type-ast" => Some(Prim::TypeAst { instantiated: true }),
            "type-ast-generic" => Some(Prim::TypeAst {
                instantiated: false,
            }),
            "type-try-as" => Some(Prim::TryAsType),
            "Set" => Some(Prim::SetCtor),
            "set-of" => Some(Prim::SetOf),
            "set-to-list" => Some(Prim::SetToList),
            "set-contains" => Some(Prim::SetContains),
            "set-len" => Some(Prim::SetLen),
            "set-insert" => Some(Prim::SetInsert),
            "set-remove" => Some(Prim::SetRemove),
            "set-union" => Some(Prim::SetUnion),
            "set-intersection" => Some(Prim::SetIntersection),
            "set-difference" => Some(Prim::SetDifference),
            _ => None,
        }
    }

    /// Whether this primitive is a BINARY INTEGER operation — arithmetic, division, shift, or bitwise.
    /// Every one has the shape `∀a. (Int a) → (Int a) → (Int a)` and folds on two constant integer
    /// operands (a provable trap → CDZ0304); an operand that is not compile-time-known stays a runtime
    /// `Core::Arith`. (A comparison is NOT one of these — its result is `Bool`, handled separately.)
    pub fn is_arith(self) -> bool {
        matches!(
            self,
            Prim::Add
                | Prim::Sub
                | Prim::Mul
                | Prim::Div
                | Prim::Rem
                | Prim::Shl
                | Prim::Shr
                | Prim::BitAnd
                | Prim::BitOr
                | Prim::BitXor
        )
    }

    /// Whether this primitive is a FLOAT arithmetic operator (`+.` `-.` `*.` `/.`) — shape `∀a. (Float
    /// a) → (Float a) → (Float a)`. Distinct from `is_arith` (the integer ops): a float op folds two
    /// constant floats (round-to-nearest-even at the width) and NEVER traps on overflow (IEEE); a runtime
    /// operand emits the machine `f64.add`/… (F4).
    pub fn is_float_arith(self) -> bool {
        matches!(self, Prim::FAdd | Prim::FSub | Prim::FMul | Prim::FDiv)
    }

    /// Whether this primitive is an integer CONVERSION — a unary op from a polymorphic source integer to
    /// a fixed target width. `Wrap` (truncating, returns `T`) is the only one now; the checked `Of`
    /// (returning `Option<T>`) joins it with sum types. Routed as a unary application in `lower`/`select`.
    pub fn is_conversion(self) -> bool {
        matches!(self, Prim::Wrap | Prim::CheckedOf)
    }

    /// Whether this primitive is a relational comparison (`< > <= >=` or equality `=`) — shape `∀a. a →
    /// a → Bool`, a bare type variable so it relates `Bool` and (structurally) any value as well as
    /// integers, with a `Bool` result. Folds two constant SCALARS to a `ConstBool`; a compound/runtime
    /// operand declines. Never traps.
    pub fn is_comparison(self) -> bool {
        matches!(self, Prim::Lt | Prim::Gt | Prim::Le | Prim::Ge | Prim::Eq)
    }

    /// Whether this primitive is a BINARY OPERATOR reached by application (arithmetic OR comparison) —
    /// the set the prelude installs as operator records and `meta_apply_of` dispatches. Used to route
    /// an `Apply` whose head is one of these into the operator-fold path.
    pub fn is_binop(self) -> bool {
        self.is_arith() || self.is_comparison()
    }

    /// The ground type-value this primitive denotes directly, if it is one (`BoolTy`→Bool, …). A
    /// ground type is a type VALUE with no application; a constructor prim returns `None` here (it
    /// yields a type only when applied).
    pub fn ground_type(self) -> Option<crate::ty::Ty> {
        match self {
            Prim::BoolTy => Some(crate::ty::Ty::Bool),
            Prim::UnitTy => Some(crate::ty::Ty::Unit),
            Prim::BytesTy => Some(crate::ty::Ty::Bytes),
            Prim::StringTy => Some(crate::ty::Ty::String),
            Prim::CharTy => Some(crate::ty::Ty::Char),
            Prim::SymbolTy => Some(crate::ty::Ty::Symbol),
            Prim::BigIntTy => Some(crate::ty::Ty::BigInt),
            Prim::RationalTy => Some(crate::ty::Ty::Rational),
            _ => None,
        }
    }

    /// Whether this primitive DENOTES A TYPE — a ground type-value (`Bool`/`String`/…) or a type
    /// CONSTRUCTOR (`List`/`Map`/`Int`/`->`/`Tuple`/… — applied in type position to build a `Ty`). This
    /// is the single role split a `(meta apply)`-carrying record can't reveal on its own: a `List` module
    /// and the `+` operator BOTH resolve through `(meta apply)` to a `Prim`, but `List` builds a TYPE
    /// while `+` computes a VALUE. Which one a prim is is a property the prelude fixes when it authors the
    /// intrinsic — so the fact lives here beside `is_arith`/`ground_type`, not in a name table
    /// downstream. Used only by the highlight query to colour a type-former distinctly from a value op;
    /// it changes no compiled byte. A variant/value constructor (`SumNew`/`TupleNew`/…) is NOT here — it
    /// builds a VALUE, not a type.
    pub fn denotes_type(self) -> bool {
        self.ground_type().is_some()
            || matches!(
                self,
                Prim::IntCtor
                    | Prim::UIntCtor
                    | Prim::FloatCtor
                    | Prim::FnCtor
                    | Prim::TupleCtor
                    | Prim::RecordCtor
                    | Prim::ListCtor
                    | Prim::MapCtor
                    | Prim::SetCtor
                    | Prim::SumCtor
                    | Prim::QtyCtor
            )
    }
}

/// The resolved meaning of one AST node. Children are referenced by AST `StructId`; a query descends
/// by reading their slots on demand.
#[derive(Clone, PartialEq, Debug)]
pub enum Resolved {
    /// An integer literal at its exact arbitrary precision. Its machine width is a downstream type
    /// decision; the narrowing (and any out-of-range decline) happens at selection.
    Int(IntValue),
    /// A boolean literal.
    Bool(bool),
    /// A string literal — its text already normalized to canonical form by the reader (escapes expanded,
    /// NFC). A `Ty::String` constant; folds to a `Core::ConstStr` and escapes as its baked UTF-8 bytes.
    Str(String),
    /// A SYMBOL literal (`#"meter"`, 17-symbols) — the reader-sugar equivalent of `(Symbol.of "meter")`.
    /// Types as `Ty::Symbol` (DISTINCT from `Ty::String` — the nominal boundary, so `(= #"x" "x")` is a
    /// type error CDZ0202, and `(= #"x" (Symbol.of "x"))` is well-typed and true). Its IDENTITY is its
    /// text (content-derived), so a CONSTANT symbol shares the `Core::ConstStr` REP — it lowers to
    /// `Core::ConstStr` exactly like `Symbol.of` on a constant string, and equality folds via the shared
    /// constant-string equality. The `Unit.base #"meter"` builder reads its text directly (a base-dimension
    /// name), so `unit_of` accepts this form as it did the `Str` it used to resolve to.
    SymbolConst(String),
    /// A byte-string literal `b"…"` — the reader unescaped it to raw bytes. A `Ty::Bytes` constant; lowers
    /// to a `Core::BytesOf` of its bytes (same shape `(Bytes.of (list …))` builds), so it bakes at escape,
    /// compares/slices/concats as a constant, and renders back `b"…"`. The companion of `Str` for bytes.
    Bytes(Vec<u8>),
    /// A CHAR literal (`#\a`) — a single Unicode scalar value. Types as `Ty::Char` (DISTINCT from `Int`);
    /// folds to a `Core::ConstChar`. Constant equality/ordering compare by scalar value (`Char.to-int`/
    /// `from-int` and `String.scalar-at` are later increments).
    Char(char),
    /// A FLOATING-POINT literal (`2.0`). Types as `Ty::Float` — DISTINCT from `Ty::Int`, so mixing a
    /// float and an integer in one arithmetic operator is rejected (no silent promotion). Its VALUE does
    /// not yet run: `core_of` DECLINES (there is no float arithmetic / boundary rep yet), so a pure-float
    /// program declines while an int↔float MIX rejects at the type check — both decline-don't-miscompile.
    /// The exact `Decimal` is carried so a future float-arithmetic increment reads the literal value.
    Float(crate::ast::Decimal),
    /// A RATIONAL LITERAL — a `(RationalTag n d)` node the reader emits for BOTH the bare `3/2` literal and
    /// the `#rational(3 2)` alias (v-syntax #6627). `num`/`den` are its two integer-leaf children. Types as
    /// `Ty::Rational`; folds to the SAME normalized `Core::ConstRational` that `Prim::RationalOf` produces
    /// (gcd-reduce, sign on the numerator) by routing through `lower_rational_of` — so `3/2` and
    /// `(Rational.of 3 2)` are identical downstream. WITHOUT this, the `(RationalTag …)` list fell to the
    /// `Resolved::Apply` path, whose head is the `Leaf::Rational` atom → the bare-head CDZ0201 (resolve.rs)
    /// → a bare rational literal was UNCOMPILABLE on every surface incl. the binary-AST path (v-ast-compound).
    Rational {
        num: crate::ast::StructId,
        den: crate::ast::StructId,
    },
    /// The unit value (`()`).
    Unit,
    /// A reference to a binding: the name at this occurrence denotes the value at `value` (the
    /// initializer of the nearest enclosing `let`/`def`-parameter binding of that name). `type_of` and
    /// `core_of` follow the ref — a bare name IS its bound value's fact.
    Ref { value: StructId },
    /// A `let` binding form: each `(name init)` pair binds `name` to `init` for the initializers and
    /// body that follow (sequential; a later binding sees earlier ones; a repeat shadows). The whole
    /// form's value is `body`'s value. Bindings are carried as `(binder-name-occ, init-occ)` pairs so
    /// scope resolution finds them by walking here from a reference.
    Let {
        bindings: Vec<(StructId, StructId)>,
        body: StructId,
    },
    /// A two-way conditional. The three children are AST occurrences resolved on demand.
    If {
        cond: StructId,
        then_: StructId,
        else_: StructId,
    },
    /// A logical conjunction `(and a b)` / disjunction `(or a b)` — a SHORT-CIRCUITING boolean connective
    /// (core-semantics.md §Boolean Connectives Short-Circuit). Both operands are Bool; `and` evaluates
    /// `rhs` only when `lhs` is true, `or` only when `lhs` is false — so a connective SHIELDS a trapping/
    /// effectful right operand exactly as a conditional's unselected branch does. Control flow, not a
    /// strict value operator, so it is grammar (like `if`), not a prelude record (which would eval both).
    /// `is_and` distinguishes the two (they share the same short-circuit shape, differing only in which
    /// constant the shielding branch yields). Lowers to a `Core::And`/`Core::Or` the backend emits as an
    /// `if`. (Negation `(not a)` is a strict one-operand form — `Resolved::Not`.)
    And {
        lhs: StructId,
        rhs: StructId,
        is_and: bool,
    },
    /// A logical negation `(not a)` — `a` is a Bool, the result its complement. Strict (one operand,
    /// nothing to shield). Lowers to `Core::Not` (emitted `i32.eqz`).
    Not { operand: StructId },
    /// The fallible short-circuit operator `(try e)` — the s-expr form of the ML postfix `e?`
    /// (`DESIGN-try-operator-rcdzc.md`). `operand` is a `Result(a, b)` / `Option(a)` expression; `(try e)`
    /// UNWRAPS its success payload (yielding `a`) on the normal path, and on the failure variant SHORT-
    /// CIRCUITS the enclosing fallible boundary (the enclosing function's `Result`/`Option` result type),
    /// making the boundary's value the failure itself. NOT a monad wedge and NOT a user-visible effect: it
    /// is a compile-time desugar onto the effects system's within-function abortive lowering (a synthesized
    /// `Mir::Block` + `Mir::Break`, reusing the E4 substrate), so it contributes nothing to the effect row.
    /// Carried as a first-class node through resolve/infer so a type error points at the `?`, then desugared
    /// at Hir→Mir against the boundary. A `?` with no fallible boundary is CDZ0230; a `?`'d type that
    /// disagrees with the boundary is the ordinary CDZ0203 mismatch.
    Try { operand: StructId },
    /// A `(match scrutinee (pattern body)…)` — the pattern engine's surface. `scrutinee` is the value
    /// examined; each arm is a `(pattern-occ, body-occ)` pair, tried top-to-bottom. A pattern is carried
    /// as its AST occurrence (NOT a `Pattern` enum — `intermediate-representations.md`: patterns are
    /// ordinary nodes classified where consumed), so a literal pattern is an `Int`/`Bool` node and the
    /// wildcard is the name `_`. A SCALAR scrutinee is handled with literal, binder, and wildcard arms:
    /// an arm is a probe `scrutinee == literal` (or always, for a binder/`_`) and its body; the match
    /// lowers to a chain of `if`s (folded when the scrutinee is constant). A sum/tuple/record scrutinee
    /// walks the value heap rather than probing a scalar.
    Match {
        scrutinee: StructId,
        arms: Vec<(StructId, StructId)>,
    },
    /// A record literal: a fixed SET of named fields, each label mapping to its value occurrence. Held
    /// as a `BTreeMap` so the fields are canonically ordered (order-independent equality/projection)
    /// and a field lookup is O(log n), not a linear scan. The labels are symbols (never resolved); the
    /// values resolve on demand. A duplicate label is a `Poison` before construction (a record's field
    /// names are a set — `core-semantics.md` §A Record Has A Fixed Set Of Named Fields).
    /// (`fields` behind an `Rc` so CLONING a `Resolved::Record` — which `resolved_of` does on every
    /// memoized read — is a refcount bump, not a deep map copy. A record read field-by-field
    /// (`member_value` re-clones the operand's resolved form per access) was O(N²) in map clone;
    /// mirrors the `Ty::Record` Rc choice, faithful to Cadenza's ref-counted port target.)
    Record {
        fields: std::rc::Rc<BTreeMap<Symbol, StructId>>,
    },
    /// Member access `(. operand key)` — the ONE generic projection. `key` is a label read from the
    /// key occurrence's spelling, NOT resolved (`prelude-and-resolution.md` §Member Access Is One
    /// Generic Projection That Does Not Inspect Its Key). The projection resolves the field against
    /// the operand's type/value downstream.
    Member { operand: StructId, key: Symbol },
    // (The `Resolved::Extern` variant was REMOVED in U4: cross-component interop is unified with EFFECTS —
    // a peer op is an escaping effect bound to a peer contract, resolved via the ordinary member-access /
    // perform path, not a dedicated extern-op reference.)
    /// A TUPLE literal `(tuple e0 e1 …)` — a fixed-arity POSITIONAL product. The elements are AST
    /// occurrences in order (resolved on demand); the tuple's ARITY and per-position element types ARE
    /// its type (a tuple of different arity or a differently-typed position is a different type —
    /// `type-system.md` §The Structural Types Are Record, Tuple, And Sum). Distinct from `Record` (named
    /// fields): a tuple is accessed by POSITION (`Proj`), a record by NAME (`Member`).
    /// (`elems` behind an `Rc<[StructId]>` so cloning a `Resolved::Tuple` is O(1) — same rationale as
    /// `Record`; a tuple projected element-by-element re-clones the operand's resolved form per access.)
    Tuple { elems: std::rc::Rc<[StructId]> },
    /// A LIST literal `(list e0 e1 …)` — a HOMOGENEOUS variable-length sequence. The elements are AST
    /// occurrences in order (resolved on demand); every element's type unifies to ONE element type (a
    /// mixed list is ill-typed — CDZ0203), so its type is `Ty::List(elem)`. Distinct from `Tuple` (a
    /// fixed-arity product with per-position types): a list's length is a runtime property and all
    /// elements share a type. Built on the persistent `vec-*` heap at run time.
    List { elems: std::rc::Rc<[StructId]> },
    /// A SET literal `(set e0 e1 …)` — a first-class tagged homogeneous set construction (operator ruling
    /// 2026-08-27: data structures are pulled all the way through the compiler, NOT desugared to
    /// `Set.of`-over-list in the reader). Elements are AST occurrences in order; DUPLICATES collapse at
    /// build (set semantics), and every element unifies to ONE element type, so its type is `Ty::Set(elem)`.
    /// It lowers to `Core::SetOf` — the SAME node `Set.of (list …)` produces — so the set VALUE still
    /// renders `(Set.of (list …sorted))`. The set analogue of `List`.
    Set { elems: std::rc::Rc<[StructId]> },
    /// A MAP literal `(map (k v) …)` — a persistent association of keys to values. Each entry is a
    /// `(key-occ, value-occ)` PAIR of ORDINARY VALUE occurrences — the key is NOT a compile-time label
    /// (unlike a `Record` field, read by `read_key` into a `Symbol`): it is an expression resolved in
    /// scope, so a bound name keys by its VALUE (`(let ((a 5)) (map (a 1)))` is the map at key 5), a
    /// computed key `(+ 2 3)` is a runtime key, and an unbound key is the ordinary CDZ0101 scope error
    /// (never coerced to a String). Both axes are HOMOGENEOUS — all keys unify to one type, all values
    /// to one — so its type is `Ty::Map(key, value)`; two maps with different KEY SETS are the SAME type
    /// (the keyset is runtime data, unlike a record's fixed field set). Built on the persistent CHAMP
    /// `map-*` heap at run time; a later duplicate key overwrites (keys compared by value).
    Map {
        entries: std::rc::Rc<[(StructId, StructId)]>,
    },
    /// The SUB-VALUE a MAP PATTERN's binder binds. A map pattern `(map (k p) … .. rest)` is a KEY-DIRECTED
    /// lookup (ask-61, core-semantics.md §A Map Is Matched By Key-Directed Patterns): a VALUE binder `p` at
    /// key `k` binds the value the map holds at `k` (a `Map.lookup`), and the REST binder binds the map
    /// with the named keys removed (a `Map.remove` per named key). `scrutinee` is the match scrutinee; a
    /// value binder carries `key = Some(k)` (its type is the map's VALUE type), the rest binder `key =
    /// None` (its type is the map type) with `named` the keys removed. Over a CONSTANT `Core::MapNew`
    /// scrutinee both fold at lowering (`lower_match_map`): a value binder to the entry's value, the rest
    /// binder to a `Core::MapNew` minus the named keys. Scoped to its arm (resolve Case M), the map analogue
    /// of `SumPayload`/`BinField`.
    MapField {
        scrutinee: StructId,
        /// The access PATH from `scrutinee` down to the matched MAP — EMPTY when the map pattern is the
        /// direct match scrutinee (`(match m ((map (k v)) …))`), or `Elem`/`Payload` steps when the map is
        /// NESTED inside a tuple/record/variant pattern (`(match t ((tuple (map (k v)) …) …))` — the map is
        /// at `Elem(0)` of the tuple scrutinee). Walked to reach the nested `Ty::Map`/`Core::MapNew` before
        /// the value/rest read, exactly as `SumPayload.steps` reaches a nested payload.
        path: std::rc::Rc<[crate::core::PathStep]>,
        /// `Some(key)`: a VALUE binder at `key`. `None`: the REST binder (scrutinee minus `named`).
        key: Option<StructId>,
        /// The keys the pattern NAMES — removed to form the rest map. Empty for a value binder.
        named: std::rc::Rc<[StructId]>,
        /// The access sub-path INTO the value at `key`, for a value binder NESTED inside a value
        /// sub-pattern — `(map ("a" (tuple x y)))` binds `x` with `value_steps=[Elem(0)]`, `(map ("a"
        /// (Some n)))` binds `n` with `[Payload]`. EMPTY when the value position is a bare binder (the
        /// common case — the value IS the binder). Applied AFTER the key lookup, exactly as
        /// `SumPayload.steps` walks into a variant payload. Unused for the REST binder (`key = None`).
        value_steps: std::rc::Rc<[crate::core::PathStep]>,
        /// The variant HEADS at each `Payload` step in `value_steps` (for a ctor value sub-pattern), so the
        /// nested binder's type is read at the right instantiation — the value-sub-path twin of
        /// `SumPayload.heads`. Empty when `value_steps` has no `Payload` step.
        value_heads: std::rc::Rc<[StructId]>,
    },
    /// The FIELD a RECORD sub-pattern's binder binds, when the record is NESTED inside a tuple/list/variant
    /// match pattern — `(match t ((tuple (record (x a)) c) …))` binds `a` to field `x` of the record at
    /// tuple-slot 0. `scrutinee` is the match scrutinee; `path` is the `Elem`/`Payload` steps from the
    /// scrutinee down to the nested RECORD value (walked to reach the `Ty::Record`/`Core::Record` before the
    /// field read, exactly as `SumPayload.steps` reaches a nested payload); `key` is the field's `Symbol`,
    /// whose sorted SLOT resolves at FOLD time (`eval::runtime_member_index`), exactly like a top-level
    /// record arm's `Resolved::Member` folds to a `Core::Proj` at the field's sorted slot. Simpler than
    /// `MapField` (a record field read is a `Member` + a path prefix — no rest binder, no named-set, no
    /// key-directed lookup). A TOP-LEVEL record arm is `Resolved::Member`'s job (Case 6rec, empty path); this
    /// is the NESTED-in-compound twin. Scoped to its arm (resolve Case 6rec-nested).
    RecordField {
        scrutinee: StructId,
        path: std::rc::Rc<[crate::core::PathStep]>,
        key: Symbol,
        /// The descent BELOW the field value, when the field value is ITSELF a compound sub-pattern binding
        /// `name` deeper — `(record (= x (tuple c d)))` binds `c` at `sub_path = [Elem(0)]` of field `x`'s
        /// value (§235: a record field value binder may be any nested pattern to any depth). EMPTY for a
        /// BARE-binder field (the common case — `name` IS field `x`). A record field read lowers to an
        /// `Elem(<sorted-slot>)` step (records are flat arrays read by slot), so the runtime walk is
        /// `path ++ [Elem(slot_of_key)] ++ sub_path` — all ordinary `Elem`/`Payload` steps the `SumPayload`
        /// walker already handles; no new `PathStep` kind. `sub_heads` covers any `Payload` step in `sub_path`.
        sub_path: std::rc::Rc<[crate::core::RecordSubStep]>,
        /// The variant-constructor head at EACH `Payload` step in `path` THEN in `sub_path`, in order — so
        /// inference can walk the scrutinee's type level by level through a variant payload to reach the
        /// nested `Ty::Record` (a record nested UNDER a variant, `(Wrap (record (= x a)))`) and any variant
        /// below the field. One entry per `Payload` step; EMPTY for the common tuple/list-only nesting (all
        /// `Elem` steps). The record-field twin of `SumPayload.heads`.
        heads: std::rc::Rc<[StructId]>,
    },
    /// The RESIDUAL RECORD a record REST pattern's `.. rest` binder binds — `(match r ((record (= a x)
    /// (.. rest)) …))` binds `rest` to a NEW record of the scrutinee's fields MINUS the `named` ones
    /// (`{ b = r.b, c = r.c }` when the record is `{a,b,c}` and `a` is named). The record twin of a
    /// `MapField` REST binder (`key = None`), and the residual counterpart of `RecordField`: a record's
    /// field set is STATIC (from the solved scrutinee type), so the residual is a fixed field-subset GATHER
    /// (not a runtime split) — its type is `(Record …)` over the non-named fields, and it folds over a
    /// CONSTANT record to a `Core::Record` of those fields. `scrutinee` is the match scrutinee; `named` are
    /// the field-name occurrences the pattern NAMES (removed to form the residual). A top-level record arm
    /// (empty path); a nested-in-compound record rest is a later increment.
    RecordRest {
        scrutinee: StructId,
        /// The field-name occurrences the pattern NAMES — removed from the scrutinee's record to form the
        /// residual record `rest`. Read for their spellings (`read_key`) at fold/type time.
        named: std::rc::Rc<[StructId]>,
    },
    /// The RESIDUAL SET a set REST pattern's `.. rest` binder binds — `(match s (#set(e1 (.. rest)) …))`
    /// binds `rest` to the scrutinee set MINUS the named ELEMENTS (`Set.remove(s, e1)`). The set twin of a
    /// `MapField` REST binder / `RecordRest`. UNLIKE a record (whose residual NARROWS to fewer fields), a
    /// set's residual is the SAME set TYPE `(Set E)` — removing elements does not change the element type,
    /// exactly like a `MapField` rest keeps the map type. `scrutinee` is the match scrutinee; `named` are
    /// the element VALUE-expression occurrences the pattern names (removed to form the residual). The
    /// residual VALUE is built by the set-matcher desugar (`desugar_runtime_set_match`, a `Set.remove`
    /// chain — v-ast-compound's slice); this variant carries the TYPE (v-inference's slice), so a body
    /// reference to `rest` type-checks as `(Set E)`.
    SetRest {
        scrutinee: StructId,
        /// The element VALUE-expression occurrences the pattern NAMES — removed (via `Set.remove`) to form
        /// the residual set. Unused for TYPING (the residual set type is the scrutinee's set type); read by
        /// the set-matcher desugar to build the `Set.remove` chain.
        named: std::rc::Rc<[StructId]>,
    },
    /// A tuple PROJECTION `(. operand N)` — member access whose key is an INTEGER literal selects the
    /// element at position `index` (0-based). The integer key is what distinguishes a positional tuple
    /// access from a named record field access (`Member`); a name key on a tuple, or an integer key on a
    /// record, is a type error decided downstream. An `index` outside the operand tuple's static arity is
    /// a COMPILE-TIME type error (CDZ0201), never a runtime trap (`type-system.md` §A Tuple Is Split At A
    /// Position Into A Prefix And A Suffix).
    Proj { operand: StructId, index: usize },
    /// The SUB-VALUE a sum-variant pattern's binder binds, at an access PATH from the scrutinee.
    /// `(match s ((Some x) x))` resolves `x` here with `path=[Payload]`; `(match s ((Some (Some y)) y))`
    /// resolves `y` with `path=[Payload, Payload]` — the NESTED binder reaches the payload's payload.
    /// `scrutinee` is the match scrutinee occurrence; `steps` is the access path (`Payload`/`Elem`);
    /// `variant_head` is the INNERMOST variant-constructor occurrence (`(. Sum Variant)` or bare `Some`)
    /// whose `(-> payload Sum)` gives the binder's type at the scrutinee's instantiation. At lowering it
    /// becomes `Core::SumPayload { scrutinee, path }` (walk the path, unbox). A pattern binder is scoped
    /// to its arm (resolve Case 6), the sum analogue of the scalar Case 5 — but binding a nested payload.
    SumPayload {
        scrutinee: StructId,
        steps: std::rc::Rc<[crate::core::PathStep]>,
        /// The variant-constructor head at EACH `Payload` step, in order — so inference can walk the
        /// scrutinee's type level by level (each head's `(-> payload Sum)` gives the next sub-value's
        /// type at that instantiation). The last head encloses the binder. One entry per `Payload` step.
        heads: std::rc::Rc<[StructId]>,
    },
    /// A NATIVE primitive value — what a prelude `(intrinsic …)` node resolves to (an arithmetic
    /// operation or a type constructor). The irreducible bottom a `Meta.apply` names; carried as a
    /// VALUE and reduced/lowered by the machinery that owns it, never special-cased by name
    /// (`reference-compiler.md` §A Built-In Operation Is A First-Class Value, Lowered At Selection).
    Prim(Prim),
    /// Application `(head arg…)` — the ONE application form. `head` and each `arg` are AST occurrences
    /// resolved on demand; to apply, project the head value's `(meta apply)` and use it if applyable
    /// (else reject "not applyable"). One path serves an operator, a type constructor, and (later) a
    /// user function — dispatch is by the head value's meta channel, never its spelling
    /// (`prelude-and-resolution.md` §A Form Whose Head Is Not A Grammar Name Is Dispatched By The Kind
    /// Of Value Its Head Resolves To).
    /// (`args` behind an `Rc<[StructId]>` so CLONING a `Resolved::Apply` — which `resolved_of` does on
    /// EVERY memoized read — is a refcount bump, not a fresh heap `Vec`. An application is the most
    /// common node in operator-heavy / call-heavy programs, and every inline re-reads it; the per-clone
    /// `Vec` alloc/free was a top allocation source in the profile. Same rationale as the `Tuple`/
    /// `Record`/`Lambda.params` Rc choice.)
    Apply {
        head: StructId,
        args: std::rc::Rc<[StructId]>,
    },
    /// A TYPE ANNOTATION `(: expr ty_expr)` — the value of `expr`, with its type CONSTRAINED to the
    /// type `ty_expr` denotes. The annotation is transparent to the value: `(: e T)` evaluates and
    /// lowers exactly as `e` (the annotation ERASES). Its force is on inference — the type `ty_expr`
    /// reduces to is unified into `expr`'s type, so `(: 5 Int64)` pins the literal's width and `(: true
    /// Int64)` is a conflicting-use rejection (CDZ0203). This is what disambiguates an otherwise-
    /// ambiguous type (an integer parameter with no other constraint), which is why it must exist
    /// before a runtime parameter can be given a definite machine width. Both children are AST
    /// occurrences: `expr` the annotated value, `ty_expr` the type EXPRESSION — reduced to a `Ty` by
    /// the evaluator downstream (`typeval_of`), NOT here, since resolve is a pure per-node classify and
    /// reducing a type constructor like `(Int 8)` needs the evaluator.
    Annot { expr: StructId, ty_expr: StructId },
    /// A `(const <expr>)` FORCE-EVAL block — the operator's explicit const-demand construct (an
    /// EXPRESSION form, distinct from the `(const (: d T))` PARAM modifier that `strip_const_params`
    /// unwraps at load; that pass never touches a body, so a `(const …)` reaching resolve in expression
    /// position is this block). It is SEE-THROUGH for resolution + typing — `type_of((const e)) ==
    /// type_of(e)` — like [`Annot`], carrying the inner `expr` occurrence so every pass follows `.expr`.
    /// Its FORCE-EVAL semantics live in lowering (v-compiler-primitives): `core_of` const-EVALUATES the
    /// inner expr and REJECTS if it does not fully fold, and it is the const-DEMAND CONTEXT signal the
    /// recursive-const-fold unroll reads (the general-transform demand marker, replacing threading const
    /// PARAMS through args). This front-end arm only classifies + types-through; the fold-or-reject is
    /// downstream.
    ConstBlock { expr: StructId },
    /// A first-class TYPE value. A type is an ordinary value (mixable, returnable) — using `Bool` in
    /// type position projects a record's `(meta t)` field, which holds one of these; a type
    /// constructor applied (`(Int a)`, `(-> A B)`) reduces through the one evaluator to one of these.
    /// It is compile-time-only: the erasure fence forbids it reaching the runtime boundary.
    TypeVal(crate::ty::Ty),
    /// A lambda PARAMETER occurrence used as a value — a formal not yet substituted. `infer` gives it
    /// a fresh type variable (the parameter's type, to be solved); the evaluator substitutes the
    /// argument here when the lambda is β-reduced at application. `binder` is this parameter's own
    /// occurrence (its identity), so two references to the same parameter share one variable.
    Param { binder: StructId },
    /// A compile-time lambda `(fn (param…) body)` — a value. Its parameters bind in scope for `body`
    /// (the ordinary parameter-scope mechanism); the evaluator β-reduces it when applied. An
    /// operator's `Meta.t` is such a lambda over the width (`(fn (a) (-> (Int a) …))`), so a "type
    /// scheme" is just a compile-time lambda from a type/width to a type — instantiation is applying
    /// it to a fresh variable. Params are the binder-name occurrences; `body` is the body occurrence.
    Lambda {
        // `params` behind an `Rc<[StructId]>` so cloning a `Resolved::Lambda` is a refcount bump; a
        // def name resolving to its lambda is read once per call site, and `resolved_of` clones on
        // every read. Same rationale as `Record`/`Tuple`.
        params: std::rc::Rc<[StructId]>,
        body: StructId,
    },
    /// A `(handle INIT (ARM…) BODY)` — an in-program effect handler establishing a context for `body`
    /// (`capabilities-and-effects.md` §An Effect That Does Not Escape Is Discharged By A Handler). This is
    /// how a program raises an effect operation and discharges it entirely in-program — a handler wraps a
    /// sub-computation and gives its operations a context — with no host capability needed.
    //= spec/capabilities/capabilities-and-effects.md#an-effect-that-does-not-escape-is-discharged-by-a-handler
    //# A program MUST be able to raise an effect operation and discharge it with a handler that establishes a context for the sub-computation it wraps, so that an effect handled entirely within the program is expressible without a host capability.
    /// `init`
    /// is the seed state (evaluated where the handle is installed); each arm discharges one operation; the
    /// whole form's value is `body`'s value, with the accumulated state observable only through the
    /// operations. Children are AST occurrences resolved on demand. The compile-time evaluator reduces a
    /// `handle` away — resolving each enclosed performance to a concrete arm (a compile-time constant) and,
    /// for the tail-resumptive shipping surface, rewriting it to plain code (`reference-compiler.md`
    /// §Effects Are Classified First And Resolved By Monomorphization). Until that lowering lands, a
    /// `handle` DECLINES (the surface is recognized so a handled perform stops erroring on unbound
    /// `resume`; it does not yet run).
    ///
    /// `init` is the initial state the handler establishes for the sub-computation, fixed where the
    /// handle is installed (no ambient state); the whole form evaluates to `body`'s value, and the
    /// accumulated state is observable only THROUGH the effect's operations (reading state is the same
    /// mechanism as any other operation — no separate result form).
    //= spec/capabilities/capabilities-and-effects.md#a-handler-threads-state-across-the-operations-it-discharges
    //# A handler MUST establish an initial state for the sub-computation it wraps, fixed where the handler is installed, so that a handler that carries state receives its seed explicitly and no state is ambient.
    //= spec/capabilities/capabilities-and-effects.md#a-handler-threads-state-across-the-operations-it-discharges
    //# A handler MUST evaluate to the value of its body, with the state it accumulated observable only through the operations the effect declares, so that reading the accumulated state is the same mechanism as any other operation and a handler needs no separate result form. Mutation is the instance of this that reads and updates the threaded state, so that a program expresses mutable-looking computation without the value heap becoming mutable.
    Handle {
        init: StructId,
        /// The handler arms, behind an `Rc` so CLONING a `Resolved::Handle` (which `resolved_of` does on
        /// every memo read) is a refcount bump, not a deep `Vec<HandleArm>` copy — each `HandleArm` itself
        /// holds a `params: Vec`, so an N-arm handler's clone was O(N). A perform's `perform_host_target`
        /// walks PARENTS (`resolved_of` each) to find its enclosing `(host …)`, passing THROUGH the N-arm
        /// handle node every time — so re-cloning its arms per walk made a wide handler O(N²). Mirrors the
        /// `Rc` on `Tuple`/`List`/`Apply` and `Core::Record` for the identical clone-on-read reason.
        arms: std::rc::Rc<[HandleArm]>,
        body: StructId,
    },
    /// A resumption `(resume VALUE NEXT-STATE)` inside a handler arm — hand `value` back to the point that
    /// performed the operation and thread `next_state` forward as the state the rest of the handled region
    /// sees (`capabilities-and-effects.md` §A Handler Threads State`). Discharging an operation thus
    /// produces BOTH the delivered value AND the next state, folding a state across the operations purely
    /// (no mutable value heap). Modeled as a NODE (not a
    //= spec/capabilities/capabilities-and-effects.md#a-handler-threads-state-across-the-operations-it-discharges
    //# Discharging an operation MUST produce both the value delivered to the point that performed the operation and the next state carried forward to the rest of the sub-computation, so that a handler folds a state across the sequence of operations its body performs and threads it purely, without the value heap becoming mutable.
    /// fold-time-only rewrite marker) so the tail-resumptive rewrite (`Resume{v,s'}` → `v`, thread `s'`) is
    /// a structural classification, and so an abortive arm (no `Resume`) and a general arm (a non-tail
    /// `Resume`, or `k` captured as a value) are representable without an IR migration
    /// (`DESIGN-effects-rcdzc.md` §2.3). Outside a handler arm, `resume` is meaningless — a `Resume` that
    /// is not consumed by an enclosing arm's lowering is a decline.
    Resume {
        value: StructId,
        next_state: StructId,
    },
    /// A `(host (EFFECT…) BODY)` — an ENTRYPOINT delegation routing its listed effects to the component
    /// boundary (`capabilities-and-effects.md` §Host-Binding Is A Routing Decision Made At The Entrypoint).
    /// `effects` are the delegated effects' name occurrences; `body` is the delegated computation. Admitted
    /// only at an entrypoint; its manifest contribution is handled at serialization (E2). Until host
    /// lowering lands, a `host` DECLINES (surface recognized, not yet run).
    Host {
        // `effects` behind an `Rc<[StructId]>` so cloning a `Resolved::Host` is a refcount bump, not an
        // O(N) copy — the same cheap-clone migration as the `Tuple`/`List`/`Apply`/`Lambda` siblings
        // (fix-33/50/53 family); the universal `resolved_of` memo-hit clones this on every read.
        effects: std::rc::Rc<[StructId]>,
        body: StructId,
    },
    /// A `(bin <segment>…)` binary form — DUAL DIRECTION (like `(Some 5)` builds / `(Some n)` matches):
    /// in EXPRESSION position it CONSTRUCTS a `Bytes` from the segments (each a fixed-width int / bit-field
    /// / bytes splice); in PATTERN position (a `match` arm) it DESTRUCTURES a `Bytes` scrutinee. The
    /// `segs` are parsed once at resolve (kind/width/endian/sign known from the head name), the slot kept
    /// as an AST occurrence (a constant value to encode when building, a binder/literal-probe when
    /// matching). `Ty::Bytes` in value position (`binary-syntax`). A well-formedness fault (mis-aligned
    /// bit-fields, non-final unsized `(bytes …)`, non-const `bits` width) is CDZ0220, checked from `segs`.
    // `segs` behind an `Rc<[Segment]>` so cloning a `Resolved::Bin` is a refcount bump (the `BinField`
    // sibling already holds its `segs` as `Rc<[Segment]>`; this finishes the migration).
    Bin { segs: std::rc::Rc<[Segment]> },
    /// A reference to a `bin` PATTERN binder — the value a segment binder decodes from the matched Bytes
    /// scrutinee (the binary analogue of `SumPayload`, resolve Case B). `(match b ((bin (u16 n)) n) …)`:
    /// the `n` in the body resolves here, carrying the enclosing match's `scrutinee` and the segment whose
    /// binder it is. An INTEGER segment binder has type `Ty::Int` (decoded value); a `Bytes` segment
    /// binder has type `Ty::Bytes`. Lowered by decoding the segment from the scrutinee (const-folded when
    /// the scrutinee is a visible `Core::BytesOf`; the runtime cursor read is BN4). `seg_index` is the
    /// segment's position; `segs` the whole pattern's segments (so the decoder knows each preceding
    /// segment's width to compute this one's byte offset).
    BinField {
        scrutinee: StructId,
        segs: std::rc::Rc<[Segment]>,
        seg_index: usize,
    },
    /// A produced "no": an unrecognized head, a malformed form, an unbound name, or an unmodeled
    /// literal. Carries its reject/decline so the fault is reported at the node it was found.
    Poison(Reject),
}

/// The KIND of a `bin` segment — what it encodes/decodes. A fixed-width integer carries its byte width
/// and signedness; `Bits` a compile-time-constant bit width; `Bytes` a byte-sequence splice/bind (with
/// an optional size occurrence for the dependent `(bytes b n)` form).
#[derive(Clone, PartialEq, Debug)]
pub enum SegKind {
    /// A fixed-width integer: `width` BYTES (1/2/4/8 for u8/u16/u32/u64), `signed` = two's-complement
    /// (`iNN`). Big-endian unless the segment's `le` modifier is set.
    Int { width: u8, signed: bool },
    /// A bit-field of `k` bits (`(bits v k)`), `k` a compile-time constant read at resolve. Sub-byte;
    /// the running bit-sum across a `bin` must close to whole bytes (else CDZ0220).
    Bits { k: u32 },
    /// A byte-sequence segment `(bytes b [n])`: splice all of `b` (build) / bind the rest or exactly `n`
    /// bytes (match). `size` is the optional dependent-size occurrence (`n`); `None` = unsized (final).
    Bytes { size: Option<StructId> },
    /// A UTF-8 string segment `(utf8 s n)`: `size` is the dependent-size occurrence (`n`, an earlier
    /// integer segment binder). In pattern position it reads exactly `n` bytes and DECODES them as
    /// strict UTF-8 — a well-formed sequence binds `s : String`, an ill-formed one is a NON-MATCH (never
    /// a trap: `collections-and-text.md #Decoding Bytes To A String Is Total, Not Trapping`). Unlike
    /// `Bytes`, the size is REQUIRED (always `(utf8 s n)`), so there is no unsized/final form.
    Utf8 { size: StructId },
}

/// One `(kind slot [modifier])` segment of a [`Resolved::Bin`]. `slot` is the value/binder/literal
/// occurrence (resolved on demand — a constant to encode, or a pattern binder/literal probe); `kind`
/// says how to encode/decode it; `little_endian` is the `le` modifier (int segments only).
#[derive(Clone, PartialEq, Debug)]
pub struct Segment {
    pub kind: SegKind,
    pub slot: StructId,
    pub little_endian: bool,
}

/// One arm of a [`Resolved::Handle`] — the discharge of a single operation. Two surfaces. The TAIL/abortive
/// form (the shipping 4-part arm) is `(E.op (params…) state body)`: the continuation is implicit; `body`
/// contains `Resume` node(s) (tail-resumptive) or none (abortive). The GENERAL / `ctl`-style form (the E5
/// 5-part arm) is `(E.op (params…) state k body)`: `k` is an explicit binder for the delimited CONTINUATION
/// as a first-class value (a `Ty::Cont`), so `body` may STORE it (in a list/map) and `resume` becomes
/// `apply(k, v)` — the STORED/RESUMABLE-later case a DES scheduler needs (`DESIGN-general-continuations-e5.md`).
/// `cont` is `Some(k)` for the general form, `None` for the tail/abortive form.
/// `op` is the operation's projection occurrence (`(. E op)`), which carries the op's identity via its
/// `(meta effect-op)` channel; `params` are the operation's parameter binders (bound in `body`); `state`
/// is the current-state binder (the left-fold accumulator, bound in `body`); `body` is the arm body.
/// Children are AST occurrences; scope for `params`/`state`/`cont` is handled by the ordinary parent-walk (a
/// reference in `body` finds its binder), so the arm records only the shape.
#[derive(Clone, PartialEq, Debug)]
pub struct HandleArm {
    pub op: StructId,
    // `params` behind an `Rc<[StructId]>` so cloning a `HandleArm` (the `Handle.arms` `Rc<[HandleArm]>`
    // slice, and the per-arm `.clone()` in the effects fold) is a refcount bump, not an O(params) copy.
    pub params: std::rc::Rc<[StructId]>,
    pub state: StructId,
    /// The explicit continuation binder `k` of a general (`ctl`-style) 5-part arm, or `None` for the
    /// tail/abortive 4-part arm. An arm with `cont: Some(_)` is E5-general — it binds the continuation as a
    /// value; classified distinctly and (until the E5 frame-capture increment lands) declined cleanly.
    pub cont: Option<StructId>,
    pub body: StructId,
}
