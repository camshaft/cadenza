//! The core node form — one entry of the core column, the A-normal rung the one evaluator runs over
//! and a backend consumes.
//!
//! Like the resolved column, this is a *column* over the AST's own `StructId`, not a second arena:
//! `core_of(id)` fills one node's core form, referencing children by their AST `StructId`. It is in
//! **A-normal form** — every operand of an operation is an atom (a constant or, later, a bound name)
//! — so value flow is explicit (`reference-compiler.md` §The Core Representation Is In A-Normal
//! Form). It retains *structured* control (`If`); a linearizing backend flattens that itself
//! (`intermediate-representations.md` §The Fully-Linearized Block Form Is A Linearizing Backend's
//! Representation).
//!
//! **On administrative bindings.** A-normalization names a non-trivial subexpression with a binding
//! (`Let`/`LocalRef`) so its value flow is explicit rather than implicit in an expression's nesting
//! (`reference-compiler.md` §The Core Representation Is In A-Normal Form). This rung realizes the
//! FIRST case where naming earns its keep: a source `let` whose bound value is a RUNTIME computation
//! (not a compile-time constant) used MORE THAN ONCE. Today such a binding would be followed through
//! at each reference — recomputing the value per use — so [`Core::Let`] names it once and each use is
//! a [`Core::LocalRef`] to that name. A binding used at most once, or one whose value folds to a
//! constant, is still copy-propagated / erased at lowering (the ADMIN-REDEX ELIMINATION the one
//! evaluator owes — `reference-compiler.md` ¶3), so naming every intermediate adds no runtime cost
//! and the emitted bytes are unchanged for a program that has no multi-use runtime binding.
//!
//! A `Let` binding is keyed by its INITIALIZER's AST `StructId` — the stable identity a reference to
//! the binding already resolves to (`Resolved::Ref { value }`), so no fresh id space is needed for
//! this case: the slot a binding occupies and the `LocalRef`s that read it share that one occurrence.
//! (Admin bindings with NO source occurrence — the ones a general A-normalization of every operand
//! synthesizes — arrive with the core's own fresh-id space in a later stage; this rung names only the
//! binding a source `let` already gives an occurrence to.)

use crate::ast::{IntValue, StructId};
use crate::diag::Reject;
use crate::resolved::{Prim, Symbol};
use std::collections::BTreeMap;

/// One step of a match ACCESS PATH — how to reach a SUB-VALUE of the scrutinee at run time, for a
/// NESTED pattern. A match over `(Some (Some x))` dispatches on the outer discriminant, then on the
/// INNER one (`sum-disc(sum-payload(scrutinee))`), and binds `x` at `sum-payload(sum-payload(…))`. Each
/// nesting level is a `Payload` step (into a variant's payload); a tuple/record position is an `Elem`
/// step (into an array cell). A path is a `Vec<PathStep>` from the scrutinee root; the empty path is the
/// scrutinee itself. This is what lets the decision-tree matcher share a prefix (one outer `sum-disc`
/// switch) AND reach a binder at any depth (`type-system.md §Patterns Compose`).
/// One fixed-width INTEGER segment of a runtime [`Core::BinBuild`] — the lean core form of a `(uNN v)`/
/// `(iNN v)` segment: its byte `width` (1/2/4/8), `signed` (two's-complement `iNN`), `little_endian` (the
/// `le` modifier), and the `value` occurrence to encode (a runtime int emitted at select). BN4b/runtime
/// construction handles only integer segments so far; bit-fields + `(bytes …)` splices with a runtime
/// value are later slices, so a `Core::BinBuild` carries only int segments (a `bin` mixing them with a
/// runtime bytes/bits segment still declines at `lower`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BinSeg {
    pub width: u8,
    pub signed: bool,
    pub little_endian: bool,
    pub value: StructId,
}

/// One bit-field of a runtime [`Core::BinBitsBuild`] — a `(bits v k)` segment: pack the low `k` bits of
/// the runtime value `value` (an UNSIGNED `k`-bit field; a value outside `0..2^k` traps at run time, the
/// companion of the constant CDZ0304). `k` is a compile-time constant (read at resolve); the whole
/// bit-field RUN is byte-aligned (CDZ0220 checked it), so a `Core::BinBitsBuild` emits a whole number of
/// bytes. `k` ≤ 56 for a runtime field (keeps the MSB-first u64 pack accumulator from overflowing between
/// byte flushes — a wider runtime bit-field declines at `lower`; the constant path still handles it).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BinBitsField {
    pub k: u32,
    pub value: StructId,
}

/// Which binary set-algebra op a `Core::SetAlgebra` node performs — union / intersection / difference.
/// One `Core` variant serves all three (they share the `(Set a) -> (Set a) -> (Set a)` shape and a single
/// consuming-both-operands emit), the runtime op selected by this discriminant.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SetAlgebraOp {
    Union,
    Intersection,
    Difference,
}

/// Which runtime BigInt binary ARITHMETIC op a [`Core::BigIntBinOp`] performs — `+`/`-`/`*`/`/` mapped to
/// the runtime `bigint-add`/`-sub`/`-mul`/`-div`. (Comparison lowers through `bigint-cmp` + a fixed
/// compare in `lower`, so it is not one of these — this enum is arithmetic-only, producing a BigInt.)
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BigIntOp {
    Add,
    Sub,
    Mul,
    Div,
    /// `%` — remainder of truncating division (sign of the dividend). The runtime `bigint-rem` op.
    Rem,
}

/// One arm of a [`Core::MatchList`] — a LENGTH condition on the list scrutinee plus a body. The backend
/// tests the conditions in arm order against `vec-len(scrutinee)`; the first satisfied arm's body runs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ListArmCond {
    /// `vec-len == n` — a fixed-arity `(list p0 … p_{n-1})` pattern.
    LenEq(usize),
    /// `vec-len >= lead` — a rest pattern `(list p0 … p_{lead-1} .. rest)` (binds ≥ `lead` elements).
    LenGe(usize),
    /// Always matches — a bare binder / `_` (a whole-list catch-all). Equivalent to `LenGe(0)`.
    Any,
}

/// One arm of a runtime list match: its length [`ListArmCond`] and the body occurrence to emit when the
/// condition holds. Binders (leading elements, the rest sublist) resolve independently via `SumPayload`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ListArm {
    pub cond: ListArmCond,
    pub body: StructId,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PathStep {
    /// Descend into a sum variant's PAYLOAD — `sum-payload(handle)`. (A single-payload variant; a
    /// multi-payload variant's payload is a tuple, reached with a following `Elem`.)
    ///
    /// Over a NOMINAL NEWTYPE scrutinee (an erased single-variant sum), a `Payload` step is a RUNTIME
    /// NO-OP: the box is erased (`type-system.md §156`), so the value ALREADY IS its underlying value —
    /// the step only reinterprets the static type (`Ty::Nominal` → its `inner`). The path walkers detect
    /// this by the CURRENT type at the step (a `Ty::Nominal` sub-value ⇒ unwrap-no-op; a `Ty::Sum` ⇒ the
    /// real `sum-payload`), so resolve emits the SAME `Payload` step for a newtype and a boxed sum and
    /// the representation choice stays a read-off of the solved type.
    Payload,
    /// Descend into a tuple/record ARRAY cell at `index` — `arr-get(handle, index)`.
    Elem(usize),
    /// The TAIL SUBLIST of a `List` scrutinee starting at index `k` — the `rest` binder of a list REST
    /// pattern `(list p0 … p_{k-1} .. rest)`, which binds `(List a)` = the elements from `k` onward.
    /// Reads `vec-split(list, k).right` at run time (the left half, the matched leading `k` elements, is
    /// dropped); over a CONSTANT list it folds to the tail `Core::ListNew`. Only appears as the SOLE step
    /// of a rest-binder's path (a list scrutinee is flat — no nesting under a rest).
    RestFrom(usize),
}

