//! Backend-AGNOSTIC Core-IR analysis primitives, shared by the wasm backend's Lir-level
//! LICM/CSE realization (`backend/wasm/select.rs`) and the backend-independent Core-IR optimization
//! passes (`opt.rs`). These are pure functions of the Core IR — value equivalence, structural hashing,
//! reference counting, the dominating frontier, subtree size, and the heap-type classification — with
//! ZERO backend/slot/`Lir` state, so a single definition serves both the wasm-emit realization and a
//! Core-level pass without duplicating the soundness-critical frontier + heap-type logic (two copies
//! would risk drifting on exactly the guards that keep an LICM/CSE hoist sound).
//!
//! Extracted VERBATIM from `backend/wasm/select.rs` (a pure move — the definitions are unchanged; the
//! call sites there now re-import from here). The REALIZATION-coupled predicates stay in `select.rs`:
//! `is_cse_shareable` bakes in the wasm CSE's hoist-BEFORE-body assumption (it excludes `Core::LocalRef`
//! because the let-local slot is unbound at the hoist point), which a Core pass that INTRODUCES the let
//! does not share — so it is deliberately NOT here.

use crate::ast::StructId;
use crate::core::Core;
use crate::db::Db;
use crate::lower::core_of;
use crate::ty::Ty;
use std::collections::HashMap;

/// Whether a solved type is a HEAP VALUE — one held as an owned runtime handle that the Perceus
/// contract reclaims (a tuple, record, sum, or list). A scalar (integer/bool/unit) owns no heap cell,
/// so it is never dup'd/drop'd. This is what decides which `let` bindings get a closing `drop`, and it
/// gates the branchless-`select` `if` lowering OUT for a heap result (a `select` on a handle would be
/// ill-formed). A `Ty::List` is an owned `vec-*` handle exactly like a tuple/record/sum — it MUST be
/// listed here, and `valtype_of` already agrees it is an i32 handle; omitting it let an `if` over a
/// list take the scalar `select` path and emit a module that failed wasm validation (i64/i32 mismatch).
///
/// This predicate is where the reference-count reclamation the emitted component CARRIES is decided: a
/// heap-typed `let` binding gets a `drop` emitted after the body (see `emit`), so the runnable form
/// releases each value's storage after its last use — the release point being a static consequence of
/// the source, not a later collector sweep — and the runtime it targets need supply only raw memory
/// (the `alloc`/`drop`/`dup` refcount discipline is emitted BY the component, imported by name).
//= spec/capabilities/memory-and-resource-model.md#reclamation-is-carried-by-the-runnable-form
//# The runnable form of a program MUST carry its own allocation and reclamation of values, so that the runtime it targets need provide only raw memory rather than a memory manager.
//= spec/capabilities/memory-and-resource-model.md#cleanup-is-source-determined
//# A value's storage MUST be released after its last use in a way the executable semantics defines, rather than at an unspecified later time.
pub(crate) fn is_heap_type(ty: &Ty) -> bool {
    match ty {
        Ty::Tuple(_)
        | Ty::Record(_)
        | Ty::Sum { .. }
        | Ty::List(_)
        | Ty::Map(_, _)
        | Ty::Set(_)
        // A String / Symbol is a heap ROPE at run time exactly as Bytes is (see `elem_needs_rope_
        // compaction`, which treats all three alike). It MUST count as heap here so a String/Symbol
        // binder/param threaded past a consuming use (`String.concat(acc, s)` where `s` is also passed to
        // a self-recursive call) is a Perceus RETAIN candidate — else no `dup` is emitted, the shared rope
        // is freed while still referenced, and the rope walk reads OUT OF BOUNDS past a depth threshold (a
        // wasm trap). Omitting String/Symbol here was the gap the List ops did not hit (List was included).
        | Ty::String
        | Ty::Symbol
        | Ty::Bytes => true,
        // A BIGINT is ALWAYS a heap leaf (never a fixnum immediate — `box_bigint`'s sign-magnitude raw
        // leaf) and a RATIONAL is a 2-BigInt-handle heap node (`box_rational_normalized`). Both are HEAP
        // values, so a `let`-bound / param BigInt or Rational threaded past a consuming use — e.g. a
        // Rational `m` bound from an `rmax`/`rabs` fold then FANNED into a compound constructor
        // (`Vec3(rzero()-m, rzero()-m, rzero()-m)` / `Vec3(m, m, m)`, the cad rotate-bbox shape) — MUST be a
        // Perceus RETAIN candidate. Else `collect_retain_candidate_binders` skips it, `mark_binder_dups`
        // never runs for it, NO `dup` is emitted for its multi-use, the shared handle is freed after the
        // first consume, and a later `arr-set` stores/derefs the recycled slot → wasm OUT OF BOUNDS (a
        // wrong-value/OOB miscompile). This is the numeric-leaf twin of the String/Symbol omission fixed
        // just above; `lower.rs`'s `ty_heap_walkable` already classifies `Ty::BigInt | Ty::Rational` as
        // heap, so this restores the select.rs/lower.rs agreement.
        | Ty::BigInt
        | Ty::Rational => true,
        // A quantity ERASES to its inner numeric type before the backend (`lower` strips the `Qty`), so
        // a `Ty::Qty` should not reach selection. Defensively classify it by its inner type — a quantity
        // over a heap numeric would be heap, but Layer 1's numerics are all scalars (int/float).
        Ty::Qty { inner, .. } => is_heap_type(inner),
        // A NOMINAL tag "adds nothing to the value's runtime representation" (type-system.md §156) — at run
        // time a `Ty::Nominal` IS its `inner` shape (a single-variant newtype like `(type Box (B (List T)))`
        // ERASES to the bare list handle). So a nominal binder wrapping a heap shape (`bx : Box` = a list) is
        // a heap value and MUST be a Perceus RETAIN candidate — else `bx` threaded past a consuming use of
        // its erased payload gets no `dup` and the shared handle is FBIP-mutated while still referenced
        // (drift). Classify by the erased inner shape, exactly like `Qty`.
        Ty::Nominal { inner, .. } => is_heap_type(inner),
        // A runtime CLOSURE VALUE (`Core::Closure`) is a flat arr cell on the value heap (`box-int(code)` +
        // captures), so a `Ty::Fn`-typed binder/param that holds one is a HEAP value and MUST be a Perceus
        // RETAIN candidate — else `collect_retain_candidate_binders` skips it, `mark_binder_dups` never runs,
        // NO `dup` is emitted for a multi-use, and the shared cell is freed while still referenced: a closure
        // DOUBLED into one collection literal (`#list(f f)`) puts the SAME rc==1 cell into two slots → the
        // list's drop double-frees it, or a residual direct apply reads a freed cell → a garbage funcref →
        // `call_indirect` trap (breaker's #5980 doubled-occurrence residual). A FULLY-SOLVED closure type
        // (`Ty::Fn(Int64, Int64)`, no free var) is the gap `is_heap_type_for_retain`'s `has_free_var` fallback
        // did NOT cover. This is the closure twin of the String/Symbol + BigInt/Rational omissions fixed above
        // (a heap value whose TYPE was not classified heap). A runtime `Fn` value is ONLY ever a heap closure
        // (`Core::Closure` — a lambda lift), so this is sound; the dup/drop of a closure cell is an ordinary
        // arr dup/drop.
        Ty::Fn(_, _) => true,
        _ => false,
    }
}

/// The child occurrences of `id` LICM descends looking for hoistable operands — the operand positions of
/// the pure operators, the branches/arms of control flow, and the operands of calls/heap ops. Kept
/// deliberately broad on the READ side (finding a hoist under any parent is sound), but it never returns a
/// binder/pattern occurrence (only value positions). A variant not listed yields no children (its
/// subexpressions are simply not searched — a missed opportunity, never an unsound hoist).
pub(crate) fn licm_children(db: &mut Db, id: StructId) -> Vec<StructId> {
    match core_of(db, id) {
        Core::Arith { lhs, rhs, .. }
        | Core::Compare { lhs, rhs, .. }
        | Core::StrCmp { lhs, rhs, .. }
        | Core::FloatCompare { lhs, rhs, .. }
        | Core::ValueEq { lhs, rhs }
        | Core::ValueCmp { lhs, rhs, .. }
        | Core::ValueEqShaped { lhs, rhs, .. }
        | Core::And { lhs, rhs, .. }
        | Core::ListConcat { lhs, rhs }
        | Core::MapMerge { lhs, rhs }
        | Core::BytesConcat { lhs, rhs } => vec![lhs, rhs],
        Core::Convert { operand, .. }
        | Core::Not { operand }
        | Core::Proj { operand, .. }
        | Core::ListLen { operand }
        | Core::BytesLen { operand }
        | Core::StrScalarLen { operand }
        | Core::BytesCompact { operand } => vec![operand],
        Core::MapSize { map } => vec![map],
        Core::SetLen { set } => vec![set],
        Core::SumPayload { scrutinee, .. } | Core::SumExpect { scrutinee, .. } => vec![scrutinee],
        Core::If {
            cond, then_, else_, ..
        } => vec![cond, then_, else_],
        Core::Let { body, bindings } => {
            // The bindings' INIT expressions are value positions (their binders are not); the body too.
            let mut v: Vec<StructId> = bindings.iter().map(|(_, init)| *init).collect();
            v.push(body);
            v
        }
        Core::Match { scrutinee, arms } => {
            let mut v = vec![scrutinee];
            v.extend(arms.iter().map(|a| a.body));
            v
        }
        Core::MatchList { scrutinee, arms } => {
            let mut v = vec![scrutinee];
            v.extend(arms.iter().map(|a| a.body));
            v
        }
        Core::Call { args, .. } => args.to_vec(),
        Core::Tuple { elems } | Core::ListNew { elems } | Core::BytesOf { elems } => elems.to_vec(),
        Core::ListPush { list, elem } | Core::ListPrepend { list, elem } => vec![list, elem],
        Core::ListAt { list, index, .. } => vec![list, index],
        // A variant with binders/patterns or an unanalyzed shape yields no searchable children.
        _ => vec![],
    }
}

