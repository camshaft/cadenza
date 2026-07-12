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
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PathStep {
    /// Descend into a sum variant's PAYLOAD — `sum-payload(handle)`. (A single-payload variant; a
    /// multi-payload variant's payload is a tuple, reached with a following `Elem`.)
    Payload,
    /// Descend into a tuple/record ARRAY cell at `index` — `arr-get(handle, index)`.
    Elem(usize),
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
    /// A boolean constant.
    ConstBool(bool),
    /// The unit value.
    Unit,
    /// A record value — a fixed set of named fields, each field's value referenced by its AST
    /// occurrence. A field read FOLDS (`core_of` of a member projects the field's core directly), so a
    /// `Record` that SURVIVES to selection is one used as a runtime value (e.g. returned) — the backend
    /// builds it on the value heap (`arr-alloc` + per-field `box-*`/`arr-set`, fields in canonical
    /// order). Carrying the variant lets member-access fold read the field set.
    Record { fields: BTreeMap<Symbol, StructId> },
    /// A TUPLE value — a fixed-arity positional product, each element referenced by its AST occurrence.
    /// Present only when the tuple SURVIVES to selection as a RUNTIME value (constructed from runtime
    /// operands, or a constant tuple that escapes — a projection of a compile-time-visible tuple folds to
    /// the element in `lower`, leaving no `Tuple`). The backend builds it on the value heap
    /// (`arr-alloc` + per-element `box-*`/`arr-set`), or — for a proven-CONSTANT tuple — builds it ONCE
    /// (the static build-once path, §2d). Elements are lowered on demand.
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
    /// A SUM VALUE CONSTRUCTION — `(Option.Some 5)` or a bare nullary `None`. `disc` is the variant's
    /// discriminant (read off the ctor's `(meta variant)` at lowering); `payloads` are the argument
    /// occurrences (empty for a nullary variant). The backend builds `sum-new(disc, payload)` where the
    /// payload handle is: an empty array `arr-alloc(0)` for a nullary variant (`value-heap-runtime.md`
    /// §Sum: "a nullary variant carries the unit value — an arr of length 0"), the single boxed payload
    /// for a one-payload variant, or a tuple handle built from the payloads for a multi-payload variant.
    /// The nominal tag is compile-time only — the runtime holds only `(disc, payload)`.
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
    MatchSum {
        scrutinee: StructId,
        /// The access PATH from `scrutinee` to the sub-value the ROOT switch tests — always empty (the
        /// root dispatches on the scrutinee itself); a nested switch carries its own full path in
        /// [`SumCont::Switch`]. Kept for uniform emit (every switch walks a path from the scrutinee).
        path: Vec<PathStep>,
        arms: Vec<SumArm>,
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
    /// A two-way conditional over atoms; structured control retained. Children are AST `StructId`s.
    If {
        cond: StructId,
        then_: StructId,
        else_: StructId,
    },
    /// A SHORT-CIRCUITING boolean conjunction `(and lhs rhs)` / disjunction `(or lhs rhs)` — present only
    /// when it did not fold to a constant in `lower`. `is_and` picks the semantics: `and` emits `if lhs
    /// then rhs else false`, `or` emits `if lhs then true else rhs` — so the RIGHT operand is evaluated
    /// only on the non-short-circuiting branch, shielding a trapping/effectful `rhs` exactly as a
    /// conditional's unselected branch does (core-semantics.md §Boolean Connectives Short-Circuit). The
    /// backend emits it as that `if` over the operands' i32 boolean values.
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
    Call { callee: usize, args: Vec<StructId> },
    /// A reference to a FUNCTION PARAMETER — the `binder` is the parameter's name occurrence (its
    /// identity, matching what `resolve` binds a reference to). The backend maps it to a `local.get` of
    /// the parameter's slot. This is the runtime value a bare literal is not: a parameter's value is
    /// unknown at compile time, so a `Param` reaching selection lowers to a local read rather than a
    /// constant. (Only present when a function body is lowered STANDALONE — i.e. it is emitted as a real
    /// wasm function, not inlined-and-folded at a constant call site.)
    Param { binder: StructId },
    /// A produced "no" carried into the core.
    Poison(Reject),
}