/// A match-arm PROBE — the test that decides whether an arm is taken, over a SCALAR scrutinee. A
/// literal probe compares the scrutinee against a constant; the wildcard always matches (the arm is
/// the unconditional tail). A bare binder is ALSO a `Wild` probe — the binding is a scope concern
/// (`resolve` points a body reference at the scrutinee), so it needs no probe variant. Carried as DATA
/// (not a synthesized comparison node), so lowering a `match` builds no AST. A sum/tuple/record
/// scrutinee walks the value heap rather than probing a scalar, so it does not extend this enum.
#[derive(Clone, PartialEq, Debug)]
pub enum Probe {
    /// `scrutinee == this integer` — an integer-literal pattern.
    Int(IntValue),
    /// `scrutinee == this boolean` — a boolean-literal pattern.
    Bool(bool),
    /// `scrutinee == this string` — a string-literal pattern (`("hello" …)`). Only the CONSTANT-scrutinee
    /// FOLD is realized (a constant scrutinee selects the first arm whose string equals it); a RUNTIME
    /// string scrutinee is not a scalar (`is_scalar` is Int/Bool), so its match declines until the runtime
    /// string-equality probe is emitted (a later increment).
    Str(String),
    /// A LIST pattern's length test — the list at this path must have `len` elements (`at_least` false, a
    /// fixed-arity `(list p0 … p{len-1})`) or AT LEAST `len` (`at_least` true, a rest pattern `(list p0 …
    /// p{len-1} .. rest)` binding the tail). Like `Str`, only the CONSTANT-scrutinee FOLD is realized (a
    /// constant `Core::ListNew` of the right length passes, then each leading element is destructured at
    /// `Elem(i)` and any rest binder reads `RestFrom(len)`); a RUNTIME list payload is not a scalar, so its
    /// match declines until the runtime list matcher is wired into the payload path. Carried as DATA, gated
    /// once the enclosing discriminant constraints are satisfied, exactly like a literal test.
    ListLen { len: usize, at_least: bool },
    /// The wildcard `_` OR a bare binder — always matches.
    Wild,
}

/// One arm of a scalar [`Core::Match`]: a [`Probe`], an optional GUARD, and a body. The arm fires when
/// the scrutinee satisfies `probe` AND (if present) `guard` evaluates to true; otherwise matching falls
/// through to the next arm. `guard` is a boolean expression occurrence lowered on demand, evaluated with
/// the arm's binder in scope (a binder pattern binds the scrutinee, resolve Case 5). A guarded arm does
/// NOT count toward exhaustiveness — its guard may fail — so a guarded wildcard is not a covering tail.
#[derive(Clone, PartialEq, Debug)]
pub struct MatchArm {
    pub probe: Probe,
    /// The arm's guard condition (a boolean expression occurrence), or `None` for an unguarded arm.
    pub guard: Option<StructId>,
    pub body: StructId,
}

/// One arm of a sum SWITCH (`Core::MatchSum` or a nested [`SumCont::Switch`]): which variant it matches
/// (by discriminant) and what happens next. A `disc: Some(k)` arm matches when the switched-on
/// discriminant `== k`; a `disc: None` arm is the DEFAULT (wildcard) tail (always matches). What
/// follows a match is a [`SumCont`] — a leaf body, or a NESTED switch on a deeper sub-value (the
/// decision tree recursing to share the outer discriminant probe, `type-system.md §Patterns Compose`).
/// A payload binder is not carried here — it resolves to a [`Core::SumPayload`] on its own (a reference
/// in the body reads the payload at its path), so an arm needs only its discriminant + continuation.
#[derive(Clone, PartialEq, Debug)]
pub struct SumArm {
    /// The variant discriminant this arm matches, or `None` for the DEFAULT (wildcard) tail.
    pub disc: Option<u32>,
    /// What happens when this arm matches — a leaf body or a nested switch.
    pub cont: SumCont,
}

/// The CONTINUATION of a matched sum arm: either a LEAF (the arm's body occurrence, lowered on demand)
/// or a nested SWITCH the decision tree recurses into. A nested switch dispatches on the discriminant of
/// a DEEPER sub-value (reached by its own `path` from the root scrutinee), which is how `(Some (Some x))`
/// and `(Some None)` share the ONE outer `Some` probe and then split on the inner discriminant — the
/// Maranget decision-tree shape (only two tag checks on the `Some (Some …)` path, not a linear re-probe).
#[derive(Clone, PartialEq, Debug)]
pub enum SumCont {
    /// The matched arm's body occurrence (lowered on demand).
    Leaf(StructId),
    /// A GUARDED arm — the variant has already matched (its discriminant constraint satisfied), and the
    /// arm fires only if `cond` (a boolean the payload binder is in scope for) holds: `if cond then body
    /// else <els>`. On a false guard, control FALLS THROUGH to `els` — the continuation built from the
    /// REMAINING rows of the same sub-matrix (a later arm of the same variant, or the default), exactly
    /// as a scalar guarded probe threads its `else` to the next arm. A guarded arm does NOT count toward
    /// exhaustiveness, so `els` must independently cover the variant (checked when the sub-matrix is
    /// compiled). `body` and `cond` are lowered on demand.
    Guarded {
        cond: StructId,
        body: StructId,
        els: Box<SumCont>,
    },
    /// A LITERAL-PAYLOAD test — a variant pattern whose payload (or a deeper sub-value) is a LITERAL
    /// rather than a binder: `(Some 0)` matches `Some` carrying EXACTLY `0` (`core-semantics.md §Pattern
    /// Matching`: "nested patterns can combine constructors and literals … the literal refines the
    /// match"). The sub-value at `path` (from the ROOT scrutinee, `sum-payload`/`arr-get` steps) is read
    /// and compared against the literal `probe`; on a match, control proceeds to `then_`; on a MISMATCH it
    /// FALLS THROUGH to `els` — the continuation built from the REMAINING rows (a later arm of the same
    /// variant, typically the binding arm `(Some k)`), exactly as [`Guarded`] threads a false guard's
    /// `else`. A literal test does NOT count toward exhaustiveness (it may not match — it needs an
    /// unguarded/binder fall-through of the same variant), the same rule guards follow. Distinct from a
    /// discriminant [`Switch`] (which tests `sum-disc`); this tests a scalar VALUE at a payload leaf, so
    /// the payload's variant is already fixed by an enclosing switch. `then_`/`els` are continuations
    /// (so several literal tests on one arm nest, and the matched body is itself a `Leaf`/`LitTest`).
    LitTest {
        path: Vec<PathStep>,
        probe: Probe,
        then_: Box<SumCont>,
        els: Box<SumCont>,
    },
    /// A nested switch on the sub-value at `path` (from the ROOT scrutinee) — try each arm's disc, else
    /// the default arm. `path` is the full path from the scrutinee (not relative to the parent switch),
    /// so the backend walks it from the scrutinee handle uniformly at every depth.
    Switch {
        path: Vec<PathStep>,
        arms: Vec<SumArm>,
    },
}