/// Collect the DOMINATING FRONTIER of the body at `id` — the set of node occurrences that are ALWAYS
/// EVALUATED on entry, regardless of which branch any control flow takes. This is the emit-position
/// dominance set a CSE hoist to the top is sound against: a node in it runs before the rest of the body no
/// matter what, so computing it once up-front adds no work on any path and moves no trap (its trap, if any,
/// fires at the same first-occurrence point). The walk descends UNCONDITIONALLY-reached positions only: a
/// pure operator's operands and a `let`'s bindings+body are always evaluated, but an `If` conditionally
/// runs its branches — so descend ONLY its `cond` (always evaluated); likewise a `Match`/`MatchList`/
/// `MatchSum` runs only the selected arm, so descend ONLY its scrutinee. (A whole straight-line body is its
/// own frontier — no control flow prunes anything — so this subsumes the old `body_is_straight_line` gate.)
pub(crate) fn collect_dominating_frontier(
    db: &mut Db,
    id: StructId,
    out: &mut std::collections::HashSet<StructId>,
) {
    if !out.insert(id) {
        return; // already visited this occurrence
    }
    let unconditional: Vec<StructId> = match core_of(db, id) {
        // Control flow: only the DECIDING sub-value is always evaluated; the branches/arms are conditional.
        Core::If { cond, .. } => vec![cond],
        Core::Match { scrutinee, .. }
        | Core::MatchList { scrutinee, .. }
        | Core::MatchSum { scrutinee, .. } => vec![scrutinee],
        // A SHORT-CIRCUITING connective (`and`/`or`, `Core::And { is_and }`) evaluates its LEFT operand
        // unconditionally but SHIELDS its RIGHT operand exactly as a conditional's unselected branch:
        // `and` = `if lhs then rhs else false`, `or` = `if lhs then true else rhs` (core.rs Core::And doc;
        // core-semantics.md #Boolean Connectives Short-Circuit). So only `lhs` is in the dominating frontier
        // — `rhs` runs only on the non-short-circuiting path. WITHOUT this arm, `rhs` fell through to
        // `licm_children` (which returns both operands) and entered the frontier, so a repeated TRAPPING
        // `rhs` subexpression (`(and b (= (/ 10 d) (/ 10 d)))`) was CSE-hoisted to the body root and ran
        // unconditionally → a spurious divide-by-zero trap at `d=0` even when `b` is false (adv-55, a wasm
        // soundness miscompile via the always-on select.rs CSE; the O2 Core CSE shares this frontier too).
        Core::And { lhs, .. } => vec![lhs],
        // A `Core::HandleAbort` (v-effects, CASE 1) is a NON-LOCAL exit — its `value` is the whole handle's
        // result, emitted then followed by a bare wasm `return` that unwinds every enclosing block. Its
        // `value` subtree must NOT enter the dominating frontier: a scalar hoisted OUT of it to the body root
        // would be speculated onto every path (exactly the adv-55 short-circuit class above — a conditional/
        // divergent subexpression wrongly treated as always-evaluated, spuriously moving its trap/work).
        // Today `licm_children` already yields no children for it (so the `_` arm is empty), but making the
        // barrier EXPLICIT here fences the future general/nested HandleAbort (CASE 2 — a MID-function,
        // `handle_id`-keyed conditional abort whose `value` is genuinely conditional): a later change giving
        // `licm_children` a `HandleAbort => vec![value]` arm must not silently let the abort value be hoisted.
        Core::HandleAbort { .. } => vec![],
        // Everything else `licm_children` enumerates evaluates ALL its children unconditionally (a pure
        // operator's operands, a `let`'s bindings + body, a call's args, a compound's elements).
        _ => licm_children(db, id),
    };
    for child in unconditional {
        collect_dominating_frontier(db, child, out);
    }
}

/// Walk the value-position tree at `id` (via `licm_children`), recording per-StructId a REFERENCE COUNT
/// (how many parent edges point at it — a node reached twice counts 2) into `counts`, and the distinct
/// StructIds in first-seen order into `order`. A shared subtree's interior is walked ONCE (the count above
/// captures the node's own multiplicity); descending per visit would over-count nested nodes / blow up on
/// a deep DAG.
pub(crate) fn collect_node_refs(
    db: &mut Db,
    id: StructId,
    counts: &mut HashMap<StructId, u32>,
    order: &mut Vec<StructId>,
) {
    node_refs_via(db, id, licm_children, counts, order);
}

/// The shared ref-counting first-seen DFS both [`collect_node_refs`] (value-position tree, `licm_children`)
/// and [`b2_node_refs`] (complete `core_child_ids`, descends `MatchSum`) drive — parameterized by the
/// `children` walker. Per-StructId reference COUNT into `counts` (a node reached twice counts 2), distinct
/// StructIds in first-seen order into `order`; a shared subtree's interior is walked ONCE (the count
/// captures its own multiplicity — descending per visit would over-count / blow up on a deep DAG).
fn node_refs_via(
    db: &mut Db,
    id: StructId,
    children: fn(&mut Db, StructId) -> Vec<StructId>,
    counts: &mut HashMap<StructId, u32>,
    order: &mut Vec<StructId>,
) {
    let n = counts.entry(id).or_insert(0);
    *n += 1;
    if *n == 1 {
        order.push(id);
        for child in children(db, id) {
            node_refs_via(db, child, children, counts, order);
        }
    }
}

/// The B2 sharing-aware-emit DETECTION primitive (see `backend/wasm/DESIGN-sharing-aware-emit-let-slot.md`):
/// the distinct shared HEAP-HANDLE nodes in the value-position DAG rooted at `body` — a `StructId` reached
/// by ≥2 parent edges (`collect_node_refs` count ≥ 2) whose lowered type is a heap handle (`is_heap_type`).
/// These are the nodes a shared-DAG-walk re-descends exponentially and that a `Core::Let` slot binding
/// would compute ONCE (the durable fix for the cmb1/pom5 emit-phase re-descent). The shared `StructId` is
/// produced by `reduce_handle`'s resume-value substitution (the `copy_pure`/`resolved_subtrees` share-path,
/// effects.rs), NOT by a source `let` (which already names its value — one parent edge). Returned in
/// first-seen (`collect_node_refs` `order`) order for determinism.
///
/// SEAM-AGNOSTIC + PURE: only READS the (already-lowered) Core column via `core_of`/`type_of`; installs no
/// override and rewrites nothing, so it is safe to call from the post-layout B2 seam (it runs over a column
/// the layout phase has lowered, so no first-demand poison). Excludes `Core::LocalRef` (already a slot
/// read) and non-heap / `Unit` nodes (a scalar needs no dup/drop; a `Unit` handle owns no reference). The
/// distinct-fresh-binder slot construction + placement (dominating-frontier + unconditional-reach) are the
/// CALLER's job — this is detection only.
///
/// NOTE: `b2_bind_plan` (below) supersedes this for the live pass — it uses the COMPLETE `core_child_ids`
/// traversal (descends `MatchSum` arms), where this body-level detector sees only `licm_children`. This
/// remains as the body-level detector its own unit tests pin (the coverage-delta reference for the plan),
/// so it is `#[cfg(test)]` — retained + tested, but not on the production path.
#[cfg(test)]
pub(crate) fn collect_shared_heap_binding_candidates(db: &mut Db, body: StructId) -> Vec<StructId> {
    let mut counts: HashMap<StructId, u32> = HashMap::new();
    let mut order: Vec<StructId> = Vec::new();
    collect_node_refs(db, body, &mut counts, &mut order);
    let mut out: Vec<StructId> = Vec::new();
    for id in order {
        let count = counts.get(&id).copied().unwrap_or(0);
        if is_shared_heap_node(db, id, count) {
            out.push(id);
        }
    }
    out
}

/// Whether `id` is a SHARED HEAP-HANDLE node — the B2 gate-2 detection filter, shared by
/// [`collect_shared_heap_binding_candidates`] and [`b2_bind_plan`]. Three conditions: (1) SHARED — reached
/// by ≥2 parent edges (`count`, the re-descent source); (2) NOT already a slot read (`Core::LocalRef` —
/// binding it would be redundant/cyclic); (3) a HEAP HANDLE — `is_heap_type` and not `Unit` (a scalar share
/// is the scalar CSE's job with no dup/drop; a `Unit` owns no reference). `count` is the caller's
/// `node_refs_via` reference count for `id`.
fn is_shared_heap_node(db: &mut Db, id: StructId, count: u32) -> bool {
    if count < 2 {
        return false;
    }
    if matches!(core_of(db, id), Core::LocalRef { .. }) {
        return false;
    }
    let ty = crate::infer::type_of(db, id);
    is_heap_type(&ty) && !matches!(ty.strip_nominal(), Ty::Unit)
}

/// [B2] the FREE binders of `id`'s subtree — LocalRef/Param binders read within it that are NOT bound by a
/// `Core::Let` inside it. Complete `core_child_ids` traversal (descends MatchSum). The B2 template-share
/// gate reads this: a share whose free binders are all `Core::Param`s is a value-share (same value every
/// occurrence); one reading a re-threaded inner `Core::Let` binder is a state-threaded template (unsound).
pub(crate) fn free_binders_of(db: &mut Db, id: StructId) -> std::collections::HashSet<StructId> {
    let mut free = std::collections::HashSet::new();
    let mut bound = std::collections::HashSet::new();
    let mut seen = std::collections::HashSet::new();
    fn rec(
        db: &mut Db,
        id: StructId,
        free: &mut std::collections::HashSet<StructId>,
        bound: &mut std::collections::HashSet<StructId>,
        seen: &mut std::collections::HashSet<StructId>,
    ) {
        if !seen.insert(id) {
            return;
        }
        match core_of(db, id) {
            Core::LocalRef { binder } | Core::Param { binder } => {
                free.insert(binder);
            }
            Core::Let { bindings, .. } => {
                for (b, _) in bindings.iter() {
                    bound.insert(*b);
                }
            }
            _ => {}
        }
        for c in crate::backend::wasm::select::core_child_ids(db, id) {
            rec(db, c, free, bound, seen);
        }
    }
    rec(db, id, &mut free, &mut bound, &mut seen);
    free.retain(|b| !bound.contains(b));
    free
}

/// One entry of the B2 sharing-aware-emit BIND PLAN (see `backend/wasm/DESIGN-sharing-aware-emit-let-slot.md`):
/// a shared heap node the emit-side install (v-rust-backend) binds ONCE into a `Core::Let` slot. The plan
/// is the interface between v-core-opt's analysis (this) and v-rb's emit-coupled install: v-rb overrides
/// `scope_node` to a `Core::Let` whose single binding is `(fresh_binder, copy-of shared_id's core)` and
/// whose body is `scope_node`'s ORIGINAL core, then overrides each `member` to `Core::LocalRef{fresh_binder}`.
#[derive(Debug, Clone)]
pub(crate) struct B2BindPlanEntry {
    /// The node to install the `Core::Let` AT — the deepest node whose subtree contains ALL `members` (the
    /// members' LCA). For an arm-internal share this is the arm-body node, so the `Let` nests INSIDE the arm
    /// (binders in scope, handle-expr top untouched → no handler-emit perturbation). NEVER the fn/handle body.
    pub scope_node: StructId,
    /// The shared node whose core is COPIED into the binding's initializer.
    pub shared_id: StructId,
    /// The shared node's type — for the fresh binder's synth + slot valtype.
    pub ty: Ty,
    /// The K occurrence ids to repoint to `Core::LocalRef{fresh_binder}`.
    pub members: Vec<StructId>,
}

/// Compute the B2 bind plan for `body`: the shared heap nodes that are SOUND to bind-once into a `Core::Let`
/// slot, each with its install site. This is the v-core-opt half of the durable cmb1/pom5 fix; v-rb's
/// install consumes the returned plan. Pure analysis (no override). The four gates:
/// 1. DETECTION via the COMPLETE `core_child_ids` traversal (descends `MatchSum` arms — the arm-internal
///    shares that CAUSE the hang are invisible to `licm_children`, proven: cmb1 31 vs 127).
/// 2. HEAP-SHARE: parent-edge count ≥ 2, `is_heap_type`, not `LocalRef`/`Unit`. PLUS two bind-safety
///    whitelist predicates (v-rb, from the full-corpus drift-guard's pure-value miscompiles): (P1)
///    FULLY-SOLVED TYPE — the share's type must be `is_fully_solved` (no `Deferred`/`Var` int axis, no
///    `Any`/`Var`); an unsolved type gives the slot an indeterminate valtype → 'no local slot' emit failure
///    (2 of the 3 Record.with drift-guard shares had a Deferred-int type). (P3) NOT A MATCH SCRUTINEE — the
///    share must not be dispatched on (a `Match`/`MatchSum`/`MatchList` scrutinee, or a
///    `SumExpect`/`SumPayload` read); binding it behind a slot breaks the rust backend's nested-variant
///    subject resolution → a rust-only decline the wasm sweep can't see.
/// 3. NOT-A-TEMPLATE-SHARE / VALUE-STABILITY = LOOP-INVARIANCE: (3a) the share's free binders
///    (`free_binders_of`) must all be `Core::Param`s — a share reading a re-threaded inner `Core::Let`
///    binder is a STATE-THREADED template (cbk1/trn6: same node, different value per resume-state) and
///    binding it once is a MISCOMPILE. (3b) even a Param is stable-to-hoist ONLY when the body is NOT
///    recursive: in a recursive body a Param threaded through the self-call / loop back-edge is re-bound
///    per iteration, so the K uses of the shared StructId span DIFFERENT iterations and binding once
///    captures ONE iteration's value (rq3 99≠199 stale value, plt2 freed-alias trap). A CLOSED share (no
///    free binders) is loop-invariant regardless. This is the value-stability = loop-invariance test both
///    v-rb and v-agent-harness converged on — a Param is loop-invariant only if not reached through a
///    recursion back-edge; cmb1 (the fixable witness) is non-recursive, rq3/plt2 are recursive-driver.
/// 4. BIND-SAFE (byte-neutral): `is_trap_free(shared) OR collect_dominating_frontier(scope_node)` contains
///    the shared node. A trapping rep_init bound at an LCA that only CONDITIONALLY reaches a use is a MOVED
///    trap (opt-sweep divergence); but when scope_node's dominating frontier already contains the shared
///    node it binds on exactly the paths that already evaluated it (byte-neutral). Mirrors the LICM gate at
///    select.rs:5251. Admits cmb1: its recomputed divide (divisor k+1, NOT trap-free) sits in the inner-if
///    GUARD its arm-body LCA unconditionally evaluates → in the frontier.
pub(crate) fn b2_bind_plan(db: &mut Db, body: StructId) -> Vec<B2BindPlanEntry> {
    // B2 = HEAP-HANDLE shares. The detection + value-stability (3a/3b) + bind-safety (4) + members/LCA
    // machinery is share-KIND-AGNOSTIC (see `shared_node_bind_plan`); only the per-node ELIGIBILITY (gate 2
    // HEAP-SHARE + the v-rb whitelist P1/P3) is heap-specific, carried by `b2_heap_eligible`. The O2 scalar
    // CSE (a later slice) drives the SAME engine with a scalar eligibility predicate.
    shared_node_bind_plan(db, body, b2_heap_eligible)
}

/// Per-node ELIGIBILITY for a share KIND: returns `Some(ty)` when the node at `id` (reached by `count`
/// parent edges, reachable from `body`) is an eligible SHARE for this kind, `None` to skip it. ONLY the
/// KIND-SPECIFIC gates live here — B2's heap gate-2 + P1/P3 (`b2_heap_eligible`), or the O2 CSE's scalar
/// gate (a later slice). The SHARED value-stability (3a/3b) + bind-safety (4) gates + members/LCA are
/// applied uniformly by [`shared_node_bind_plan`], so they stay IDENTICAL across kinds (a divergence there
/// would let heap and scalar sharing disagree on what is sound).
type ShareEligibility = fn(&mut Db, StructId, StructId, u32) -> Option<Ty>;

/// B2's HEAP-HANDLE eligibility: gate (2) shared heap handle + (2b/P1) fully-solved type + (2c/P3) not a
/// rust-slot-UNSAFE sum-variant scrutinee. Returns the node's type (for the slot valtype) when admitted.
fn b2_heap_eligible(db: &mut Db, body: StructId, id: StructId, count: u32) -> Option<Ty> {
    // (2) HEAP-SHARE: shared (count≥2), not a LocalRef, a heap handle (not Unit) — see is_shared_heap_node.
    if !is_shared_heap_node(db, id, count) {
        return None;
    }
    let ty = crate::infer::type_of(db, id);
    // (2b) FULLY-SOLVED TYPE (v-rb bind-safety whitelist P1): a slot binding fixes the slot's valtype
    // to the shared node's type up front; an unsolved type (a Deferred int sign/width — the shape v-rb's
    // install dump showed on 2 Record.with shares) yields an indeterminate slot valtype → emit fails
    // 'let-binding reference has no local slot'. Require a determinate machine representation.
    if !ty.is_fully_solved() {
        return None;
    }
    // (2c) P3-NARROW (v-rb rust-grounded): a dispatched-on share is excluded ONLY when a read of it is a
    // SUM-VARIANT read (MatchSum/SumExpect, or a SumPayload with a `Payload` path step) — the rust
    // backend resolves such a payload via a bind minted by a MatchSum arm on the DIRECT scrutinee, so a
    // slotted LocalRef scrutinee has no bind and DECLINES ('sum payload has no bound match arm'), a
    // rust-only decline the wasm sweep can't see. A share whose dispatch-reads are ALL Tuple/Record/List
    // reads (Match/MatchList scrutinee, or SumPayload with an Elem/RestFrom-only path) is resolved
    // DIRECTLY off the (slotted) scrutinee by dot-index/split → rust-slot-SAFE, so it is ADMITTED for
    // bind-once. This narrows the old blanket "any dispatched-on share is excluded" P3 to exactly the
    // sum-variant shape P3 was added for. cmb1's 127 shares are all Tuple state-tuple reads → all
    // admitted (v-rb measured bind-once of them = both-backend 1 PASS, correct 828567056280870).
    if b2_is_dispatched_on(db, body, id) && !b2_dispatch_rust_slot_safe(db, body, id) {
        return None;
    }
    Some(ty)
}