/// The core (A-normal) form of one node.
#[derive(Clone, PartialEq, Debug)]
pub enum Core {
    /// An integer constant at exact arbitrary precision. The narrowing to the machine width its
    /// solved type fixes is the backend's job at selection.
    ConstInt(IntValue),
    /// An EXACT RATIONAL constant — a NORMALIZED pair of arbitrary-precision integers (numerator,
    /// denominator): lowest terms (gcd-reduced), the sign on the numerator, the denominator strictly
    /// positive (`> 0`, never zero — a zero denominator traps at construction, never reaching here). A
    /// `Ty::Rational` value folded in `lower` (`Rational.of`/`of-int` + exact `+`/`-`/`*`/`/`/compare over
    /// the pair, via `IntValue` bignum arithmetic — B4-1). Two equal rationals share ONE normalized pair,
    /// so `=` is structural over `(num, den)`. Crossing the boundary as a `{numerator, denominator}` record
    /// + a runtime rational compound are a later B4 slice (the escape/emit paths decline for now).
    ConstRational(IntValue, IntValue),
    /// A boolean constant.
    ConstBool(bool),
    /// A string constant — the canonical text of a string literal. A `Ty::String` value; escapes as its
    /// baked UTF-8 bytes (like a constant compound). Runtime string ops are a later stage.
    ConstStr(String),
    /// A CHAR constant — a single Unicode scalar value (`Ty::Char`). Constant equality/ordering compare
    /// by scalar value (`c as u32`). Crossing the boundary as a char value + `Char.to-int`/`from-int` are
    /// later increments (a char at the boundary still declines — no scalar machine path yet).
    ConstChar(char),
    /// A FLOATING-POINT constant — the EXACT `Decimal` of a float literal (no `f64` rounding until a
    /// width is chosen). A `Ty::Float` value. This increment folds float EQUALITY (two constants compared
    /// by their canonical Float64 value — `1e19` and `1e20` differ, `-0.0` and `0.0` differ); float
    /// ARITHMETIC and crossing the boundary as a float value are later increments (a float value at the
    /// boundary / an arithmetic operand still declines — no f64 machine path yet).
    ConstFloat(crate::ast::Decimal),
    /// The canonical NOT-A-NUMBER Float constant (`nan`) — a `Ty::Float` value distinct from any
    /// `ConstFloat` (`Decimal` holds only finite values, so NaN cannot be a `ConstFloat`). Under the
    /// canonical-byte-form equality every NaN is equal to every NaN and unequal to every finite float
    /// (`core-semantics.md` §Floating-Point Equality Follows The Canonical Byte Form). Its bit pattern is
    /// `f64::NAN.to_bits()` — the one canonical quiet NaN. Folds in `=`; does not yet cross the boundary
    /// (no written value form) — the escape/emit paths decline, like a runtime float.
    ConstFloatNan,
    /// The unit value.
    Unit,
    /// A record value — a fixed set of named fields, each field's value referenced by its AST
    /// occurrence. A field read FOLDS (`core_of` of a member projects the field's core directly), so a
    /// `Record` that SURVIVES to selection is one used as a runtime value (e.g. returned) — the backend
    /// builds it on the value heap (`arr-alloc` + per-field `box-*`/`arr-set`, fields in canonical
    /// order). Carrying the variant lets member-access fold read the field set. The field set is fixed
    /// and statically known (the `Symbol` keys), each field holding a value of its own type:
    //= spec/capabilities/core-semantics.md#a-record-has-a-fixed-set-of-named-fields
    //# A record MUST associate a fixed set of statically-known field names each with a value, where distinct fields may hold values of distinct types.
    ///
    /// The field map is behind an `Arc` so CLONING a `Core::Record` (which `core_of` does on every memo
    /// read, and every recursive Core-tree walk — `collect_host_arg_strings`, layout, select — does per
    /// node) is a refcount bump, not a deep O(fields) `BTreeMap` copy. A wide record read field-by-field
    /// (`(+ (. r f0) (+ (. r f1) …))`) re-reads the record's `Core` per access, so an owned map made that
    /// O(N²) — 3200 fields ≈ 2.8s, ~50% in `BTreeMap::clone`. Mirrors [`crate::ty::Ty::Record`], which is
    /// `Arc`-wrapped for the identical reason.
    Record {
        fields: std::sync::Arc<BTreeMap<Symbol, StructId>>,
    },
    /// A TUPLE value — a fixed-arity positional product, each element referenced by its AST occurrence.
    /// Present only when the tuple SURVIVES to selection as a RUNTIME value (constructed from runtime
    /// operands, or a constant tuple that escapes — a projection of a compile-time-visible tuple folds to
    /// the element in `lower`, leaving no `Tuple`). The backend builds it on the value heap
    /// (`arr-alloc` + per-element `box-*`/`arr-set`), or — for a proven-CONSTANT tuple — builds it ONCE
    /// (the static build-once path, §2d). Elements are lowered on demand. Fixed-size and positional (the
    /// `elems` vector's length is the arity), and its elements may be of distinct types:
    //= spec/capabilities/core-semantics.md#a-tuple-is-a-fixed-size-positional-product
    //# A tuple MUST be a fixed-size value whose elements are accessed positionally.
    //= spec/capabilities/core-semantics.md#a-tuple-is-a-fixed-size-positional-product
    //# A tuple MAY hold elements of distinct types.
    Tuple { elems: Vec<StructId> },
    /// A tuple PROJECTION — read element `index` of the tuple the `operand` occurrence denotes. Present
    /// only when the operand is a RUNTIME tuple (a projection of a compile-time-visible tuple folds to
    /// the element directly in `lower`, so it never reaches here). The backend emits `arr-get` +
    /// `get-*`. The `index` is within the operand's static arity (checked in `type_errors` before
    /// selection — an out-of-arity index is a compile-time reject, never a runtime trap).
    Proj { operand: StructId, index: usize },
    /// A LIST value construction — `(list 1 2 3)`. Present when it survives to selection as a RUNTIME
    /// value (constructed from runtime operands, or a constant list that escapes). The backend builds it
    /// on the persistent `vec-*` heap: `vec-empty` then a `vec-push` per element (each boxed by the
    /// element type). Elements are lowered on demand. Homogeneous — one element type.
    ListNew { elems: Vec<StructId> },
    /// `List.len` of the list the `operand` occurrence denotes — the runtime `vec-len` op, an `Int64`.
    /// Present when the operand is a RUNTIME list (a constant list's length folds to a `ConstInt` in
    /// `lower`, so it never reaches here).
    ListLen { operand: StructId },
    /// `List.push` — append `elem` to `list`, returning the new list (runtime `vec-push`; persistent, no
    /// mutation). `elem` is boxed by its type before the push, exactly as a list element is at construction.
    ListPush { list: StructId, elem: StructId },
    /// `List.concat` — concatenate `lhs` and `rhs` into one list (runtime `vec-concat`). Both are list
    /// handles of the same element type.
    ListConcat { lhs: StructId, rhs: StructId },
    /// `List.update` — replace the element at `index` of `list` with `elem`, returning the new list
    /// (runtime `vec-update`; persistent, no mutation; an out-of-bounds `index` traps). `index` is an
    /// `Int64` occurrence wrapped to the `u32` the op takes; `elem` is boxed by its type before the
    /// update, exactly as a list element is at construction/push.
    ListUpdate {
        list: StructId,
        index: StructId,
        elem: StructId,
    },
    /// `List.at` — the FALLIBLE indexed read, present when the list is a RUNTIME value (a constant list +
    /// constant index FOLDS to a `SumNew` in `lower`, so it never reaches here). The backend emits a
    /// bounds-checked runtime form: evaluate the list handle ONCE (a scratch local), read its `vec-len`,
    /// and if `0 <= index < len` build `Some(<boxed vec-get element, dup'd — vec-get BORROWS but the
    /// `Some` payload is CONSUMED>)`, else `None`. `disc_some`/`disc_none` are the built-in Option
    /// variants' discriminants (read at lowering off the result type's declaration, not baked by name);
    /// `elem` is the solved element type, choosing the box/unbox ops. The runtime companion of the
    /// fold — one code path per index-in-range test, yielding a heap `Option` handle.
    ListAt {
        list: StructId,
        index: StructId,
        disc_some: u32,
        disc_none: u32,
    },
    /// A BYTES value construction — `(Bytes.of (list …))`. Present when it survives to selection as a
    /// RUNTIME value (built from a runtime list, or a constant that escapes — a constant `Bytes.of`
    /// whose bytes are all known folds to baked bytes in `lower`). `elems` are the list-element
    /// occurrences (each an Int64 in `0..=255`); the backend builds the sequence on the persistent
    /// rope `bytes-*` heap: `bytes-alloc(len)` then a range-checked `bytes-set` per element (an element
    /// `< 0` or `> 255` traps at run time, matching the fold's compile-time CDZ0304). Only the
    /// literal-length form is built here; a runtime-length list source is a later increment.
    BytesOf { elems: Vec<StructId> },
    /// `Bytes.len` of the bytes the `operand` denotes — the runtime `bytes-len` op, an `Int64`. Present
    /// when the operand is a RUNTIME bytes value (a compile-time-visible `Bytes.of` folds its length to
    /// a `ConstInt` in `lower`, so it never reaches here). The bytes companion of `ListLen`.
    BytesLen { operand: StructId },
    /// `Bytes.at` — the FALLIBLE indexed byte read, present when the bytes operand is a RUNTIME value (a
    /// constant `Bytes.of` + constant index FOLDS to a `SumNew` in `lower`, so it never reaches here). The
    /// backend emits a bounds-checked runtime form: read `bytes-len`, and if `0 <= index < len` build
    /// `Some(box-int(bytes-get(bytes, index)))` — a byte is a raw i32 VALUE (`bytes-get` returns it), so
    /// unlike `ListAt` there is no borrowed-handle `dup`; the value is zero-extended and boxed into the
    /// `Some` payload — else `None`. `disc_some`/`disc_none` are the built-in Option variants' discs. The
    /// byte companion of `ListAt`; the result element is always `Int64`.
    BytesAt {
        bytes: StructId,
        index: StructId,
        disc_some: u32,
        disc_none: u32,
    },
    /// `String.at` / `String.scalar-at` on a RUNTIME string — read the i-th UNICODE SCALAR of a
    /// UTF-8-backed String, fallibly (`String → Int64 → (Option String|Char)`). A constant string + index
    /// FOLDS in `lower` (`chars().nth`), so this reaches the backend only for a runtime string. A String
    /// is a flat UTF-8 byte leaf, so the backend WALKS the byte buffer scalar-by-scalar (a byte is a
    /// scalar START iff `(byte & 0xC0) != 0x80` — a non-continuation byte): skip `index` scalars, then the
    /// scalar's byte span is `[pos, pos+scalar_len)` where `scalar_len` counts the lead byte + its
    /// continuation bytes. In bounds → `Some(bytes-slice(str, pos, scalar_len))` (the one-scalar String is
    /// the byte-slice — `char` payload is the same byte-slice for `scalar-at`, distinguished at the source
    /// type). A negative index or one at/beyond the scalar count → `None`. The scalar companion of
    /// `BytesAt` (which reads a raw byte); here the payload is a multi-byte string span, indexed by SCALAR.
    StrAt {
        string: StructId,
        index: StructId,
        disc_some: u32,
        disc_none: u32,
    },
    /// `Bytes.concat` — append `lhs` and `rhs` into one byte sequence (runtime `bytes-concat`; consumes
    /// both, empty is the identity). Present when the pair is not both compile-time-visible constants (a
    /// constant pair folds to a `Core::BytesOf` in `lower`). The byte companion of `Core::ListConcat`.
    BytesConcat { lhs: StructId, rhs: StructId },
    /// `BigInt.of x` on a RUNTIME fixed-width integer — widen `x` (an i64-slot value) into a `BigInt`
    /// heap leaf via the runtime `bigint-of-i64` op. A CONSTANT source folds to `Core::ConstInt` retyped
    /// `BigInt` in `lower` (B1) and never reaches here; this is the runtime path (B3b).
    BigIntOfI64 { value: StructId },
    /// `Int64.of b` / `(UInt N).of b` on a RUNTIME `BigInt` `b` — the checked narrowing back to a
    /// fixed width via `bigint-to-i64-checked` (traps out of range at run time). The `width`/`signed` of
    /// the target refine the range the runtime op checks against once narrower-than-i64 checking lands;
    /// today the runtime checks the i64 range and a narrower target's over-range constant is already
    /// rejected at compile time (B1). A constant `BigInt` source folds in `lower`.
    BigIntToI64 { operand: StructId },
    /// A runtime BigInt BINARY op — `+`/`-`/`*`/`/` (the runtime `bigint-add`/`-sub`/`-mul`/`-div`) or a
    /// comparison lowered through `bigint-cmp`. Present when at least one operand is a runtime `BigInt`
    /// (a constant pair folds via `num-bigint` in `lower`). Produces a `BigInt` handle for arithmetic;
    /// the comparison forms wrap `bigint-cmp` + a fixed compare (built in `lower`, so this arm is
    /// arithmetic-only). `div` traps on a zero divisor at run time.
    BigIntBinOp {
        op: BigIntOp,
        lhs: StructId,
        rhs: StructId,
    },
    /// A runtime BigInt COMPARISON — `<`/`>`/`<=`/`>=`/`=` (and `≠`, as `not =`) over `BigInt` operands,
    /// lowered through the runtime `bigint-cmp` op (a three-way `-1`/`0`/`1` for `a < b`/`a = b`/`a > b`)
    /// then the operator's fixed i64 compare-with-zero: `<` is `cmp <ₛ 0`, `>` is `cmp >ₛ 0`, `<=` is
    /// `cmp <=ₛ 0`, `>=` is `cmp >=ₛ 0`, `=` is `cmp == 0` (`eqz`). Result is a `Bool` (i32 0/1). Present
    /// when at least one operand is a runtime `BigInt` (a constant pair folds via `num-bigint` in `lower`);
    /// like the arithmetic ops, `bigint-cmp` BORROWS both operands (the emit drops each owned temporary).
    BigIntCmp {
        op: Prim,
        lhs: StructId,
        rhs: StructId,
    },
    /// A `(bin <seg>…)` CONSTRUCTION with at least one RUNTIME segment value (an all-constant `(bin …)`
    /// folds to a `Core::BytesOf` in `lower`). Builds a `Bytes` on the rope heap at run time: each
    /// fixed-width integer segment range-checks its value against the segment (trap "binary value does not
    /// fit segment" if out of range — the runtime companion of the constant CDZ0304) and writes its `w`
    /// bytes big-endian (`le` reversed). The segments are the LEAN [`BinSeg`] form (width/signedness/
    /// endianness + the value occurrence), so `Core` does not depend on the resolver's `Segment`.
    BinBuild { segs: Vec<BinSeg> },
    /// A RUN of `(bits v k)` bit-field segments with at least one RUNTIME value, packed MSB-first into a
    /// `Bytes` on the rope heap at run time. The run is byte-aligned (CDZ0220 guaranteed `sum(k) % 8 == 0`),
    /// so it produces `sum(k) / 8` bytes. Each field's value range-checks against its `k`-bit width (trap
    /// "binary value does not fit segment" if `< 0` or `>= 2^k` — the companion of the constant CDZ0304),
    /// then its low `k` bits shift into a u64 accumulator that flushes whole bytes from the top as they
    /// close. Emitted by `lower` when a `bits`-only maximal run has a runtime value; a run mixing bits with
    /// int/bytes segments concatenates the pieces (`Core::BytesConcat`), like the int-run splitter.
    BinBitsBuild { fields: Vec<BinBitsField> },
    /// Read a fixed-width INTEGER segment out of a runtime `Bytes` scrutinee at a STATIC byte offset — the
    /// value a `bin`-pattern binder decodes when matching a runtime scrutinee (`(match b ((bin (u16 n)) n)
    /// …)`). Emits `w` `bytes-get`s from `bytes` at `byte_offset..+width`, assembles them (big-endian, `le`
    /// reversed), and sign- or zero-extends to an `Int64` per `signed`. The caller (`lower_match_bin`) has
    /// already guarded that the scrutinee is long enough (the arm's length probe), so this read is in
    /// bounds. `byte_offset` is static — fixed-offset segments only (a dependent-size `(bytes b n)` before
    /// this segment would make the offset dynamic, which the runtime matcher does not build yet).
    BinIntRead {
        bytes: StructId,
        byte_offset: u32,
        width: u8,
        signed: bool,
        little_endian: bool,
    },
    /// Read the FINAL `(bytes rest)` segment out of a runtime `Bytes` scrutinee — the remainder from a
    /// static `byte_offset` to the end. Emits `bytes-slice(bytes, byte_offset, bytes-len - byte_offset)`
    /// (the tail after the fixed prefix). The caller (`lower_match_bin`) guarded `bytes-len >= byte_offset`
    /// (the arm's length probe), so the slice is in bounds. `byte_offset` is static (a final rest after
    /// fixed-width int segments; a dependent-size `(bytes b n)` with a dynamic offset is a later slice).
    BinRestRead { bytes: StructId, byte_offset: u32 },
    /// `Bytes.slice` — the FALLIBLE sub-range read, present when the operand is a RUNTIME value (a
    /// constant `Bytes.of` + constant `start`/`len` FOLDS to a `SumNew` in `lower`). The backend
    /// bounds-checks (`start >= 0 && len >= 0 && start + len <= bytes-len`), and in range builds
    /// `Some(bytes-slice(bytes, start, len))` — the slice is a Bytes HANDLE, used as the `Some` payload
    /// directly (no box) — else `None`. `disc_some`/`disc_none` are the built-in Option variants' discs.
    BytesSlice {
        bytes: StructId,
        start: StructId,
        len: StructId,
        disc_some: u32,
        disc_none: u32,
    },
    /// `Bytes.compact` — a content-equal byte sequence with independent storage (runtime `bytes-compact`;
    /// consumes its operand). Present when the operand is a RUNTIME value (a constant folds to itself).
    BytesCompact { operand: StructId },
    /// A MAP value construction — `(map (k v) …)` or `Map.empty`. `entries` are `(key, value)` occurrence
    /// pairs, IN SOURCE ORDER (a later duplicate key overwrites an earlier one, keys compared by value).
    /// Present when it survives to selection as a RUNTIME value. The backend builds it on the persistent
    /// CHAMP `map-*` heap: `map-empty` then a `map-insert(key, value)` per entry (each key/value boxed by
    /// its type before the insert, which CONSUMES the map handle + the key + the value). `key_ty`/`val_ty`
    /// are the solved key/value types (choosing the box/unbox ops). An empty map has no entries.
    MapNew {
        entries: Vec<(StructId, StructId)>,
        key_ty: crate::ty::Ty,
        val_ty: crate::ty::Ty,
    },
    /// `Map.insert` — add-or-replace `key ↦ val` in `map`, returning the new map (runtime `map-insert`;
    /// persistent, CONSUMES the map handle). `key`/`val` are boxed by their types before the insert, as a
    /// map entry is at construction. A present key replaces its value (keys compared by value).
    MapInsert {
        map: StructId,
        key: StructId,
        val: StructId,
        key_ty: crate::ty::Ty,
        val_ty: crate::ty::Ty,
    },
    /// `Map.lookup` — the FALLIBLE keyed read, present when the map is a RUNTIME value. The backend emits
    /// `map-lookup(map, key)` (BORROWS both; the boxed key is dropped after) — a NULL handle for an absent
    /// key — and wraps it: a non-null handle → `Some(<unbox value>)`, null → `None`. `disc_some`/`disc_none`
    /// are the built-in Option variants' discriminants (read at lowering off the result type). `val_ty`
    /// chooses the value unbox. The map companion of `ListAt` (a NULL-or-handle test instead of bounds).
    MapLookup {
        map: StructId,
        key: StructId,
        key_ty: crate::ty::Ty,
        val_ty: crate::ty::Ty,
        disc_some: u32,
        disc_none: u32,
    },
    /// `Map.remove` — drop `key`'s association from `map`, returning the new map (runtime `map-remove`;
    /// persistent, CONSUMES the map handle, BORROWS the key). Removing an absent key yields a map equal to
    /// the operand (total). `key` is boxed by its type before the remove.
    MapRemove {
        map: StructId,
        key: StructId,
        key_ty: crate::ty::Ty,
    },
    /// `Map.size` — the count of distinct keys the map associates, present when the map is a RUNTIME value.
    /// The backend emits `map-size(map)` (BORROWS; O(1) from the CHAMP root) + an i32→i64 extend to `Int64`.
    /// The map companion of `ListLen`.
    MapSize { map: StructId },
    /// A SET value construction — `(Set.of (list …))`. `elems` are the element occurrences (in source
    /// order; DUPLICATES collapse at build). The backend builds it on the persistent CHAMP `set-*` heap:
    /// `set-empty` then a `set-insert(elem)` per element (each boxed by `elem_ty`, which `set-insert`
    /// CONSUMES along with the set). `elem_ty` is the solved element type (choosing the box op). An empty
    /// `(Set.of (list))` has no elements. The set analogue of `MapNew`, one axis.
    SetOf {
        elems: Vec<StructId>,
        elem_ty: crate::ty::Ty,
    },
    /// `Set.contains` — the TOTAL membership predicate, present when the set is a RUNTIME value. The backend
    /// emits `set-contains(set, elem)` (BORROWS both; the boxed element is dropped after) — a `bool`
    /// directly (UNLIKE `Map.lookup`'s NULL-or-handle → Option). `elem_ty` chooses the element box.
    SetContains {
        set: StructId,
        elem: StructId,
        elem_ty: crate::ty::Ty,
    },
    /// `Set.insert` — add `elem` to `set`, returning the new set (runtime `set-insert`; persistent, CONSUMES
    /// the set handle + the boxed element). Inserting a present element is a no-op value. The set analogue
    /// of `MapInsert` (no value column).
    SetInsert {
        set: StructId,
        elem: StructId,
        elem_ty: crate::ty::Ty,
    },
    /// `Set.remove` — drop `elem` from `set`, returning the new set (runtime `set-remove`; CONSUMES the set,
    /// BORROWS the boxed element). Removing an absent element yields an equal set (total).
    SetRemove {
        set: StructId,
        elem: StructId,
        elem_ty: crate::ty::Ty,
    },
    /// `Set.len` — the count of distinct elements, present when the set is a RUNTIME value. The backend
    /// emits `set-size(set)` (BORROWS; O(1)) + an i32→i64 extend to `Int64`. The set analogue of `MapSize`.
    SetLen { set: StructId },
    /// `Set.union`/`Set.intersection`/`Set.difference` — the binary set-algebra ops, present when an
    /// operand is a RUNTIME value. The backend emits the runtime `set-union`/`set-intersection`/
    /// `set-difference` op (each CONSUMES both operands, returns the result set). `op` names which.
    SetAlgebra {
        op: SetAlgebraOp,
        lhs: StructId,
        rhs: StructId,
    },
    /// A SUM VALUE CONSTRUCTION — `(Option.Some 5)` or a bare nullary `None`. `disc` is the variant's
    /// discriminant (read off the ctor's `(meta variant)` at lowering); `payloads` are the argument
    /// occurrences (empty for a nullary variant). The backend builds `sum-new(disc, payload)` where the
    /// payload handle is: an empty array `arr-alloc(0)` for a nullary variant (`value-heap-runtime.md`
    /// §Sum: "a nullary variant carries the unit value — an arr of length 0"), the single boxed payload
    /// for a one-payload variant, or a tuple handle built from the payloads for a multi-payload variant.
    /// The nominal tag is compile-time only — the runtime holds only `(disc, payload)`. This is what a
    /// constructor application produces: applying `Some`/`None`/… yields a Sum value tagged with the
    /// variant's discriminant. The emitter carries NO source-level name into the value heap: a sum is a
    /// discriminated payload (`disc` + boxed value) and a record is a positional `arr-alloc` product (its
    /// field NAMES are the compile-time `Symbol` keys, never stored), so the runtime holds only structure
    /// and data and the position→name association is compile-time knowledge. The `disc` a sum carries is
    /// the runtime datum recording WHICH variant a value is, never the sum's TYPE — the compiler knows the
    /// static type at every use site (no type erasure) and maps a discriminant to a variant name itself:
    //= spec/contracts/component-abi.md#the-runtime-does-not-name-or-render-values
    //# The value-heap runtime MUST NOT hold the field names of a record, the variant names of a sum, or any other source-level name of a value, so that a record is a positional product and a sum is a discriminated payload at run time and the association of a position with a name is compile-time knowledge the runtime does not carry.
    //= spec/contracts/component-abi.md#the-runtime-does-not-name-or-render-values
    //# The value-heap runtime MUST NOT hold a value's TYPE as a per-value tag, so that — because the language has no type erasure and the compiler therefore knows a value's static type at every use site — the runtime stores only structure and data (a product's elements, a sum's variant discriminant, a leaf's payload) and never a type identity a reader would dispatch on. The variant discriminant a sum carries is the runtime datum recording WHICH variant a value is, not the sum's type; the compiler maps a discriminant to a variant name.
    //= spec/capabilities/core-semantics.md#a-sum-type-constructor-is-a-single-arity-function-producing-the-tagged-variant
    //# A sum type constructor MUST be represented as a single-arity function that, when applied to exactly one argument, produces a Sum value tagged with the constructor's variant name.
    //= spec/capabilities/type-system.md#sum-types-are-constructed-and-deconstructed
    //# A value of a sum type MUST be constructed through one of its variants.
    ///
    /// A NULLARY variant is a constructor whose ARGUMENT is the unit value — `None` builds `sum-new(disc,
    /// arr-alloc(0))`, the empty array standing for the unit payload — NOT a value pre-constructed at the
    /// prelude; `(None unit)` is the application that produces it, uniform with `(Some 5)`. Every sum value
    /// is built this way — by APPLICATION of its constructor, in all cases.
    //= spec/capabilities/core-semantics.md#a-sum-type-constructor-is-a-single-arity-function-producing-the-tagged-variant
    //# A "nullary" variant MUST be a constructor whose argument type is Unit, not a pre-constructed Sum value.
    //= spec/capabilities/core-semantics.md#a-sum-type-constructor-is-a-single-arity-function-producing-the-tagged-variant
    //# Construction MUST be via application in all cases: `(Some 5)`, `(None unit)`, `(Sign.Zero unit)`.
    SumNew { disc: u32, payloads: Vec<StructId> },
    /// A MATCH over a SUM scrutinee, compiled to a DECISION TREE. The ROOT switch dispatches on
    /// `sum-disc(scrutinee)` (`path` is empty — the scrutinee itself); each arm's continuation is a leaf
    /// body or a NESTED switch on a deeper sub-value ([`SumCont`]). This is what lets `(Some (Some x))`,
    /// `(Some None)`, and `None` share the ONE outer `Some` probe and then split on the inner
    /// discriminant — the Maranget shape (two tag checks on the deep path, not a linear re-probe per arm).
    /// Present only when the scrutinee is a RUNTIME sum (a constant sum folds to the selected arm's core
    /// in `lower`, like a scalar match). The backend walks each switch's `path` from the scrutinee handle
    /// (`sum-payload`/`arr-get`) then probes `sum-disc` against each arm's discriminant; a `disc: None`
    /// arm is the unconditional default tail. Distinct from `Match` (a scalar scrutinee, equality probes)
    /// because a sum walks the heap handle — the discriminant, not a scalar value, drives the dispatch. A
    /// payload binder in an arm body is NOT carried here: it resolves to a `SumPayload` independently (a
    /// reference reads `sum-payload` at the binder's path), so an arm needs only its discriminant + cont.
    /// A sum value is deconstructed ONLY through this match form, which the exhaustiveness rule governs
    /// (a match not covering the scrutinee's variant set is a CDZ0210 compile-time rejection):
    //= spec/capabilities/type-system.md#sum-types-are-constructed-and-deconstructed
    //# A value of a sum type MUST be deconstructed only through a match that the exhaustiveness rule governs.
    MatchSum {
        scrutinee: StructId,
        /// The ROOT continuation of the decision tree. Normally a [`SumCont::Switch`] on the scrutinee's
        /// own discriminant (path empty); but a disc-fold (a statically-known scrutinee discriminant)
        /// can collapse the root switch to the selected arm's continuation — a nested [`SumCont::Switch`],
        /// or a [`SumCont::Guarded`] (a guarded arm of the selected variant). A root that is a bare
        /// `Leaf` folds to its body in `lower` and never reaches here.
        root: Box<SumCont>,
    },
    /// A match over a RUNTIME `List` scrutinee, dispatched by LENGTH (a constant list folds the selected
    /// arm in `lower`). Each arm is a [`ListArm`] — a length CONDITION (exact `n` for a fixed-arity
    /// `(list …)`, or ≥ `lead` for a rest pattern `(list p… .. rest)`, or always for a bare binder/`_`) and
    /// a body. The backend reads `vec-len(scrutinee)` ONCE and tests each arm's condition in order (first
    /// match wins), emitting the first arm's body whose condition holds. A leading element binder resolves
    /// to `SumPayload{Elem(i)}` (`vec-get`) and the rest binder to `SumPayload{RestFrom(k)}` (`vec-split`)
    /// on their own, so an arm carries only its length condition + body. Exhaustiveness (every length ≥ 0
    /// covered) is checked in `lower` (CDZ0210 otherwise), so the last arm's condition always holds at run
    /// time (there is a catch-all tail) — the emit needs no fallthrough trap.
    MatchList {
        scrutinee: StructId,
        arms: Vec<ListArm>,
    },
    /// The SUB-VALUE of a sum scrutinee at an access PATH, extracted by a variant pattern's binder.
    /// `(match s ((Some x) x))` → `x` reads `sum-payload(scrutinee)` (path `[Payload]`); `(match s
    /// ((Some (Some y)) y))` → `y` reads `sum-payload(sum-payload(scrutinee))` (path `[Payload,
    /// Payload]`). The backend walks each step (`sum-payload`/`arr-get`) then unboxes by the leaf's
    /// solved type (`get-int`/`get-bool`, or the handle as-is for a compound). The path's discriminants
    /// are not needed at run time (control is already in the matched arm).
    SumPayload {
        scrutinee: StructId,
        path: Vec<PathStep>,
    },
    /// `Option.expect` / `Result.expect` on a RUNTIME sum — unwrap the PRESENT variant's payload or TRAP
    /// on absence. The present variant is discriminant `disc_present` (Some/Ok = 0); the backend probes
    /// `sum-disc(scrutinee) == disc_present`, and on a match reads `sum-payload` + unboxes by this node's
    /// solved type (the payload type), else emits `unreachable` (an unconditional trap). The `"message"`
    /// operand is DROPPED — the wasm trap carries no text (the corpus `(trap MSG)` grades on the trap, not
    /// its message; so the "trap carries the message as its reason" obligation is NOT yet realized —
    /// UNCITED). Present only when the scrutinee is a runtime sum; a constant present variant FOLDS to its
    /// payload in `lower` (a constant absent variant is a provable trap — not yet folded, declines).
    //= spec/capabilities/core-semantics.md#requiring-the-value-of-an-optional-traps-on-absence
    //# An optional MUST offer an operation that returns its contained value when one is present and raises a trap when it is absent, so that turning absence into a halt is one explicit operation rather than a behavior wired into each operation that produces an optional.
    SumExpect {
        scrutinee: StructId,
        disc_present: u32,
    },
    /// `trap` — an UNCONDITIONAL divergence (`Prim::Trap`, the diverging primitive `∀a. String → a`). The
    /// backend emits `unreachable` (wasm) / `unreachable!()` (rust): the program HALTS here, producing no
    /// value (core-semantics.md §A Trap Occurs Only Where Its Computation Is Observed). `unreachable`
    /// leaves the stack polymorphic, so a `Core::Trap` validates in ANY result position (the runtime
    /// counterpart of its `Never` type — the else-branch of `SumExpect` emits the same instruction). The
    /// `String` message argument is DROPPED (the wasm trap carries no text); the node carries nothing.
    ///
    /// The `unreachable` halts the program at THIS point rather than continuing with an unspecified value:
    //= spec/capabilities/core-semantics.md#a-trap-halts-execution-at-a-defined-point
    //# A trap MUST halt the program at a defined point rather than continue with an unspecified value.
    Trap,
    /// A two-way conditional over atoms; structured control retained. Children are AST `StructId`s. The
    /// backend emits a wasm `if`/`else`, so ONLY the branch the condition selects executes:
    //= spec/capabilities/core-semantics.md#conditionals-evaluate-one-branch
    //# A conditional MUST evaluate only the branch its condition selects.
    If {
        cond: StructId,
        then_: StructId,
        else_: StructId,
    },
    /// A SHORT-CIRCUITING boolean conjunction `(and lhs rhs)` / disjunction `(or lhs rhs)` — present only
    /// when it did not fold to a constant in `lower`. `is_and` picks the semantics: `and` emits `if lhs
    /// then rhs else false`, `or` emits `if lhs then true else rhs` — so the RIGHT operand is evaluated
    /// only on the non-short-circuiting branch, shielding a trapping/effectful `rhs` exactly as a
    /// conditional's unselected branch does. The backend emits it as that `if` over the operands' i32
    /// boolean values.
    //= spec/capabilities/core-semantics.md#boolean-connectives-short-circuit
    //# A logical conjunction MUST evaluate its right operand only when its left operand is true, and a logical disjunction MUST evaluate its right operand only when its left operand is false, so that a connective shields a trapping or effectful right operand exactly as the unselected branch of a conditional does.
    And {
        lhs: StructId,
        rhs: StructId,
        is_and: bool,
    },
    /// A scalar MATCH over `scrutinee` — arms tried top-to-bottom, each a [`MatchArm`] (a probe, an
    /// optional GUARD, and a body). A `Probe` is either a literal to compare the scrutinee against
    /// (`== literal`) or the wildcard (always matches); a `guard` is a boolean expression evaluated with
    /// the arm's binder in scope that must ALSO hold for the arm to fire. Present only when the scrutinee
    /// is a RUNTIME scalar (a constant scrutinee folds to the selected arm's core in `lower`). The backend
    /// emits a chain of `if`s: probe the scrutinee against each literal (AND its guard, if any), take that
    /// arm's body on a match, else fall through to the next; an unguarded wildcard arm is the
    /// unconditional tail. `scrutinee`, each body, and each guard are AST `StructId`s (lowered on demand);
    /// the probe carries the literal as data so no comparison node is synthesized. A binder arm is a
    /// `Wild` probe (see [`Probe`]); a sum/tuple/record scrutinee walks the value heap rather than here.
    Match {
        scrutinee: StructId,
        arms: Vec<MatchArm>,
    },
    /// An A-normal binding sequence: name each `(binder, value)` — its VALUE computed once — then the
    /// `body` uses each name by a [`Core::LocalRef`]. The `binder` is the initializer's AST `StructId`
    /// (the identity a reference to the binding resolves to), so the slot a binding occupies and the
    /// refs that read it agree without a fresh id space. Present ONLY for a source `let` binding whose
    /// value is a runtime computation used more than once — the case where naming avoids recomputing;
    /// a single-use or constant binding is copy-propagated / erased at lowering, leaving no `Let`.
    /// Bindings are sequential (a later one may reference an earlier's name), matching `let*`.
    Let {
        bindings: Vec<(StructId, StructId)>,
        body: StructId,
    },
    /// A reference to a [`Core::Let`] binding — read the value named by `binder` (the initializer
    /// occurrence the binding was keyed by). The backend maps it to a `local.get` of the binding's
    /// slot, exactly as a `Param` reads a parameter slot. Present only when the referenced binding was
    /// KEPT as a `Core::Let` (a multi-use runtime value); a reference to a propagated binding lowers to
    /// the value's own core instead.
    LocalRef { binder: StructId },
    /// A runtime arithmetic operation on two operands (children by AST `StructId`). Present only when
    /// the fold could NOT reduce the operation to a constant (an operand is not compile-time-known — a
    /// FUNCTION PARAMETER). Constant arithmetic folds to `ConstInt`/`Poison` in `lower`. The machine op
    /// the backend emits is selected from the operands' solved width.
    Arith {
        op: Prim,
        lhs: StructId,
        rhs: StructId,
    },
    /// A runtime comparison on two operands (children by AST `StructId`) — result is a `Bool` (an i32
    /// at the machine level). Present only when the fold could not decide it (a runtime operand); two
    /// constants fold to `ConstBool` in `lower`. The machine op is selected from the operands' width
    /// and signedness.
    Compare {
        op: Prim,
        lhs: StructId,
        rhs: StructId,
    },
    /// A runtime STRUCTURAL EQUALITY on two COMPOUND operands (a sum/tuple/record/list heap value) —
    /// result is a `Bool`. Present only for `=` when the fold could not decide it because at least one
    /// operand is a runtime compound (two constant compounds fold to `ConstBool` via `const_compound_eq`
    /// in `lower`, and two runtime SCALARS take `Compare`). The backend emits a `value-eq` runtime call
    /// (the same tagless `champ_eq` walk the map/set key path runs): equal iff same shape + component-wise
    /// equal, variant discriminant before payload (core-semantics.md §Equality Is Structural). `value-eq`
    /// BORROWS both operands, so an owned-temporary operand is dropped after the compare.
    ValueEq { lhs: StructId, rhs: StructId },
    /// A runtime integer CONVERSION on one operand (child by AST `StructId`) — present only when the
    /// fold could not reduce it (the operand is a runtime value). `Prim::Wrap` truncates the operand to
    /// the node's solved TARGET width/signedness (read off at selection); a constant operand folds to a
    /// `ConstInt` in `lower`. The target is the conversion node's OWN solved type, not the operand's.
    Convert { op: Prim, operand: StructId },
    /// A runtime boolean NEGATION `!operand` — the `operand` is a `Bool` (an i32 0/1). Present only from
    /// the `(if c false true)` fold in `lower` (a boolean-coercion negation); a constant operand folds to
    /// the negated `ConstBool` there instead. The backend emits `<operand> ; i32.eqz`.
    Not { operand: StructId },
    /// A runtime CALL to a top-level function — `callee` is the `db.defs` index of the function, `args`
    /// the call-site argument occurrences (lowered in the CALLER's frame, pushed in order). Present only
    /// when the application could NOT β-reduce to a normal form at compile time — i.e. a RECURSIVE
    /// callee (a non-recursive call still inlines, so it never becomes a `Call`; this is the one path
    /// that forces a real wasm call). The callee is emitted as its own wasm function (reachability adds
    /// it to the layout's emission order); the backend emits each arg then `call <callee's abs index>`.
    ///
    /// Recursion is realized HERE as a STATIC reference to code — the callee's definition index resolved
    /// to a wasm function index — never as a heap value that points back at a value created earlier. So a
    /// recursive definition introduces no cycle into the value heap, and the compiler emits no construct
    /// that would form one, which is what lets the reference-count reclamation leave no value uncollected.
    //= spec/capabilities/memory-and-resource-model.md#the-value-heap-is-acyclic
    //# A recursive definition MUST refer to itself through a static reference to code rather than through a value that points back into the heap, so that recursion introduces no cycle into the value heap.
    //= spec/capabilities/memory-and-resource-model.md#the-value-heap-is-acyclic
    //# The compiler MUST NOT emit a construct that forms a cycle among heap values, so that a reference-count reclamation discipline leaves no value uncollected.
    Call { callee: usize, args: Vec<StructId> },
    /// A reference to a FUNCTION PARAMETER — the `binder` is the parameter's name occurrence (its
    /// identity, matching what `resolve` binds a reference to). The backend maps it to a `local.get` of
    /// the parameter's slot. This is the runtime value a bare literal is not: a parameter's value is
    /// unknown at compile time, so a `Param` reaching selection lowers to a local read rather than a
    /// constant. (Only present when a function body is lowered STANDALONE — i.e. it is emitted as a real
    /// wasm function, not inlined-and-folded at a constant call site.)
    Param { binder: StructId },
    /// A RUNTIME CLOSURE VALUE — a flat closure built on the value heap. The backend builds a product
    /// cell (the same tagless `arr` cell a tuple uses) whose slot 0 is `box-int(code)` — the funcref
    /// TABLE slot naming the lambda-lifted function's code — and whose remaining slots are the
    /// `captures` (the lambda's free variables, captured BY VALUE, each an ordinary runtime handle).
    /// Present only when a lambda must survive to run time (it could not be β-reduced away — it is
    /// passed as an argument to a RECURSIVE function, so the call cannot inline). A recursive closure's
    /// `code` is a STATIC table slot, never a heap pointer into itself, so the heap stays acyclic
    /// (`memory-and-resource-model.md` §The Value Heap Is Acyclic). `DESIGN-runtime-closures-rcdzc.md`
    /// §3. (This increment builds the EMPTY-capture combinator; a non-empty capture set is a later
    /// increment — a lambda with free variables declines the lift.)
    ///
    /// The `captures` are the lambda's free variables captured BY VALUE at the point the closure is
    /// BUILT, so applying it later observes those captured bindings — not whatever is in scope at the
    /// call site. A closure is an ordinary runtime VALUE — an i32 handle to its cell — so it can be bound
    /// to a name, passed as an argument (the case that forces this lift), returned, and stored in a
    /// compound exactly like any other value:
    //= spec/capabilities/core-semantics.md#a-function-is-a-first-class-value
    //# A function MUST be a value that can be bound to a name, passed as an argument, returned as a result, and stored in a data structure, like any other value.
    //= spec/capabilities/core-semantics.md#a-function-is-a-first-class-value
    //# A function value MUST capture the bindings in scope at the point it is created, so that applying it later observes those captured bindings rather than the bindings in scope at the point of application.
    Closure {
        code: usize,
        captures: Vec<StructId>,
    },
    /// A read of the k-th CAPTURED free variable inside a LIFTED closure body — `arr-get(env, 1 + index)`
    /// then the value's own unbox/borrow. The lifted function receives its closure CELL as its first wasm
    /// parameter (local slot 0, the "env"); a body reference to a captured variable reads it back from the
    /// env at cell index `1 + index` (cell slot 0 is `box-int(code)`, so captures start at 1). This is the
    /// runtime read a captured variable is (vs a `Param`, read from a wasm local): a captured value lives
    /// in the closure cell, not a parameter slot. `DESIGN-runtime-closures-rcdzc.md` §3.
    Captured {
        index: usize,
        /// The captured value's solved type (so selection knows the `get-*` to unbox by — a scalar cell
        /// read returns a boxed handle that must be unboxed, a compound stays a handle).
        ty: crate::ty::Ty,
    },
    /// Apply a RUNTIME CLOSURE VALUE at FULL ARITY via `call_indirect` through the funcref table. The
    /// `closure` operand is the closure CELL — a heap product whose slot 0 is `box-int(table-slot)` and
    /// whose remaining slots are the captures. The lifted function is invoked with `(env = the closure
    /// cell, args…)`: the cell and the args are pushed, then `arr-get(cell, 0)`+`get-int` reads the table
    /// slot for the indirection index. Present only when the applied head is a runtime function value — a
    /// function-typed PARAMETER `g` applied inside a (recursive) body. A single-arg application carries
    /// one arg; a MULTI-arg application `(g a b)` carries all of them (the lifted lambda is
    /// `(env, a, b) -> result`), so long as it is applied at FULL arity — a PARTIAL application of a
    /// runtime multi-param closure (runtime currying) still declines at the application site.
    /// `DESIGN-runtime-closures-rcdzc.md` §3.
    CallClosure {
        closure: StructId,
        args: Vec<StructId>,
    },
    /// A HOST CALL — a perform of an effect operation DELEGATED to the component boundary by an enclosing
    /// `(host (E…) …)` (`capabilities-and-effects.md` §Host-Binding Is A Routing Decision Made At The
    /// Entrypoint). The operation is a component-level WIT import: its declaring effect is an interface,
    /// the operation a function in it (a dotted `E.op` is never a top-level extern — the component model
    /// forbids the dot, so the boundary encoder maps it to `interface E` + `func op`). The backend emits
    /// each arg then a call to the imported function; `effect`/`op` name it symbolically (like
    /// `Lir::CallImport` for a runtime op), and the serializer resolves the pair to the import's core
    /// function index once the whole host-import set is laid out. `result` is the op's declared result
    /// type (the value the host returns) — the expression evaluated for the host call yields exactly that
    /// value, which is the unit value when the op's WIT signature returns unit. This increment lowers the
    /// SCALAR boundary (a scalar/unit arg and a scalar/unit result); a string/compound arg or result is a
    /// later increment (the old seed's `HostString` (ptr,len) shape).
    //= spec/capabilities/core-semantics.md#an-expression-evaluated-only-for-its-effect-yields-the-unit-value
    //# An expression evaluated only for the host call it makes MUST yield the value that host call returns, which is the unit value when the call's WIT signature returns unit.
    HostCall {
        effect: String,
        op: String,
        args: Vec<StructId>,
        result: crate::ty::Ty,
    },
    /// A SEQUENCING block — evaluate each `stmt` FOR ITS SIDE EFFECT (discarding its value), then evaluate
    /// `tail` as the block's value. The core form of a `(do S… tail)` whose non-final statements have
    /// OBSERVABLE side effects that must run (a host call — its call crosses the boundary even though its
    /// result is discarded). A `do` whose intermediates are PURE folds to just `tail` (the intermediates
    /// contribute nothing), so this node is produced ONLY when a statement reaches a side effect that
    /// selection must emit. The backend emits each stmt (a Unit-returning host call leaves nothing on the
    /// stack; a value-returning stmt would need a `drop`, not yet produced here) then the tail.
    ///
    /// Because the statements are emitted in written order and each host call is a straight-line boundary
    /// call, the host calls a program makes are observed in exactly the order the program made them —
    /// host-call order is part of the program's observable behavior, fixed by this emission order. The
    /// block's value is its `tail`; an effect-only `(do S…)` whose statements run for their host calls has
    /// a Unit-returning host call as its tail, so a program that terminates normally without producing a
    /// value other than through its host calls yields the unit value.
    //= spec/capabilities/core-semantics.md#host-calls-are-ordered-and-part-of-observable-behavior
    //# The sequence of host calls a program makes MUST be observed in the order the program made them.
    //= spec/capabilities/core-semantics.md#an-expression-evaluated-only-for-its-effect-yields-the-unit-value
    //# A program that terminates normally without producing a value other than through the host calls it makes MUST produce the unit value as its normal-termination value.
    Seq {
        stmts: Vec<StructId>,
        tail: StructId,
    },
    /// A produced "no" carried into the core.
    Poison(Reject),
}