/// The share-KIND-AGNOSTIC bind-plan ENGINE both B2 (heap handles) and the O2 scalar CSE (a later slice)
/// share: detect the shared-DAG nodes, and for each node the `eligible` predicate ADMITS, apply the uniform
/// value-stability (3a/3b) + bind-safety (4) gates + members/LCA, emitting a `B2BindPlanEntry`.
/// Parameterizing ONLY the per-node eligibility keeps the correctness-critical shared gates IDENTICAL across
/// kinds — the tick-cg handler-threaded-TEMPLATE gate (3a), the rq3/plt2 LOOP-INVARIANCE gate (3b), the
/// moved-trap bind-safety gate (4). Pure analysis (no override); v-rb's install consumes the returned plan.
fn shared_node_bind_plan(
    db: &mut Db,
    body: StructId,
    eligible: ShareEligibility,
) -> Vec<B2BindPlanEntry> {
    // (1) DETECTION: core_child_ids parent-edge counts (descends MatchSum), first-seen order.
    let mut counts: HashMap<StructId, u32> = HashMap::new();
    let mut order: Vec<StructId> = Vec::new();
    b2_node_refs(db, body, &mut counts, &mut order);

    // VALUE-STABILITY = LOOP-INVARIANCE (gate-3b): computed ONCE per body. A shared node's free binders are
    // loop-invariant (safe to bind once) only when the body is NOT recursive — in a recursive body a binder
    // threaded through the self-call / loop back-edge is re-bound to a DIFFERENT value per iteration, so the
    // K uses of the shared StructId span DIFFERENT iterations and binding once captures ONE iteration's value
    // (rq3 99!=199 stale, plt2 freed-alias trap). See the memory note: cmb1 (the fixable witness) is
    // non-recursive; rq3/plt2 (the miscompiles) are recursive-driver bodies.
    let body_is_recursive = crate::eval::is_recursive(db, body);

    let mut plan: Vec<B2BindPlanEntry> = Vec::new();
    for id in order {
        // (2) PER-KIND ELIGIBILITY (gate 2 + kind whitelist); yields the node's type for the slot when admitted.
        let count = counts.get(&id).copied().unwrap_or(0);
        let Some(ty) = eligible(db, body, id, count) else {
            continue;
        };
        // (3) NOT-A-TEMPLATE-SHARE / VALUE-STABILITY = LOOP-INVARIANCE.
        let free = free_binders_of(db, id);
        // (3a) every free binder must be a Core::Param — a share reading a re-threaded inner Let binder
        // denotes different values per dispatch (the cbk1/trn6 template class), unsound to bind once.
        if free
            .iter()
            .any(|&b| !matches!(core_of(db, b), Core::Param { .. }))
        {
            continue;
        }
        // (3b) even a Param is stable-to-hoist only when it is NOT reached through a recursion/loop
        // back-edge: in a recursive body a Param threaded to the self-call is re-bound per iteration, so a
        // share reading ANY free binder there is per-iteration-distinct (rq3/plt2). A CLOSED share (no free
        // binders) is loop-invariant regardless. Conservative: over-excluding a genuinely-invariant share in
        // a recursive body only forfeits the opt (leaves it re-descended) — never a miscompile.
        if body_is_recursive && !free.is_empty() {
            continue;
        }
        // The shared node is ONE StructId (the compact DAG shares it); its K PARENTS all read it, so the
        // "members" to repoint is that single id (count≥2 above already established the K-way sharing).
        let members = b2_member_occurrences(db, body, id);
        if members.is_empty() {
            continue; // not reachable from body (defensive)
        }
        // scope_node = the deepest node whose subtree contains the shared id (its nearest enclosing scope).
        let Some(scope_node) = b2_members_lca(db, body, &members) else {
            continue;
        };
        // (4) BIND-SAFE: trap-free OR the scope_node UNCONDITIONALLY REACHES the shared node. A trapping
        // rep_init (divide/index/overflow) bound at a dominator that only CONDITIONALLY reaches a use would
        // MOVE the trap onto a path that skipped it (opt-sweep divergence). But when scope_node's dominating
        // frontier already contains the shared id, binding there evaluates it on exactly the paths that
        // already did — byte-neutral. Mirrors the LICM hoist-safety gate (select.rs:5251
        // `is_trap_free || frontier.contains`). This admits cmb1: its recomputed divide sits in the inner-if
        // GUARD its LCA unconditionally evaluates, so the frontier contains it though the compound is a
        // (k+1)-divisor divide (is_trap_free false).
        if !crate::lower::is_trap_free(db, id) {
            let mut frontier = std::collections::HashSet::new();
            collect_dominating_frontier(db, scope_node, &mut frontier);
            if !frontier.contains(&id) {
                continue;
            }
        }
        plan.push(B2BindPlanEntry {
            scope_node,
            shared_id: id,
            ty,
            members,
        });
    }
    plan
}

/// Whether `target` is DISPATCHED ON anywhere reachable from `body` — it appears as the `scrutinee` of a
/// `Match`/`MatchSum`/`MatchList`, or as the `scrutinee` of a `SumExpect`/`SumPayload` (a variant
/// discriminant/payload read). Such a node must stay a DIRECT construction for the rust backend's
/// nested-variant lowering (it resolves the inner switch subject type from the entered-variant payload of
/// the direct node); repointing it to a slot `LocalRef` loses that → a rust-backend decline the wasm sweep
/// can't see. The B2 bind PLAN (P3) excludes a dispatched-on share. Walks via `core_child_ids` (the
/// complete traversal), a visited-set bounding it; checks the scrutinee field of each dispatch node.
fn b2_is_dispatched_on(db: &mut Db, body: StructId, target: StructId) -> bool {
    let mut seen = std::collections::HashSet::new();
    fn rec(
        db: &mut Db,
        id: StructId,
        target: StructId,
        seen: &mut std::collections::HashSet<StructId>,
    ) -> bool {
        if !seen.insert(id) {
            return false;
        }
        let hit = matches!(core_of(db, id),
            Core::Match { scrutinee, .. }
            | Core::MatchSum { scrutinee, .. }
            | Core::MatchList { scrutinee, .. }
            | Core::SumExpect { scrutinee, .. }
            | Core::SumPayload { scrutinee, .. }
            if scrutinee == target);
        if hit {
            return true;
        }
        for c in crate::backend::wasm::select::core_child_ids(db, id) {
            if rec(db, c, target, seen) {
                return true;
            }
        }
        false
    }
    rec(db, body, target, &mut seen)
}

/// P3-NARROW (BOTH-BACKEND-safe intersection): whether every dispatch-read of `target` reachable from
/// `body` is a NON-CONSUMING TUPLE/RECORD FIELD READ — a `SumPayload` whose path is a SINGLE `Elem(i)` step
/// (an `arr-get` dot-index off the scrutinee) — AND `target` is Tuple/Record-typed. Only THIS shape is
/// bind-once-safe on BOTH backends:
///   • RUST (v-rb, `emit_sum_payload`/`ctx.sum_binds`, backend/rust/expr.rs): a Tuple/Record `Elem` read is
///     resolved DIRECTLY off the (slotted) scrutinee by dot-index — no `MatchSum`-arm-minted bind — so a
///     slotted `LocalRef` scrutinee resolves fine (why cmb1's tuple reads passed rust; a sum-variant
///     `Payload` read instead needs the bind and declines).
///   • WASM: an `arr-get` is a NON-CONSUMING read, so sharing the scrutinee via a slot does not perturb the
///     Perceus linear-consumption accounting. A `MatchList` rest pattern lowers its tail binder to
///     `SumPayload{RestFrom(k)}` = a `vec-split` that CONSUMES/reshapes the list in place (FBIP); binding
///     such a scrutinee once and reading the slot K times is a use-after-free → wasm OUT-OF-BOUNDS (the
///     TWO-LIST FIFO opt-sweep divergence — O2/O3 trap vs O1 value). So `MatchList`/`RestFrom` are EXCLUDED
///     even though rust would not decline on them — the rust-safe set is WIDER than the both-backend-safe
///     set, and the opt-sweep is the arbiter that caught the difference.
/// EXCLUDED (any of): `Match`/`MatchList`/`MatchSum`/`SumExpect` scrutinee; a `SumPayload` with a `Payload`
/// or `RestFrom` step, a multi-step path, or over a non-Tuple/Record scrutinee. Returns true IFF `target`
/// is dispatched on (≥1 read) AND every read is this safe field-read shape — the caller uses it only to
/// REFINE the dispatched-on exclusion. cmb1's 127 state-tuple shares are all single-`Elem` reads over a
/// `Tuple` → admitted (v-rb measured bind-once of them = both-backend 1 PASS, correct 828567056280870).
/// Conservative by construction: over-exclusion only forfeits the opt, never miscompiles. Bounded walk.
fn b2_dispatch_rust_slot_safe(db: &mut Db, body: StructId, target: StructId) -> bool {
    // Gate on the scrutinee's own type first — only a Tuple/Record has non-consuming `Elem` field reads.
    let tty = crate::infer::type_of(db, target);
    if !matches!(tty, Ty::Tuple(_) | Ty::Record(_)) {
        return false;
    }
    let mut seen = std::collections::HashSet::new();
    let mut any = false;
    let mut all_safe = true;
    fn rec(
        db: &mut Db,
        id: StructId,
        target: StructId,
        seen: &mut std::collections::HashSet<StructId>,
        any: &mut bool,
        all_safe: &mut bool,
    ) {
        if !seen.insert(id) {
            return;
        }
        match core_of(db, id) {
            // The ONLY safe shape: a single-`Elem` SumPayload = a non-consuming `arr-get` dot-index.
            Core::SumPayload { scrutinee, path } if scrutinee == target => {
                *any = true;
                if !matches!(&path[..], [crate::core::PathStep::Elem(_)]) {
                    *all_safe = false;
                }
            }
            // Any OTHER dispatch read of the target is unsafe on at least one backend (sum-variant switch/
            // unwrap on rust; consuming list split on wasm; scalar/length match — conservatively excluded).
            Core::Match { scrutinee, .. }
            | Core::MatchList { scrutinee, .. }
            | Core::MatchSum { scrutinee, .. }
            | Core::SumExpect { scrutinee, .. }
                if scrutinee == target =>
            {
                *any = true;
                *all_safe = false;
            }
            _ => {}
        }
        for c in crate::backend::wasm::select::core_child_ids(db, id) {
            rec(db, c, target, seen, any, all_safe);
        }
    }
    rec(db, body, target, &mut seen, &mut any, &mut all_safe);
    any && all_safe
}

/// `collect_node_refs` twin using the COMPLETE `core_child_ids` traversal (descends `MatchSum` arms), for
/// B2 detection. Shares the ref-counting DFS via [`node_refs_via`] — only the child-walker differs.
fn b2_node_refs(
    db: &mut Db,
    id: StructId,
    counts: &mut HashMap<StructId, u32>,
    order: &mut Vec<StructId>,
) {
    node_refs_via(
        db,
        id,
        crate::backend::wasm::select::core_child_ids,
        counts,
        order,
    );
}

/// The occurrence ids of `target` reachable from `body` — for a shared node this is the SAME `StructId`
/// (the DAG shares one node), so this returns `[target]` when reachable. (The K parent EDGES are what the
/// re-descent walks; the emit-side repoint overrides the single shared `StructId`, which every parent reads.)
fn b2_member_occurrences(db: &mut Db, body: StructId, target: StructId) -> Vec<StructId> {
    // A compact shared DAG has ONE StructId for the shared value (reduce_handle's copy_pure share-path), so
    // the "members" to repoint is that one id; its K parents all read it. Confirm it is reachable.
    let mut seen = std::collections::HashSet::new();
    let mut found = false;
    b2_reachable(db, body, target, &mut seen, &mut found);
    if found { vec![target] } else { vec![] }
}

fn b2_reachable(
    db: &mut Db,
    id: StructId,
    target: StructId,
    seen: &mut std::collections::HashSet<StructId>,
    found: &mut bool,
) {
    if *found || !seen.insert(id) {
        return;
    }
    if id == target {
        *found = true;
        return;
    }
    for c in crate::backend::wasm::select::core_child_ids(db, id) {
        b2_reachable(db, c, target, seen, found);
    }
}

/// The LCA (deepest common dominator) of `members` within `body`: the deepest node whose subtree (via
/// `core_child_ids`) contains EVERY member. Since a shared node is ONE `StructId`, its members are that id,
/// and the LCA is the deepest node whose subtree contains it — but v-rb's install needs the deepest node
/// that DOMINATES it AND is a valid install point (a scope-introducing node whose body it can wrap). We
/// return the deepest node on the path from `body` to the shared id whose subtree contains the shared id
/// exactly once at each step; for the single-shared-id case this is the shared id's own nearest enclosing
/// arm/let/block. Implemented as: the deepest ancestor of `target` (in the core_child_ids tree) all of
/// whose OTHER-than-the-target-path children do not also contain target — i.e. the target's immediate
/// dominator that is a structural container. (First cut: the body itself if the target is reached once;
/// v-rb refines the exact install node from this dominator.)
fn b2_members_lca(db: &mut Db, body: StructId, members: &[StructId]) -> Option<StructId> {
    let target = *members.first()?;
    // Find the deepest node whose subtree contains `target` — walk down, at each node descend into the
    // unique child whose subtree contains target; stop when no single child does (target is here or splits).
    let mut cur = body;
    loop {
        let children = crate::backend::wasm::select::core_child_ids(db, cur);
        let mut containing: Vec<StructId> = Vec::new();
        for c in &children {
            let mut seen = std::collections::HashSet::new();
            let mut found = false;
            b2_reachable(db, *c, target, &mut seen, &mut found);
            if found {
                containing.push(*c);
            }
        }
        if containing.len() == 1 && containing[0] != target {
            cur = containing[0];
        } else {
            // Either target IS a direct child (bind at cur, the enclosing scope) or multiple children
            // contain it (cur is the convergence LCA). Return cur — the deepest dominating scope node.
            return Some(cur);
        }
    }
}

/// The number of nodes in the value-position subtree at `id` (via `licm_children`) — the CSE ordering key
/// (inner-first). A shared node is counted structurally; the absolute value only needs to be MONOTONE in
/// containment (a subtree is strictly larger than any subtree it contains), which this is.
pub(crate) fn subtree_size(db: &mut Db, id: StructId) -> u32 {
    1 + licm_children(db, id)
        .into_iter()
        .map(|c| subtree_size(db, c))
        .sum::<u32>()
}

/// Whether the nodes at `a` and `b` lower to the STRUCTURALLY IDENTICAL core computation — the basis
/// for common-subexpression elimination. Two nodes are equal iff their core forms are the same operator
/// over recursively-equal operands, bottoming out at the same param/local slot or the same constant.
/// This is used ONLY to decide whether a repeated operand can be computed ONCE and read twice, so it is
/// deliberately CONSERVATIVE: any core kind not enumerated here (a call, a conditional, a heap
/// construct — whose equality would need more than structural matching, or whose sharing is not clearly
/// safe) compares UNEQUAL, so CSE simply does not fire. Every kind that DOES compare equal is a PURE,
/// deterministic scalar computation (arithmetic/comparison/conversion/projection over equal operands,
/// or a leaf) — so computing it once and reusing the value is observably identical to computing it
/// twice, INCLUDING its trap behavior (a trapping subexpression traps at the same first-occurrence
/// point whether shared or not). Effects would break this, but rcdzc has none yet.
/// A CHEAP, SHALLOW structural hash of the core value at `id` — the O(N²)-partition pre-filter for
/// [`core_eq`]. It hashes the top kind's discriminant + leaf values + the DISCRIMINANTS of immediate
/// children (one level; NOT the whole subtree), so it is O(1)-ish per candidate. The soundness contract
/// is one-directional and exactly what a hash-bucket pre-filter needs: **`core_eq(a,b)` ⇒
/// `core_hash_key(a) == core_hash_key(b)`** (two values `core_eq` considers equal have the same kind,
/// same leaf values, and same immediate-child kinds, so they hash identically). The converse need NOT
/// hold — two different values may collide — because within a bucket the exact `core_eq` still decides.
/// So bucketing candidates by this key, then running `core_eq` only WITHIN a bucket, is behaviour-
/// identical to the old all-pairs `core_eq` scan, but turns O(cands²) `core_eq` calls (each a
/// subtree-cloning walk) into O(cands) key computations + `core_eq` only among genuine collisions.
///
/// This is a FULL-DEPTH structural hash (kind + leaf values + each child's own full hash), MEMOIZED in
/// `memo` so each node is hashed exactly ONCE — the whole pass over all candidates is O(total core
/// nodes), NOT O(candidates · depth). A shallow (one-level) hash was insufficient: on a UNIFORM-shape
/// body — a deep left-nested `(+ (+ … (* p 0)) (* p 1))` accumulator chain, or N distinct `(* p k)`
/// terms — every node has the SAME shallow key (`Arith(Add)` over `[Arith,Arith]`, or `Arith(Mul)` over
/// `[Param,ConstInt]`), so ALL candidates collided into ONE bucket and the within-bucket `core_eq`
/// scan degraded right back to O(N²) with deep-recursive compares (measured: `deep_runtime_arith` N=400
/// = 486ms, ~8×/dbl). Recursing to the LEAVES separates them by their differing constants/params, so a
/// uniform chain's candidates land in distinct buckets. Contract unchanged: `core_eq(a,b) ⇒ hash(a) ==
/// hash(b)` (equal core forms hash identically all the way down), so the exact `core_eq` still decides
/// within a bucket — behaviour-identical, only the bucketing is finer.
pub(crate) fn core_hash_key(
    db: &mut Db,
    id: StructId,
    memo: &mut crate::fxhash::FxHashMap<StructId, u64>,
) -> u64 {
    use std::hash::{Hash, Hasher};
    if let Some(&h) = memo.get(&id) {
        return h;
    }
    let mut h = crate::fxhash::FxHasher::default();
    match core_of(db, id) {
        Core::ConstInt(v) => {
            1u8.hash(&mut h);
            v.to_i64().hash(&mut h);
        }
        Core::ConstBool(b) => {
            2u8.hash(&mut h);
            b.hash(&mut h);
        }
        Core::Unit => 3u8.hash(&mut h),
        Core::Param { binder } => {
            4u8.hash(&mut h);
            binder.0.hash(&mut h);
        }
        Core::LocalRef { binder } => {
            5u8.hash(&mut h);
            binder.0.hash(&mut h);
        }
        Core::Arith { op, lhs, rhs } => {
            10u8.hash(&mut h);
            op.hash(&mut h);
            core_hash_key(db, lhs, memo).hash(&mut h);
            core_hash_key(db, rhs, memo).hash(&mut h);
        }
        Core::Compare { op, lhs, rhs } => {
            11u8.hash(&mut h);
            op.hash(&mut h);
            core_hash_key(db, lhs, memo).hash(&mut h);
            core_hash_key(db, rhs, memo).hash(&mut h);
        }
        Core::Convert { op, operand } => {
            12u8.hash(&mut h);
            op.hash(&mut h);
            core_hash_key(db, operand, memo).hash(&mut h);
        }
        Core::Not { operand } => {
            13u8.hash(&mut h);
            core_hash_key(db, operand, memo).hash(&mut h);
        }
        Core::Proj { operand, index } => {
            14u8.hash(&mut h);
            index.hash(&mut h);
            core_hash_key(db, operand, memo).hash(&mut h);
        }
        // Everything else (float-compare, collection counts, indexed reads, payload reads, `if`, and any
        // kind `core_eq` treats as UNEQUAL) hashes by its bare kind tag. Still a SOUND pre-filter (equal
        // values share a kind); these are far rarer as CSE candidates so their buckets stay small.
        other => {
            std::mem::discriminant(&other).hash(&mut h);
        }
    }
    let out = h.finish();
    memo.insert(id, out);
    out
}

pub(crate) fn core_eq(db: &mut Db, a: StructId, b: StructId) -> bool {
    if a == b {
        return true; // the SAME occurrence — trivially identical.
    }
    match (core_of(db, a), core_of(db, b)) {
        (Core::ConstInt(x), Core::ConstInt(y)) => x.eq_value(&y),
        (Core::ConstBool(x), Core::ConstBool(y)) => x == y,
        (Core::Unit, Core::Unit) => true,
        // A leaf reference: equal iff the SAME binder (same param/local slot → same value).
        (Core::Param { binder: x }, Core::Param { binder: y }) => x == y,
        (Core::LocalRef { binder: x }, Core::LocalRef { binder: y }) => x == y,
        // A pure binary op: same operator over recursively-equal operands. (Arithmetic and comparison
        // are the operators whose two runtime operands can be the shared subexpression.)
        (
            Core::Arith {
                op: ox,
                lhs: lx,
                rhs: rx,
            },
            Core::Arith {
                op: oy,
                lhs: ly,
                rhs: ry,
            },
        )
        | (
            Core::Compare {
                op: ox,
                lhs: lx,
                rhs: rx,
            },
            Core::Compare {
                op: oy,
                lhs: ly,
                rhs: ry,
            },
        ) => ox == oy && core_eq(db, lx, ly) && core_eq(db, rx, ry),
        // A pure float equality (`Core::FloatCompare`): same operator AND WIDTH over recursively-equal
        // operands. `is_cse_shareable` already admits a `FloatCompare` (it is a total canon-and-compare),
        // so `core_eq` MUST recognize two equal ones or the CSE could never fire for it — the sibling of
        // the `Compare` arm above, plus the `width` (an f32-eq and an f64-eq of the same operands are
        // DIFFERENT machine ops — `i32.eq` over canon f32 bits vs `i64.eq` over canon f64 bits — so a
        // width mismatch is not the same value).
        (
            Core::FloatCompare {
                op: ox,
                lhs: lx,
                rhs: rx,
                width: wx,
            },
            Core::FloatCompare {
                op: oy,
                lhs: ly,
                rhs: ry,
                width: wy,
            },
        ) => ox == oy && wx == wy && core_eq(db, lx, ly) && core_eq(db, rx, ry),
        // A pure conversion: same op over an equal operand AND the same TARGET TYPE. `Core::Convert`'s `op`
        // is only the conversion PRIM (e.g. `Prim::Wrap`) — the truncation TARGET width/signedness is NOT in
        // `op`; the backend reads it off the CONVERT NODE'S OWN solved type at selection (`int_ty_of(id)` →
        // `Machine`). So `Int32.wrap n` and `UInt8.wrap n` share `op = Prim::Wrap` AND the same operand `n`
        // and would MERGE on `ox == oy && core_eq(px,py)` alone — the second reusing the first's slot → both
        // take the first's width (an order-dependent SILENT WRONG VALUE: `(+ (Int64.of (Int32.wrap n)) (Int64.of
        // (UInt8.wrap n)))` at n=300 gave 600/88 instead of 344). Compare the two nodes' solved TARGET types so
        // width-different conversions of one operand stay DISTINCT. Over-strict (two nominal wrappers of the
        // same machine type won't share) is SAFE — it only forgoes a share, never merges a differing width.
        (
            Core::Convert {
                op: ox,
                operand: px,
            },
            Core::Convert {
                op: oy,
                operand: py,
            },
        ) => {
            ox == oy
                && crate::infer::type_of(db, a) == crate::infer::type_of(db, b)
                && core_eq(db, px, py)
        }
        // A tuple projection: same index off an equal (runtime) operand.
        (
            Core::Proj {
                operand: px,
                index: ix,
            },
            Core::Proj {
                operand: py,
                index: iy,
            },
        ) => ix == iy && core_eq(db, px, py),
        // A COLLECTION COUNT (`List.len`/`Bytes.len`/`Map.size`/`Set.len`) is a TOTAL O(1) BORROWING read
        // returning a SCALAR — pure, no rc change, deterministic — so two counts of an equal collection
        // yield the same value and share safely (the count analogue of `Proj`/`SumPayload`). This lets CSE
        // compute a repeated `(List.len xs)` — a `vec-len` runtime import — ONCE across `(+ (len xs) (* (len
        // xs) 3))`. Each takes ONE operand handle; equal iff those handles are `core_eq`.
        (Core::ListLen { operand: ox }, Core::ListLen { operand: oy })
        | (Core::BytesLen { operand: ox }, Core::BytesLen { operand: oy })
        | (Core::StrScalarLen { operand: ox }, Core::StrScalarLen { operand: oy }) => {
            core_eq(db, ox, oy)
        }
        (Core::MapSize { map: mx }, Core::MapSize { map: my }) => core_eq(db, mx, my),
        (Core::SetLen { set: sx }, Core::SetLen { set: sy }) => core_eq(db, sx, sy),
        // A sum-variant payload read: equal iff the SAME path off an equal (runtime) scrutinee — the
        // pattern-binder analogue of `Proj`. `sum-payload`/`get-*` BORROW the handle and are pure (no rc
        // change, no effect), so two reads of the same payload yield the same value; sharing them lets the
        // arith-CSE compute `(Some x)`'s `x` ONCE for `(+ x x)` exactly as it already does for a repeated
        // tuple/record field `(+ (. r x) (. r x))`. `path` is a small `Vec<PathStep>` (each `Copy`), so
        // `==` is a cheap element compare.
        (
            Core::SumPayload {
                scrutinee: sx,
                path: px,
            },
            Core::SumPayload {
                scrutinee: sy,
                path: py,
            },
        ) => px == py && core_eq(db, sx, sy),
        // A `List.at`/`Bytes.at` indexed read: equal iff the SAME (list/bytes, index) off equal operands,
        // with the same Option discriminants. `vec-get`/`bytes-get` (behind a bounds check) BORROW the
        // sequence and are deterministic (no rc change, no effect), so two reads of the same element yield
        // the same `Option` value. Shared only as the scrutinee of a scalar-unwrapping `SumExpect` (an
        // `Option`-typed node is filtered from candidacy by the scalar gate); both operands `core_eq`.
        (
            Core::ListAt {
                list: lx,
                index: ix,
                disc_some: sx,
                disc_none: nx,
            },
            Core::ListAt {
                list: ly,
                index: iy,
                disc_some: sy,
                disc_none: ny,
            },
        ) => sx == sy && nx == ny && core_eq(db, lx, ly) && core_eq(db, ix, iy),
        (
            Core::BytesAt {
                bytes: bx,
                index: ix,
                disc_some: sx,
                disc_none: nx,
            },
            Core::BytesAt {
                bytes: by,
                index: iy,
                disc_some: sy,
                disc_none: ny,
            },
        ) => sx == sy && nx == ny && core_eq(db, bx, by) && core_eq(db, ix, iy),
        // An `Option.expect`/`Result.expect` (`SumExpect`) unwrap: equal iff the SAME present-discriminant
        // off an equal scrutinee. Borrowing + deterministic (present → the payload, absent → trap); two
        // identical unwraps yield the same value and trap identically. This is what makes a repeated
        // `(Option.expect (List.at xs i))` — scalar-valued — compute its bounds-check + `vec-get` + unbox
        // ONCE across `(+ (…at xs i) (…at xs i))`, the indexed-read analogue of the `List.len` CSE.
        (
            Core::SumExpect {
                scrutinee: sx,
                disc_present: dx,
            },
            Core::SumExpect {
                scrutinee: sy,
                disc_present: dy,
            },
        ) => dx == dy && core_eq(db, sx, sy),
        // A `Map.lookup`: equal iff the SAME map, key, and Option discriminants. It BORROWS the map and is
        // deterministic; two lookups of the same key yield the same `Option`. Shared only as the scrutinee
        // of a scalar-unwrapping `SumExpect` (an `Option`-typed node is filtered from candidacy by the
        // scalar gate). The `key_ty`/`val_ty` fields are derived from the operands (identical when `core_eq`)
        // and `Ty` is not `PartialEq`, so they are not compared.
        (
            Core::MapLookup {
                map: mx,
                key: kx,
                disc_some: sx,
                disc_none: nx,
                ..
            },
            Core::MapLookup {
                map: my,
                key: ky,
                disc_some: sy,
                disc_none: ny,
                ..
            },
        ) => sx == sy && nx == ny && core_eq(db, mx, my) && core_eq(db, kx, ky),
        // A boolean negation: equal iff the negated operands are. `not` is `i32.eqz` — pure and total.
        (Core::Not { operand: ox }, Core::Not { operand: oy }) => core_eq(db, ox, oy),
        // A conditional `select`/`if`: equal iff the condition AND both branches are recursively equal —
        // then the two `if`s compute the identical value, so the arith-CSE can compute the whole `if` ONCE
        // and read it twice (`(+ (if (< a b) a b) (if (< a b) a b))` = min(a,b) computed once). `core_eq`
        // returns true here ONLY when cond/then/else all match its PURE set (a leaf, arith, compare,
        // convert, proj, payload, not, or a nested pure `if`), so a branch with a call/effect never
        // qualifies — the shared `if` is pure and deterministic, safe to compute once. Both arms are
        // evaluated in neither the original nor the shared form (an `if` runs one branch), so no trap is
        // added or dropped by sharing.
        (
            Core::If {
                cond: cx,
                then_: tx,
                else_: ex,
            },
            Core::If {
                cond: cy,
                then_: ty,
                else_: ey,
            },
        ) => core_eq(db, cx, cy) && core_eq(db, tx, ty) && core_eq(db, ex, ey),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lower::synth_core;

    // The B2 detection primitive reports a HEAP node reached by ≥2 parent edges — the multi-parent shared
    // `StructId` `reduce_handle`'s resume-value substitution produces (the emit-phase re-descent source),
    // which a `Core::Let` slot would compute once. Built directly with `synth_core` so the DAG shape is
    // deterministic and independent of `reduce_handle`'s folding: `ListConcat{lhs: x, rhs: x}` over ONE
    // heap `x` gives `x` two parent edges (`licm_children(ListConcat) = [lhs, rhs]`).
    #[test]
    fn detection_reports_a_two_parent_heap_node_not_a_scalar_or_unshared_one() {
        let mut db = crate::db::Db::load(crate::testkit::parse(
            "(module m (def (main) 0) (export main))",
        ));
        let list_int = Ty::List(Box::new(Ty::int64()));
        // ONE heap child node, reached from BOTH operands of a concat → 2 parent edges.
        let x = synth_core(
            &mut db,
            Core::ListNew { elems: [].into() },
            list_int.clone(),
        );
        let shared = synth_core(&mut db, Core::ListConcat { lhs: x, rhs: x }, list_int);
        let cands = collect_shared_heap_binding_candidates(&mut db, shared);
        assert!(
            cands.contains(&x),
            "the twice-reached heap ListNew node is a shared-heap binding candidate"
        );
        for &c in &cands {
            assert!(is_heap_type(&crate::infer::type_of(&mut db, c)));
            assert!(!matches!(core_of(&mut db, c), Core::LocalRef { .. }));
        }

        // A SCALAR twice-reached node is NOT a candidate (scalar CSE's job, not B2's — no dup/drop).
        let s = synth_core(
            &mut db,
            Core::ConstInt(crate::ast::IntValue::from_i64(7)),
            Ty::int64(),
        );
        let scalar_shared = synth_core(
            &mut db,
            Core::Arith {
                op: crate::resolved::Prim::Add,
                lhs: s,
                rhs: s,
            },
            Ty::int64(),
        );
        assert!(
            !collect_shared_heap_binding_candidates(&mut db, scalar_shared).contains(&s),
            "a twice-reached SCALAR node is not a B2 heap candidate"
        );
    }

    // SOUNDNESS FENCE (v-effects `Core::HandleAbort`, CASE 1 — non-local exit): the abort `value` subtree
    // must NOT enter the dominating frontier. HandleAbort emits its `value` then a bare wasm `return` that
    // unwinds every enclosing block — so a scalar hoisted OUT of the abort value to the body root would be
    // SPECULATED onto every path (the adv-55 short-circuit class: a divergent/conditional subexpression
    // wrongly treated as always-evaluated, moving its work/trap). This pins that `collect_dominating_frontier`
    // yields ONLY the HandleAbort node itself, never a node inside its `value` — so neither the O2 CSE hoist
    // (guard D1) nor b2's unconditional-reach gate can lift a share out of / across the abort. Guards the
    // future CASE-2 (mid-function, `handle_id`-keyed CONDITIONAL abort) against a permissive `licm_children`
    // refactor: even if `licm_children` were later given a `HandleAbort => vec![value]` arm, the explicit
    // frontier arm keeps the abort value out of the hoist frontier.
    #[test]
    fn dominating_frontier_does_not_descend_into_a_handle_abort_value() {
        let mut db = crate::db::Db::load(crate::testkit::parse(
            "(module m (def (main) 0) (export main))",
        ));
        // A HandleAbort whose `value` is a repeated-scalar `(+ (& p 7) (& p 7))` — the exact shape the CSE
        // frontier would normally treat as an always-evaluated share candidate.
        let p = synth_param(&mut db);
        let c7 = synth_core(
            &mut db,
            Core::ConstInt(crate::ast::IntValue::from_i64(7)),
            Ty::int64(),
        );
        let masked = synth_core(
            &mut db,
            Core::Arith {
                op: crate::resolved::Prim::BitAnd,
                lhs: p,
                rhs: c7,
            },
            Ty::int64(),
        );
        let value = synth_core(
            &mut db,
            Core::Arith {
                op: crate::resolved::Prim::Add,
                lhs: masked,
                rhs: masked,
            },
            Ty::int64(),
        );
        let handle_id = synth_id_marker(&mut db);
        let ha = synth_core(&mut db, Core::HandleAbort { value, handle_id }, Ty::int64());

        let mut frontier = std::collections::HashSet::new();
        collect_dominating_frontier(&mut db, ha, &mut frontier);
        assert!(
            frontier.contains(&ha),
            "the HandleAbort node itself is in its own frontier (the walk's root insert)"
        );
        assert!(
            !frontier.contains(&value),
            "the abort VALUE must NOT enter the dominating frontier — it runs only on the non-local-exit path"
        );
        assert!(
            !frontier.contains(&masked),
            "no scalar INSIDE the abort value enters the frontier → nothing is hoisted out of / across the abort"
        );
    }

    // b2_bind_plan emits an entry for a trap-free param-closed shared heap node (a value-share), with the
    // shared id + a scope_node that dominates it. A scalar share or a trapping share is excluded.
    #[test]
    fn bind_plan_emits_a_value_share_entry_with_a_dominating_scope() {
        let mut db = crate::db::Db::load(crate::testkit::parse(
            "(module m (def (main) 0) (export main))",
        ));
        let list_int = Ty::List(Box::new(Ty::int64()));
        // A heap ListNew (trap-free, no free binders) reached from both operands of a concat → shared.
        let x = synth_core(
            &mut db,
            Core::ListNew { elems: [].into() },
            list_int.clone(),
        );
        let body = synth_core(
            &mut db,
            Core::ListConcat { lhs: x, rhs: x },
            list_int.clone(),
        );
        let plan = b2_bind_plan(&mut db, body);
        let entry = plan.iter().find(|e| e.shared_id == x);
        assert!(
            entry.is_some(),
            "a trap-free shared heap ListNew is a bind-plan entry"
        );
        let e = entry.unwrap();
        assert_eq!(e.ty, list_int, "entry carries the shared node's type");
        // scope_node dominates the shared id (its subtree contains it).
        let mut seen = std::collections::HashSet::new();
        let mut found = false;
        b2_reachable(&mut db, e.scope_node, x, &mut seen, &mut found);
        assert!(found, "scope_node dominates the shared node");
    }

    // GATE-3 (not-a-template): a shared heap node that reads a NON-Param binder (an inner Core::Let binder,
    // i.e. state that a resume could re-thread) is EXCLUDED — binding a state-threaded template once
    // collapses distinct per-dispatch values (the cbk1/trn6 miscompile class). Slice-1 admits only
    // param-closed value-shares. (Slice-2 will refine this to admit a bound-once inner-Let binder.)
    #[test]
    fn bind_plan_excludes_a_share_reading_an_inner_let_binder() {
        let mut db = crate::db::Db::load(crate::testkit::parse(
            "(module m (def (main) 0) (export main))",
        ));
        let list_int = Ty::List(Box::new(Ty::int64()));
        // A binder id `k` + a heap node that READS it via LocalRef, shared twice, then wrapped in a Let
        // that binds `k` — so the share's free binder `k` is an inner Let binder (non-Param).
        let k = synth_core(
            &mut db,
            Core::ConstInt(crate::ast::IntValue::from_i64(0)),
            Ty::int64(),
        );
        let reads_k = synth_core(&mut db, Core::LocalRef { binder: k }, list_int.clone());
        let shared = synth_core(
            &mut db,
            Core::ListConcat {
                lhs: reads_k,
                rhs: reads_k,
            },
            list_int.clone(),
        );
        let k_init = synth_core(
            &mut db,
            Core::ListNew { elems: [].into() },
            list_int.clone(),
        );
        let body = synth_core(
            &mut db,
            Core::Let {
                bindings: [(k, k_init)].into(),
                body: shared,
            },
            list_int,
        );
        let plan = b2_bind_plan(&mut db, body);
        assert!(
            !plan.iter().any(|e| e.shared_id == reads_k),
            "a shared heap node reading an inner Let binder is excluded (slice-1: param-closed only)"
        );
    }

    // GATE-4 (bind-safe): a TRAPPING shared heap node that is only CONDITIONALLY reached from its scope_node
    // is EXCLUDED — binding a trapping value at a dominator that does NOT unconditionally reach a use would
    // MOVE the trap onto a path that skipped it (opt-sweep divergence). Here the trapping share sits in BOTH
    // branches of an `if` (so it is shared, count 2) but NOT in the cond, so the LCA is the `if` node whose
    // dominating frontier is just the cond → the share is not unconditionally reached → excluded. (Contrast
    // the admit test above, where a concat reaches the same shape unconditionally.)
    #[test]
    fn bind_plan_excludes_a_trapping_share_reached_only_conditionally() {
        let mut db = crate::db::Db::load(crate::testkit::parse(
            "(module m (def (main) 0) (export main))",
        ));
        let list_int = Ty::List(Box::new(Ty::int64()));
        // Real params p, q; a checked divide `p / q` (runtime divisor → trapping); a ListNew carrying it
        // (heap, NOT trap-free). Placed in BOTH branches of an `if` over a param cond → shared, but reached
        // only under the branch, never by the cond the `if` evaluates unconditionally.
        let p = synth_param(&mut db);
        let q = synth_param(&mut db);
        let cond_binder = synth_id_marker(&mut db);
        let cond = synth_core(
            &mut db,
            Core::Param {
                binder: cond_binder,
            },
            Ty::Bool,
        );
        let div = synth_core(
            &mut db,
            Core::Arith {
                op: crate::resolved::Prim::Div,
                lhs: p,
                rhs: q,
            },
            Ty::int64(),
        );
        let trapping_list = synth_core(
            &mut db,
            Core::ListNew {
                elems: [div].into(),
            },
            list_int.clone(),
        );
        let body = synth_core(
            &mut db,
            Core::If {
                cond,
                then_: trapping_list,
                else_: trapping_list,
            },
            list_int,
        );
        assert!(
            !crate::lower::is_trap_free(&mut db, trapping_list),
            "a ListNew carrying a runtime divide is not trap-free (test premise)"
        );
        let plan = b2_bind_plan(&mut db, body);
        assert!(
            !plan.iter().any(|e| e.shared_id == trapping_list),
            "a trapping share reached only under a branch is excluded (gate-4: not unconditionally reached)"
        );
    }

    // A distinct StructId to serve as a Param's binder key in a test.
    fn synth_id_marker(db: &mut Db) -> StructId {
        synth_core(
            db,
            Core::ConstInt(crate::ast::IntValue::from_i64(0)),
            Ty::int64(),
        )
    }

    // A `Core::Param` OCCURRENCE whose `binder` field resolves (via `core_of`) to a `Core::Param` too — the
    // real lowered shape (a param reference points at the parameter's binding occurrence, itself a Param).
    // So `free_binders_of` of a node reading this occurrence yields a binder that passes gate-3a's
    // "all free binders are Core::Param" check (unlike `synth_id_marker`, a ConstInt, which fails gate-3a).
    fn synth_param(db: &mut Db) -> StructId {
        let marker = synth_id_marker(db);
        let binder = synth_core(db, Core::Param { binder: marker }, Ty::int64());
        synth_core(db, Core::Param { binder }, Ty::int64())
    }

    // GATE-4 (bind-safe, UNCONDITIONAL-REACH arm): a TRAPPING but PARAM-CLOSED shared heap node whose
    // scope_node (LCA) UNCONDITIONALLY REACHES it (the scope_node's dominating frontier contains the shared
    // id) IS planned — binding there evaluates the trap on exactly the paths that already did (byte-neutral).
    // This is the cmb1-unblocking arm: cmb1's recomputed divide is NOT trap-free (divisor k+1) yet its
    // arm-body LCA unconditionally reaches it, so it must be admitted. (The trap-free-only first slice would
    // have wrongly excluded it → empty plan → cmb1 stays hung, as v-rb's install opt-sweep showed.)
    #[test]
    fn bind_plan_admits_a_trapping_param_closed_share_reached_unconditionally() {
        let mut db = crate::db::Db::load(crate::testkit::parse(
            "(module m (def (main) 0) (export main))",
        ));
        let list_int = Ty::List(Box::new(Ty::int64()));
        // Two real params p, q; a checked divide `p / q` (runtime divisor → trapping); a ListNew carrying it
        // (heap, NOT trap-free), shared from both operands of a concat. The concat evaluates both operands
        // unconditionally, so its dominating frontier contains the shared list → gate-4's reach arm admits.
        let p = synth_param(&mut db);
        let q = synth_param(&mut db);
        let div = synth_core(
            &mut db,
            Core::Arith {
                op: crate::resolved::Prim::Div,
                lhs: p,
                rhs: q,
            },
            Ty::int64(),
        );
        let trapping_list = synth_core(
            &mut db,
            Core::ListNew {
                elems: [div].into(),
            },
            list_int.clone(),
        );
        let body = synth_core(
            &mut db,
            Core::ListConcat {
                lhs: trapping_list,
                rhs: trapping_list,
            },
            list_int,
        );
        assert!(
            !crate::lower::is_trap_free(&mut db, trapping_list),
            "a ListNew carrying a runtime divide is not trap-free (test premise)"
        );
        let plan = b2_bind_plan(&mut db, body);
        assert!(
            plan.iter().any(|e| e.shared_id == trapping_list),
            "a trapping param-closed share unconditionally reached by its scope_node IS planned (gate-4 reach arm)"
        );
    }

    // GATE P1 (fully-solved type, v-rb bind-safety whitelist): a shared heap node whose type is NOT fully
    // solved (a Deferred int axis — the shape the Record.with drift-guard shares had) is EXCLUDED, because
    // the slot binding would fix an indeterminate valtype → 'no local slot' emit failure. The SAME node
    // shape with a Fixed-width type IS planned (control), isolating the type-solvedness as the delta.
    #[test]
    fn bind_plan_excludes_a_share_with_an_unsolved_type() {
        // Deferred-int element list = an unsolved type (the drift-guard Record.with shape).
        let deferred = Ty::Int(crate::ty::IntTy::deferred());
        assert!(
            !deferred.is_fully_solved(),
            "test premise: deferred int is unsolved"
        );
        assert!(Ty::int64().is_fully_solved(), "test premise: i64 is solved");
        let build = |solved: bool| -> bool {
            let mut db = crate::db::Db::load(crate::testkit::parse(
                "(module m (def (main) 0) (export main))",
            ));
            let elem = if solved {
                Ty::int64()
            } else {
                Ty::Int(crate::ty::IntTy::deferred())
            };
            let list_ty = Ty::List(Box::new(elem));
            let x = synth_core(&mut db, Core::ListNew { elems: [].into() }, list_ty.clone());
            let body = synth_core(&mut db, Core::ListConcat { lhs: x, rhs: x }, list_ty);
            let plan = b2_bind_plan(&mut db, body);
            plan.iter().any(|e| e.shared_id == x)
        };
        assert!(
            build(true),
            "control: a solved-type shared heap ListNew IS planned"
        );
        assert!(
            !build(false),
            "a shared heap node with an unsolved (Deferred-int) type is excluded (P1)"
        );
    }

    // P3-NARROW, EXCLUDE arm (wasm-consuming): a shared list that is a MatchList scrutinee is EXCLUDED —
    // a MatchList rest pattern lowers its tail binder to `SumPayload{RestFrom}` = a CONSUMING `vec-split`
    // (FBIP), so binding the list once and reading the slot K times is a use-after-free → wasm OUT-OF-BOUNDS
    // (the TWO-LIST FIFO opt-sweep divergence). Even though rust would resolve a slotted list scrutinee, the
    // BOTH-BACKEND-safe set excludes MatchList. The narrow admits ONLY Tuple/Record single-`Elem` reads.
    #[test]
    fn bind_plan_excludes_a_matchlist_scrutinee_share() {
        let mut db = crate::db::Db::load(crate::testkit::parse(
            "(module m (def (main) 0) (export main))",
        ));
        let list_int = Ty::List(Box::new(Ty::int64()));
        let xs = synth_core(
            &mut db,
            Core::ListNew { elems: [].into() },
            list_int.clone(),
        );
        let empty_arm = synth_core(
            &mut db,
            Core::ListNew { elems: [].into() },
            list_int.clone(),
        );
        let matched = synth_core(
            &mut db,
            Core::MatchList {
                scrutinee: xs,
                arms: [crate::core::ListArm {
                    cond: crate::core::ListArmCond::Any,
                    guard: None,
                    body: empty_arm,
                }]
                .into(),
            },
            list_int.clone(),
        );
        let body = synth_core(
            &mut db,
            Core::ListConcat {
                lhs: matched,
                rhs: xs,
            },
            list_int,
        );
        assert!(
            b2_is_dispatched_on(&mut db, body, xs),
            "test premise: xs is a MatchList scrutinee reachable from body"
        );
        assert!(
            !b2_dispatch_rust_slot_safe(&mut db, body, xs),
            "a MatchList scrutinee is NOT both-backend-safe (consuming vec-split → wasm OOB)"
        );
        let plan = b2_bind_plan(&mut db, body);
        assert!(
            !plan.iter().any(|e| e.shared_id == xs),
            "a MatchList-scrutinee share is EXCLUDED (P3-narrow, both-backend)"
        );
    }

    // P3-NARROW, EXCLUDE arm (the retained safety property): a shared heap node read as a SUM-VARIANT
    // scrutinee (here 2 SumExpect unwraps) is STILL EXCLUDED — the rust nested-variant resolver needs a
    // bind minted by a MatchSum arm on the DIRECT scrutinee, so a slotted LocalRef scrutinee has no bind
    // and declines. This is exactly the shape P3 was originally added for.
    #[test]
    fn bind_plan_excludes_a_sum_variant_scrutinee_share() {
        let mut db = crate::db::Db::load(crate::testkit::parse(
            "(module m (def (main) 0) (export main))",
        ));
        let decl = synth_id_marker(&mut db);
        let sum_ty = Ty::Sum {
            decl,
            args: vec![Ty::int64()].into(),
        };
        let p = synth_param(&mut db);
        let shared = synth_core(
            &mut db,
            Core::SumNew {
                disc: 0,
                payloads: vec![p].into(),
            },
            sum_ty,
        );
        let r1 = synth_core(
            &mut db,
            Core::SumExpect {
                scrutinee: shared,
                disc_present: 0,
            },
            Ty::int64(),
        );
        let r2 = synth_core(
            &mut db,
            Core::SumExpect {
                scrutinee: shared,
                disc_present: 0,
            },
            Ty::int64(),
        );
        let body = synth_core(
            &mut db,
            Core::Arith {
                op: crate::resolved::Prim::Add,
                lhs: r1,
                rhs: r2,
            },
            Ty::int64(),
        );
        assert!(
            b2_is_dispatched_on(&mut db, body, shared),
            "test premise: the share is a SumExpect scrutinee reachable from body"
        );
        assert!(
            !b2_dispatch_rust_slot_safe(&mut db, body, shared),
            "a SumExpect (sum-variant) scrutinee is rust-slot-UNSAFE"
        );
        let plan = b2_bind_plan(&mut db, body);
        assert!(
            !plan.iter().any(|e| e.shared_id == shared),
            "a sum-variant scrutinee share is EXCLUDED (P3-narrow retains this)"
        );
    }

    // P3-NARROW, ADMIT arm (the cmb1 shape): a shared heap TUPLE read via SumPayload paths that are all
    // `Elem` steps (tuple/record dot-index) is RUST-SLOT-SAFE — rust resolves each field DIRECTLY off the
    // (slotted) scrutinee, no MatchSum bind — so it is ADMITTED for bind-once. cmb1's 127 state-tuple shares
    // are exactly this shape (v-rb measured bind-once of them = both-backend 1 PASS).
    #[test]
    fn bind_plan_admits_a_tuple_elem_payload_share() {
        let mut db = crate::db::Db::load(crate::testkit::parse(
            "(module m (def (main) 0) (export main))",
        ));
        let p = synth_param(&mut db);
        let tup_ty = Ty::Tuple(vec![Ty::int64(), Ty::int64()].into());
        let t = synth_core(
            &mut db,
            Core::Tuple {
                elems: [p, p].into(),
            },
            tup_ty,
        );
        let r0 = synth_core(
            &mut db,
            Core::SumPayload {
                scrutinee: t,
                path: vec![crate::core::PathStep::Elem(0)].into(),
            },
            Ty::int64(),
        );
        let r1 = synth_core(
            &mut db,
            Core::SumPayload {
                scrutinee: t,
                path: vec![crate::core::PathStep::Elem(1)].into(),
            },
            Ty::int64(),
        );
        let body = synth_core(
            &mut db,
            Core::Arith {
                op: crate::resolved::Prim::Add,
                lhs: r0,
                rhs: r1,
            },
            Ty::int64(),
        );
        assert!(
            b2_is_dispatched_on(&mut db, body, t),
            "test premise: the tuple is a SumPayload scrutinee reachable from body"
        );
        assert!(
            b2_dispatch_rust_slot_safe(&mut db, body, t),
            "a tuple read via Elem-only SumPayload paths is rust-slot-safe (dot-index off the local)"
        );
        let plan = b2_bind_plan(&mut db, body);
        assert!(
            plan.iter().any(|e| e.shared_id == t),
            "a rust-slot-safe tuple/Elem-path scrutinee share is ADMITTED for bind-once (cmb1 shape)"
        );
    }
}
