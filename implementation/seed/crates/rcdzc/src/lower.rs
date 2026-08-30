//! `lower` — the query that fills the core column: for a node's `StructId`, its A-normal [`Core`]
//! form.
//!
//! One concern: lowering the resolved tree to the A-normal core. [`core_of`] reads a node's resolved
//! form (via [`crate::resolve::resolved_of`]) and produces its core form, memoizing into `db.core`;
//! it is the ONLY module that fills that column. Where a lowering decision needs the solved type it
//! READS it (via [`crate::infer::type_of`]) rather than recomputing one (`reference-compiler.md`
//! §One Pass Owns One Concern — architecture, not duvet-cited). Lowering DOES branch on the solved
//! type: a comparison folds constants but stays runtime for a scalar parameter, a match classifies
//! its scrutinee, and a runtime compound's value form is templated off its `Ty`.
//!
//! Lowering is mostly a structural map — a literal → a `Const*`, an `if` → a core `If` on the same
//! child ids (lowered on their own demand) — with three constructs that name intermediate values:
//! [`lower_let`] A-normalizes a multi-use runtime `let` into `Core::Let`/`LocalRef` (a single-use or
//! constant `let` is erased by copy-propagation), a lambda application β-reduces and lowers its result
//! (so a non-recursive call monomorphizes away), and a recursive call whose callee has a determined
//! signature lowers to a real `Core::Call`. The core's own fresh-id space is still unneeded: every
//! binding it introduces is keyed by an existing source occurrence.

use crate::arena::Slot;
use crate::ast::{CompoundCtor, IntValue, StructId};
use crate::core::Core;
use crate::db::Db;
use crate::diag::{Code, Fix, Reject};
use crate::resolve::resolved_of;
use crate::resolved::{Prim, Resolved};
use tracing::trace;

mod ast_reflect;
// The AST-reflection intrinsic lowerings live in `ast_reflect`; re-import the ones `compute` (and the
// test module) call by bare name so those call sites read unchanged.
use ast_reflect::{
    ast_variant_discs, const_byte_slice, lower_ast_decode, lower_ast_encode, lower_ast_lift,
    lower_ast_splice_lift, lower_blake3_of, lower_print, lower_read, reflect_module_value,
};
// Re-exported at crate scope so `crate::lower::is_ast_float_variant` (the rust backend's escape guard)
// keeps resolving to its new home.
pub(crate) use ast_reflect::is_ast_float_variant;
#[cfg(test)]
use ast_reflect::{SNode, SexprReader, parse_bigint_decimal};

mod match_desugar;
// Match & pattern lowering/desugaring lives in `match_desugar`. Its items are glob-re-exported into
// `lower` so `compute` (and the sibling modules, via their own `use super::*`) reach them by bare name
// exactly as before the split; the two `sroa_*` helpers are additionally re-exported at crate scope so
// `crate::lower::sroa_*` (referenced from the crate's test file) keeps resolving.
pub(super) use match_desugar::*;
#[cfg(test)]
pub(crate) use match_desugar::{sroa_let_wrap, sroa_tuple_scrutinee_candidate};

mod compute;
// The core lowering dispatcher lives in `compute`; `core_of` calls it by bare name.
use compute::compute;

mod value_form;
// Runtime/constant value-form encoding lives in `value_form`. A `pub` glob re-export preserves each
// item's original visibility (its many `pub` fns/types stay reachable as `crate::lower::*`, the private
// ones stay `pub(super)`), so both cross-crate paths and bare-name uses across the tree are unchanged.
pub use value_form::*;

mod match_tree;
// The match decision-tree compiler + exhaustiveness checking lives in `match_tree`. Its most-visible
// item is `pub(crate)` (`check_binding_pattern`), so a `pub(crate)` glob re-export preserves each item's
// visibility (pub(crate) stays crate-reachable as `crate::lower::*`, private items stay `pub(super)`).
pub(crate) use match_tree::*;

mod call_lower;
// Call/application lowering (spine, eta, lambda-lift, inline heuristics, specialization, binding-uses)
// lives in `call_lower`; its most-visible items are `pub(crate)` (`runtime_fn_spine`/`apply_spine`), so
// the same `pub(crate)` glob re-export preserves each item's visibility across the tree.
pub(crate) use call_lower::*;

mod const_eval;
// The general compile-time constant evaluator lives in `const_eval`; all its items are `pub(super)`
// (nothing crosses the crate boundary), so a plain private glob re-import brings them into `lower`'s
// scope by bare name — and sibling modules pick them up through their own `use super::*`.
use const_eval::*;

mod bin_match;
// Binary (`bin`) construction + pattern matching lives in `bin_match`; all module-private, so the same
// plain private glob re-import makes them reachable by bare name across the tree.
use bin_match::*;

mod runtime_ops;
// Runtime collection/string/record/tuple/bytes/arith op lowerings live in `runtime_ops`; its most-visible
// item is `pub(crate)` (`is_const_value`), so a `pub(crate)` glob re-export preserves each item's
// visibility (pub(crate) stays `crate::lower::*`-reachable, the rest stay pub(super)).
pub(crate) use runtime_ops::*;

/// A LAMBDA-LIFTED closure — a `(fn (param…) body)` that survived lowering as a runtime value and was
/// hoisted to a standalone wasm function. Its position in `db.lifted` is its funcref-TABLE slot. Every
/// closure is UNIFORMLY an `(env, param…) -> result` function (so a function-typed parameter can hold a
/// capturing OR a non-capturing closure interchangeably): the closure CELL (`box-int(code)` + captures)
/// is passed as the env at local slot 0, and the lambda's own parameters at slots 1.. . A body reference
/// to a lambda parameter is a `Core::Param`; a reference to a CAPTURED free variable is a
/// `Core::Captured { index }` that reads the env cell (recorded per-occurrence in `db.captured_ref`).
/// The param + result machine types come from the lambda's body. A MULTI-parameter lambda is supported
/// only when applied at FULL arity (a `(g a b)` application → one `call_indirect` with all args); a
/// partial application of a runtime multi-param closure (runtime currying) still declines.
/// `DESIGN-runtime-closures-rcdzc.md` §3.
#[derive(Clone, PartialEq, Debug)]
pub struct LiftedLambda {
    /// The lambda's `body` occurrence — the identity that dedups a lambda lifted more than once, and
    /// what the backend selects as the function body.
    pub body: StructId,
    /// The lambda's parameters, in order — each `(binder occurrence, solved machine type)`. They occupy
    /// wasm local slots `1..1+params.len()` (slot 0 is the env cell).
    pub params: Vec<(StructId, crate::ty::Ty)>,
    /// The result's solved machine type.
    pub ret_ty: crate::ty::Ty,
    /// The captured free variables, in cell order (cell index `1 + position`). Each is the binder
    /// occurrence of the enclosing binding the lambda body references. Empty for a combinator.
    pub captures: Vec<StructId>,
}

/// The core (A-normal) form of the node at `id`, filling the column on demand (memoized). Reads the
/// resolved form; children stay ids, lowered on their own demand.
/// If any of `elems` lowers to a REDUCTION-BOUND poison (`Code::RecursionBound`), return that poison so a
/// compound construction (tuple / list / record value) built around it COLLAPSES to the poison rather than
/// surviving as a giant `Core::Tuple`/`ListNew`/`Record`. A non-normalizing self-application in a compound
/// slot — `((fn v (tuple (v v) 1)) (fn v (tuple (v v) 1)))` — β-reduces (bounded by `REDUCE_NODE_BUDGET`)
/// into a memoized core chain thousands of `Core::Tuple` levels deep, bottoming out in that poison; without
/// this collapse, every downstream structural walk over the core tree (`layout::collect_call_callees`,
/// `compile::collect_reached_poisons`, emit/serialize) would descend the whole chain in native recursion
/// and OVERFLOW THE COMPILER'S STACK on a small valid-to-parse program. Collapsing at the source — the ONE
/// place the compound is built — stops that at every consumer at once, not walk-by-walk.
///
/// Only `RecursionBound` (a resource-limit poison — the reduction could not finish) collapses; a `ConstTrap`
/// element does NOT, preserving dead-code elimination (`(. (tuple 42 (/ 100 0)) 0)` projects away its
/// trapping element and warns CDZ0305, never rejecting). A `RecursionBound` element is not DCE-eligible: the
/// compiler did not PROVE the computation traps, it ran out of budget, so the enclosing compound genuinely
/// cannot be built and the poison must propagate.
fn reduction_bound_element(db: &mut Db, elems: &[StructId]) -> Option<Reject> {
    for &e in elems {
        if let Core::Poison(r) = core_of(db, e)
            && r.code == Some(Code::RecursionBound)
        {
            return Some(r);
        }
    }
    None
}

/// Lower one node's resolved form to its core form. Records fold: a bare name is its bound value's
/// core, a `let` is its body's core, and a member projection is the FIELD'S core read directly — so a
/// record used only to read a field leaves no runtime trace (it folds to the projected scalar). A
/// record used as a runtime value survives as `Core::Record` (which declines at select until the
/// value heap exists). This is the one compile-time reduction tier acting through lowering
/// (`reference-compiler.md` §A Construct Whose Value Is Fully Determined At Compile Time).
pub fn core_of(db: &mut Db, id: StructId) -> Core {
    // A pass-installed CORE OVERRIDE (the `crate::opt` `CorePass` seam) wins over the lowered form, so a
    // backend-independent optimization rewrites the Core-IR once and both backends inherit it
    // (`DESIGN-tiered-optimization-levels-rcdzc.md` §9a). Checked FIRST — before the memoized column — so an
    // override installed after the column was filled still takes effect. EMPTY in the default pipeline, so
    // this is a single O(1) miss and `core_of` is byte-identical to before the seam.
    if !db.core_override.is_empty()
        && let Some(c) = db.core_override.get(&id)
    {
        trace!(target: "rcdzc::lower", node = id.0, "core override hit");
        return c.clone();
    }
    if let Slot::Filled(c) = db.core.get(id) {
        trace!(target: "rcdzc::lower", node = id.0, "memo hit");
        return c.clone();
    }
    // A CAPTURED-reference occurrence (a body reference to a free variable of a lifted lambda) reads the
    // env cell — `Core::Captured` — rather than following the ref to its (out-of-scope) binding. Recorded
    // by `lower_lambda_value` when the lambda was lifted; keyed by this reference occurrence. Checked
    // before the ordinary resolved dispatch so the ref is not followed. (Not memoized into the column
    // here — it is filled below like any node — but the map lookup is O(1) and the value is stable.)
    if let Some(&(index, ref ty)) = db.captured_ref.get(&id) {
        let c = Core::Captured {
            index,
            ty: ty.clone(),
        };
        db.core.fill(id, c.clone());
        return c;
    }
    // Recursive-descent DEPTH GUARD. `compute` re-enters `core_of` for a node's sub-expressions, so a
    // pathologically deep nest (`(+ 1 (+ 1 …))` thousands deep) or an unproductive self-recursion a
    // nullary call re-enters (`(def (f) (f))`) would recurse until the native stack overflows and the
    // PROCESS ABORTS. Past `LOWER_DEPTH_LIMIT` decline (a resource-limit poison) instead — a compiler
    // must never crash on well-formed input, only decline or complete (decline-don't-miscompile). This
    // result is NOT memoized: the same node lowered from a shallower context (below the limit) must
    // still get its real core, so the decline is specific to this over-deep demand, not the node.
    if db.descent_depth >= crate::db::DESCENT_DEPTH_LIMIT {
        trace!(target: "rcdzc::lower", node = id.0, "lowering depth limit hit → decline (resource limit)");
        return Core::Poison(Reject::decline(
            "expression nests too deeply to compile (a recursion/resource limit was reached)",
        ));
    }
    db.descent_depth += 1;
    let c = compute(db, id);
    db.descent_depth -= 1;
    trace!(target: "rcdzc::lower", node = id.0, core = ?c, "lowered");
    db.core.fill(id, c.clone());
    c
}

/// Whether the core form at `id` reaches a `Core::HostCall` (directly or nested) — a bounded structural
/// walk over the core tree. Used by the `do`-sequencing lowering to decide whether a non-final statement
/// has a host-call side effect that must be emitted (rather than dropped by the ordinary `Ref{last}` fold).
/// Also read by `compile::collect_discarded_value_warnings` so the CDZ0307 "discarded value" warning fires
/// on EXACTLY the pure non-final statements this lowering drops — one predicate, no drift between the DCE
/// decision and the diagnostic that explains it.
pub(crate) fn subtree_reaches_host_call(db: &mut Db, id: StructId) -> bool {
    // MEMOIZED (`Db::reaches_host_call`): a pure function of the node's fixed AST/core structure. This is
    // called from TWO hot passes over OVERLAPPING subtrees — the `do`/`let` lowering (per do-block) and
    // `collect_discarded_value_warnings` (per non-final statement) — and each call re-walked the WHOLE
    // subtree calling `core_of` per node. On the real corpus workload the pair was ~45% of compile time;
    // the memo makes a node's verdict O(1) after first computation (so a statement queried by both passes,
    // and overlapping do-statement subtrees, pay the walk once).
    if let Some(&cached) = db.reaches_host_call.get(&id) {
        return cached;
    }
    let result = match db.ast.get(id).clone() {
        // A `Core::HostCall` only ever lowers from an APPLICATION `(head args)` — a `Struct::List` (a
        // host-delegated effect-op perform). So only a List node need be lowered (`core_of`) to test for
        // it; an ATOM (a name/literal) can never be a host call, so skip the (per-node, per-compile)
        // `core_of` lowering entirely for it. This is what dominated the real corpus workload: the old code
        // forced `core_of` on EVERY node — including every leaf atom — of every do-statement subtree.
        crate::ast::Struct::List(children) => {
            matches!(core_of(db, id), Core::HostCall { .. })
                || children.iter().any(|&c| subtree_reaches_host_call(db, c))
        }
        crate::ast::Struct::Atom(_) => false,
    };
    db.reaches_host_call.insert(id, result);
    result
}

/// Whether a tuple-projection operand is (transitively, through nested projections) a CAPTURED binding of
/// an enclosing closure — its base occurrence reads the env cell (`db.captured_ref`). Used to keep the
/// projection a runtime `Core::Proj` instead of folding it to the tuple ELEMENT: reducing through a
/// captured `let`-bound tuple would inline an enclosing binding as a slot-less `Core::Param` in the lifted
/// closure body. A NESTED projection `(. (. a 0) 0)` has a `Resolved::Proj` operand whose own operand is
/// the captured `a`, so recurse through the projection chain to its base.
fn proj_operand_reaches_capture(db: &mut Db, operand: StructId) -> bool {
    if db.captured_ref.contains_key(&operand) {
        return true;
    }
    match resolved_of(db, operand) {
        Resolved::Proj { operand: inner, .. } if inner != operand => {
            proj_operand_reaches_capture(db, inner)
        }
        _ => false,
    }
}

/// A-normalize a `let`: decide, per binding, whether to NAME its value (keep it as a `Core::Let`
/// binding computed once) or to COPY-PROPAGATE / erase it (let each reference follow through to the
/// value's core). A binding is KEPT iff its value is a runtime computation (not a compile-time
/// constant that folds away) AND its name is referenced MORE THAN ONCE in what follows — the case
/// where following through would recompute the value at each use. Every other binding — used at most
/// once, or constant — is propagated, the admin-redex elimination that keeps naming free
/// (`reference-compiler.md` §The Core Representation Is In A-Normal Form ¶3), so a program with no
/// multi-use runtime binding lowers exactly as before and its emitted bytes are unchanged.
///
/// The kept bindings are recorded in `db.kept_bindings` (keyed by the initializer occurrence a
/// reference resolves to) BEFORE the body is lowered, so a `Resolved::Ref` to a kept binding lowers
/// to a `Core::LocalRef` reading the shared slot. The result is a `Core::Let { bindings, body }` when
/// any binding is kept, or just the body's core when none is (no residual `let`).
/// Lower a RUNTIME (non-constant, non-poison) `String.byte-len` / `String.scalar-len` over `operand`. A
/// runtime String is a flat UTF-8 byte leaf, so `byte-len` reads that leaf's length (`Core::BytesLen`,
/// the `bytes-len` op) and `scalar-len` WALKS the leaf counting UTF-8 lead bytes (`Core::StrScalarLen`,
/// reusing `String.at`'s scalar-scan over the exported `bytes-len`/`bytes-get` — HASH-NEUTRAL). A non-
/// String operand (only reachable if the surface admitted a length op on a non-string) declines.
///
/// `#[inline(never)]`: called from `compute`, the per-descent-level recursive core-computation whose stack
/// frame is on the hot path of a deeply-nested reduction (the 1024-deep poison-chain tests). The `type_of`
/// probe materializes a `Ty` temporary; keeping it in THIS frame (not `compute`'s) preserves the compile
/// thread's stack headroom so a diverging reduction still declines at the depth guard instead of crashing.
#[inline(never)]
fn lower_runtime_str_len(
    db: &mut Db,
    prim: crate::resolved::Prim,
    operand: StructId,
    id: StructId,
) -> Core {
    use crate::resolved::Prim;
    if !matches!(crate::infer::type_of(db, operand), crate::ty::Ty::String) {
        return Core::Poison(Reject::decline(
            "a runtime string's scalar length needs a UTF-8 decoding walk (not yet built; byte-len works)",
        ));
    }
    match prim {
        Prim::StrByteLen => {
            trace!(target: "rcdzc::lower", node = id.0, "String.byte-len on a runtime string → bytes-len over its byte leaf");
            Core::BytesLen { operand }
        }
        Prim::StrScalarLen => {
            trace!(target: "rcdzc::lower", node = id.0, "String.scalar-len on a runtime string → UTF-8 lead-byte count walk");
            Core::StrScalarLen { operand }
        }
        _ => Core::Poison(Reject::decline(
            "a runtime string's scalar length needs a UTF-8 decoding walk (not yet built; byte-len works)",
        )),
    }
}

/// The source DISPLAY NAME of a variant-constructor head — a bare name (`Wrap` → `"Wrap"`) or a member
/// form `(. Sum V)` (→ `"V"`, the variant it names). Used to NAME the constructor in the under-application
/// diagnostic (`` `Wrap` needs its payload argument ``), the lower-side twin of `infer`'s
/// `variant_ctor_head_name`. `None` for a head with no readable name (an anonymous / synthesized head),
/// where the caller falls back to the un-named phrasing.
fn ctor_head_display_name(db: &Db, head: StructId) -> Option<String> {
    if let Some(n) = db.ast.as_name(head) {
        return Some(n.to_string());
    }
    // A member `(. Sum V)` head — the ctor is its KEY (second child after the `.`), a bare name.
    let tail = db.ast.as_form(head, ".")?;
    tail.get(1)
        .and_then(|&k| db.ast.as_name(k))
        .map(str::to_string)
}

/// The binding use-facts (`BindingUses`) covering the `let` region rooted at `node`. Reuses the nearest
/// ENCLOSING cached let-region's facts when there is one (sound: `let*` scoping confines a binding's
/// references to its own sub-region, so the outer region's counts/escapes read identically for a nested
/// binding), otherwise collects this region in ONE walk and caches it (`Db::let_region_uses`). Together
/// these make a DEEP nested chain O(N) total: the OUTERMOST let walks the whole nest once, every inner let
/// reuses that map in O(1). Returns an `Rc` shared across the nest.
fn enclosing_or_collected_region_uses(
    db: &mut Db,
    node: StructId,
    bindings: &[(StructId, StructId)],
    body: StructId,
) -> std::rc::Rc<BindingUses> {
    // Already collected for THIS node (a memoized re-entry)? (Scope the borrow — the arms below take `&mut
    // db` / `borrow_mut`, so never hold a `borrow()` across them.)
    if let Some(hit) = db.let_region_uses.borrow().get(&node).cloned() {
        return hit;
    }
    // Walk up to the nearest ANCESTOR `let` node; if its region is already cached, its whole-region facts
    // cover this nested region exactly (an inner binding's refs are a subset of the outer region), so reuse
    // it. The walk stops at the first `let` ancestor — O(1) for a nested chain (an inner let's parent IS the
    // enclosing let), and bounded by the local non-let nesting otherwise.
    let mut cur = db.parent_of(node);
    while let Some(p) = cur {
        if matches!(resolved_of(db, p), Resolved::Let { .. }) {
            let cached = db.let_region_uses.borrow().get(&p).cloned();
            if let Some(rc) = cached {
                // Alias THIS node to the enclosing region too, so a demand keyed by `node` is also O(1).
                db.let_region_uses.borrow_mut().insert(node, rc.clone());
                return rc;
            }
            // A let ancestor with no cached region — stop (we are the outermost still-uncollected; collect).
            break;
        }
        cur = db.parent_of(p);
    }
    // No enclosing cached region — collect this region's facts in one walk and cache them.
    let mut uses = BindingUses::default();
    for &(_name_occ, v) in bindings.iter() {
        collect_binding_uses(db, v, false, &mut uses);
    }
    collect_binding_uses(db, body, false, &mut uses);
    let rc = std::rc::Rc::new(uses);
    db.let_region_uses.borrow_mut().insert(node, rc.clone());
    rc
}

/// Operator ruling (A) STRICT heap-collection construction (#5194): a REACHED list/set/map CONSTRUCTOR MUST
/// evaluate every element ARGUMENT (its traps + effects occur) even when the resulting collection is dead /
/// discarded — the optimizer MAY elide the ALLOCATION but MUST preserve argument evaluation. Returns true iff
/// `init`'s core IS such a ctor holding an element that is NOT `is_trap_free` (a trap-possible OR effectful
/// arg). Used as the `const_compound_eq` fold-guard — decline the first-differing-element short-circuit for
/// a list/set/map operand so the runtime value-eq walk MATERIALIZES it (its args evaluate → the trap/effect
/// occurs). SPECIFIC to heap-COLLECTION ctors: a TUPLE/RECORD stays LAZY (its eq short-circuits at the first
/// differing element per §283 / 03-equality "a later trapping element is not forced"), so those are EXCLUDED.
fn is_strict_heap_ctor_with_trappable_arg(db: &mut Db, init: StructId) -> bool {
    match core_of(db, init) {
        Core::ListNew { elems } | Core::SetOf { elems, .. } => {
            let es: Vec<StructId> = elems.iter().copied().collect();
            es.into_iter().any(|e| value_ctor_can_trap(db, e))
        }
        Core::MapNew { entries, .. } => {
            let ents: Vec<(StructId, StructId)> = entries.iter().copied().collect();
            ents.into_iter()
                .any(|(k, v)| value_ctor_can_trap(db, k) || value_ctor_can_trap(db, v))
        }
        _ => false,
    }
}

/// Whether constructing `id` can TRAP. A refinement of `is_trap_free` that recurses THROUGH the heap-
/// COLLECTION ctors `is_trap_free` treats conservatively (its `_ => false` arm makes `SetOf`/`MapNew`
/// "possibly trapping" even when fully constant): a map/set/list/tuple/record/sum is trap-possible iff SOME
/// element/entry/payload is, recursively; a non-ctor leaf falls back to `!is_trap_free`. Without this,
/// `is_strict_heap_ctor_with_trappable_arg` spuriously fires on a CONSTANT list-of-maps (each `MapNew`
/// element reads as `!is_trap_free` = true) and wrongly declines the sound `const_compound_eq` fold →
/// routing a well-typed constant comparison through the runtime walk (regressed 05-compound-types "a list of
/// maps with different keys is homogeneous").
fn value_ctor_can_trap(db: &mut Db, id: StructId) -> bool {
    match core_of(db, id) {
        Core::ListNew { elems } | Core::SetOf { elems, .. } | Core::Tuple { elems } => {
            let es: Vec<StructId> = elems.iter().copied().collect();
            es.into_iter().any(|e| value_ctor_can_trap(db, e))
        }
        Core::MapNew { entries, .. } => {
            let ents: Vec<(StructId, StructId)> = entries.iter().copied().collect();
            ents.into_iter()
                .any(|(k, v)| value_ctor_can_trap(db, k) || value_ctor_can_trap(db, v))
        }
        Core::Record { fields } => {
            let fs: Vec<StructId> = fields.values().copied().collect();
            fs.into_iter().any(|v| value_ctor_can_trap(db, v))
        }
        Core::SumNew { payloads, .. } => {
            // Copy to an owned Vec (releasing the `payloads` borrow) before the `&mut db` calls in `.any`.
            let ps: Vec<StructId> = payloads.to_vec();
            ps.into_iter().any(|p| value_ctor_can_trap(db, p))
        }
        // A non-collection leaf: trap-possible iff not trap-free.
        _ => !is_trap_free(db, id),
    }
}

/// Whether `ty` is a MACHINE SCALAR (Int/Bool/Char/Float/Unit) — a value with no heap cell / refcount. A
/// force-evaluated strict-construction arg of this type leaves a bare scalar on the stack that a plain wasm
/// `drop` reclaims (no rc), so restricting CASE2's decomposition to scalar-typed trap computations keeps the
/// reclaim trivial (no rc-drop, no double-free) — the sound interim subset.
fn is_machine_scalar_ty(ty: &crate::ty::Ty) -> bool {
    matches!(
        ty.strip_nominal(),
        crate::ty::Ty::Int(_)
            | crate::ty::Ty::Bool
            | crate::ty::Ty::Char
            | crate::ty::Ty::Float(_)
            | crate::ty::Ty::Unit
    )
}

/// (A) STRICT heap-collection construction (#5194) CASE2 decomposition: collect the TRAP-possible SCALAR
/// element-arg COMPUTATIONS of a dead list/set/map ctor, recursing THROUGH nested value ctors. Per
/// v-spec-oracle, (A) requires evaluating ONLY the arg computations that can TRAP/perform — NOT building the
/// collection: an already-value / BORROWED leaf (a param/capture/constant) raises no trap (`is_trap_free`),
/// contributes nothing, and is left UNTOUCHED (never consumed → never dropped → no double-free). A
/// host-reaching (effectful) arg is force-kept by the existing machinery, so it is skipped here. INTERIM:
/// restricted to SCALAR-typed trap computations (`(/ 5 d)`, checked arith) — their discarded results are
/// scalars (a plain stack drop, no rc), so there is NO reclaim question; a heap-producing trap arg
/// (`Rational.of`) is a KNOWN gap (its trap is missed) pending the reclaim shape (v-mem).
fn collect_trap_scalar_args(db: &mut Db, id: StructId, out: &mut Vec<StructId>) {
    match core_of(db, id) {
        Core::ListNew { elems } | Core::SetOf { elems, .. } | Core::Tuple { elems } => {
            let es: Vec<StructId> = elems.to_vec();
            for e in es {
                collect_trap_scalar_args(db, e, out);
            }
        }
        Core::MapNew { entries, .. } => {
            let ents: Vec<(StructId, StructId)> = entries.to_vec();
            for (k, v) in ents {
                collect_trap_scalar_args(db, k, out);
                collect_trap_scalar_args(db, v, out);
            }
        }
        Core::Record { fields } => {
            let fs: Vec<StructId> = fields.values().copied().collect();
            for v in fs {
                collect_trap_scalar_args(db, v, out);
            }
        }
        Core::SumNew { payloads, .. } => {
            let ps: Vec<StructId> = payloads.to_vec();
            for p in ps {
                collect_trap_scalar_args(db, p, out);
            }
        }
        _ => {
            // A trap-possible, PURE (non-host) leaf. Two admissible shapes:
            //  - a MACHINE SCALAR computation (`(/ 5 d)`, checked arith): its discarded result is a bare
            //    scalar (a plain stack drop, no refcount).
            //  - a HEAP-PRODUCING trap computation that is a genuine PRODUCER — `resolved_of` is a
            //    `Resolved::Apply` (a call/op like `Rational.of` / a checked BigInt op), so its result is a
            //    FRESH OWNED value the Seq emit rc-reclaims via `OP_DROP`. Gated on `Resolved::Apply` to
            //    EXCLUDE a borrowed heap leaf — a `LocalRef`/`Param`/projection can be `!is_trap_free` (it
            //    follows a trapping init) yet is BORROWED (owned by its slot); `OP_DROP`-ing it would
            //    double-free. A producer transfers ownership out, so its standalone force + `OP_DROP` is
            //    sound. (Closes the ruling-A interim gap: a dead-discarded ctor with a `Rational.of`/BigInt
            //    trap arg now traps too, not just scalar args.)
            if !is_trap_free(db, id) && !subtree_reaches_host_call(db, id) {
                let ty = crate::infer::type_of(db, id);
                let admit = is_machine_scalar_ty(&ty)
                    || (crate::core_analysis::is_heap_type(&ty)
                        && matches!(resolved_of(db, id), Resolved::Apply { .. }));
                if admit {
                    out.push(id);
                }
            }
        }
    }
}

/// (A) CASE2: wrap `body` in a `Core::Seq` whose DISCARDED statements are the trap-possible SCALAR arg
/// computations decomposed (via `collect_trap_scalar_args`) out of the DEAD heap-collection ctors in
/// `dead_inits`, and MARK them in `db.strict_force_eval` so the backend Seq emit does NOT §283-elide them —
/// the (A)-overrides-§283 rule (v-spec-oracle). Never builds a collection, never touches a borrowed/value
/// element → no reclaim, no double-free. EXCLUDES a lambda-ish init BEFORE any `core_of` (using
/// `should_keep_binding`'s lift-free structural guards), because `core_of`-lowering a dead lambda binding
/// SPECULATIVELY LIFTS it and pollutes `db.captured_ref`, poisoning the body's shared closure fold. Returns
/// `body` unchanged when nothing trap-possible remains.
///
/// SKIPS the whole decompose when the enclosing `let`-form `node` lies within a HANDLER-ARM-TAIL / `#st`-
/// threaded region (`effects::node_in_handler_region`, #5321): the handler tail / `#st`-drop / per-dispatch-
/// reclaim lowering recognizes sequencing via `do` FORMS, NOT `Core::Seq`, so a `Seq` wrapper there perturbs
/// it (olc1/cst1/sga1 leaks). Conformance-neutral: only a RARE pure-trap dead ctor inside a reduced handle
/// then skips strict-eval — a known-gap v-spec-oracle pins, NEVER a miscompile.
fn wrap_body_with_strict_arg_eval(
    db: &mut Db,
    node: StructId,
    dead_inits: &[StructId],
    body: StructId,
) -> StructId {
    // SKIP inside a reduced-handler region (#5321). `mark_handler_region` marks the reduced-handle AST
    // subtree, but `lower_let` also processes lets SYNTHESIZED during lowering (the effects fold's #st-state
    // lets, sunk copies) whose ids post-date the mark and so are NOT in the set — yet they are PARENTED under
    // the marked rewritten body. So walk ANCESTORS (not just `node` itself): if `node` or any ancestor is a
    // marked region node, the let is within a handler region → skip the decompose (else cst1's per-dispatch
    // synthesized #st lets perturb the handler drop, +444 leak). Bounded by the parent chain length.
    let mut anc = Some(node);
    while let Some(a) = anc {
        if crate::effects::node_in_handler_region(db, a) {
            return body;
        }
        anc = db.parent_of(a);
    }
    // SKIP the whole decompose when the `let` BODY contains a CAPTURING lambda. A closure can CAPTURE a
    // (dead-looking) ctor binding — `(let ((s #set(n (+ n 1)))) (let ((f (fn (k) (Set.contains s k)))) …))`
    // (clk2): a single-use closure copy-propagates, inlining the captured `s` into the lifted closure body,
    // so `s` is NOT a clean dead-discard. Decomposing its trap-args into an enclosing `Seq` while the closure
    // also builds `s` collides with the closure-lift slot/scratch setup → invalid wasm. `body_captures_free_
    // runtime_name` is the SAME lift-free lexical walk `should_keep_binding` uses (it never `core_of`-lowers,
    // so it cannot pollute `captured_ref`). Conformance-neutral: a rare pure-trap dead ctor in a let whose
    // body captures then skips strict-eval — a known-gap, never a miscompile.
    if body_captures_free_runtime_name(db, body, &mut Vec::new()) {
        return body;
    }
    let mut stmts: Vec<StructId> = Vec::new();
    for &init in dead_inits {
        if matches!(resolved_of(db, init), Resolved::Lambda { .. })
            || crate::eval::lambda_body(db, init).is_some()
            || if_or_match_selects_lambda(db, init)
            || compound_contains_lambda(db, init)
            || subtree_reaches_host_call(db, init)
        {
            continue;
        }
        if is_strict_heap_ctor_with_trappable_arg(db, init) {
            collect_trap_scalar_args(db, init, &mut stmts);
        }
    }
    if stmts.is_empty() {
        return body;
    }
    for &s in &stmts {
        db.strict_force_eval.insert(s);
    }
    let ty = crate::infer::type_of(db, body);
    synth_core(
        db,
        Core::Seq {
            stmts: stmts.into(),
            tail: body,
        },
        ty,
    )
}

fn lower_let(
    db: &mut Db,
    node: StructId,
    bindings: &[(StructId, StructId)],
    body: StructId,
) -> Core {
    // The `(binder-name-occ, init-occ)` pairs; a reference resolves to the INIT occurrence, so that is
    // what the body's `Ref`s point at and what a kept binding is keyed by.
    // Collect every binding's use facts (reference count + whole-value escape) in ONE walk of the whole
    // `let` region — all initializer values + the body. `let*` scoping guarantees a reference to `init_k`
    // appears only in the LATER inits + body (an earlier init cannot name a later binding), so a
    // whole-region count equals each binding's exact in-scope count — no over-count. This replaces the
    // per-binding `should_keep_binding` scope walk (each O(region)), which was O(bindings × body) = O(N²)
    // on a wide `let` (the fix-16/18 class).
    //
    // A DEEP nested chain `(let ((v0 …)) (let ((v1 …)) … body))` would re-walk its body — the entire deeper
    // O(N−k) chain — at each of the N levels → O(N²) (the deep-nested twin of the wide case). But an OUTER
    // let's whole-region facts are EXACT for every nested binding too: a binding's references live only from
    // its own init onward (a subset of the outer region), so its count/escape reads identically off the
    // outer map. So reuse the nearest enclosing cached region's map (`Db::let_region_uses`) instead of
    // re-collecting; only a let with NO cached enclosing region walks (once per whole nest → O(N) total).
    // TRY SHORT-CIRCUIT (BRICK 3a): a binding whose INIT is a `?` on a constant FAILURE
    // (`DESIGN-try-operator-rcdzc.md` §4 v1) lowers to a `Core::Break`, short-circuiting the enclosing
    // boundary: the failure value flows out as the boundary body's value, so the whole `let` folds to that
    // break value and the body (+ later bindings) never runs. Sound because a `let` INIT is on the
    // UNCONDITIONAL strict spine (evaluated left-to-right before the body), so the first init that breaks is
    // reached unconditionally; every binding BEFORE it must be pure (its value is discarded when the break
    // fires). GATED by a `Resolved::Try` PRE-FILTER so a non-try init is never lowered early — calling
    // `core_of(init)` before the keep-analysis populates `db.kept_bindings` would perturb a closure/multi-use
    // binding's lowering decision (a memoization-order hazard). Only a `Resolved::Try` init is probed.
    // DESTRUCTURE-LET → single-arm MATCH (v-memory-safety ruling A): a `let` whose binder is a
    // tuple/record/sum DESTRUCTURE pattern (not a bare name) lowers AS a single irrefutable-arm `match`
    // over the init — reusing match's materialize-ONCE scrutinee AND its already-correct heap reclaim. The
    // ordinary let path re-lowered the init at EVERY element `Core::SumPayload` projection (the post-fold
    // live filter `count_localref_reads` counts only `LocalRef`, so a `SumPayload`-of-init kept binding was
    // wrongly DROPPED → re-inlined) → EXPONENTIAL on a non-foldable recursive init (the match→let
    // deep-nesting hang), and a naive keep-fix left a kept HEAP init's drop UNPLACED → OOB. A match
    // materializes the scrutinee once (kills the re-eval) AND places the heap-scrutinee drop correctly —
    // exactly why `(match X ((tuple a b)) …)` is safe+fast for both scalar and heap while the let form
    // OOB'd. The element binders resolve to the SAME `SumPayload{scrutinee: init}` either way, now reading
    // the match's single materialized scrutinee slot. Single destructure binding → one arm (the binder
    // pattern IS the arm pattern, the let body IS the arm body); a multi-binding or bare-name let keeps the
    // ordinary path below (a bare-name binding is A-normalized/kept exactly as before — unaffected).
    // Gated PRECISELY to a TUPLE/RECORD destructure binder — the irrefutable compound destructures whose
    // element binders are `SumPayload`/`Proj` reads of the init. A destructure binder appears in THREE
    // surface forms and the gate must catch ALL of them (the reported hang was the third, missed by an
    // earlier ctor-only gate): the native ctor-LEAF head `#tuple(a b)` (`compound_ctor_leaf`), the legacy
    // STRING-prim head `("tuple" …)` (`compound_ctor_prim`), and — crucially — the NAME head `(tuple a b)` /
    // `(record …)` that the ML/paren surface `let (a,b) = …` desugars to (`head_name`; `compound_ctor*`
    // deliberately reject a name head). An ANNOTATED binder `(: x T)` (head `:`), a bare name, a `bin`/`list`/
    // `map` binder, or a sum-ctor pattern (`(Some x)`) are NOT caught — head_name is `:`/`bin`/`Some`/… ≠
    // tuple|record — so they keep the ordinary path (annotated-binder diagnostics, bin/list/map reclaim, the
    // CDZ0210 refutable-sum reject all unchanged).
    if bindings.len() == 1
        && (matches!(
            db.ast
                .compound_ctor_prim(bindings[0].0)
                .or_else(|| db.ast.compound_ctor_leaf(bindings[0].0)),
            Some(CompoundCtor::Tuple | CompoundCtor::Record)
        ) || matches!(
            db.ast.head_name(bindings[0].0),
            Some("tuple") | Some("record")
        ))
    {
        let (pat, init) = bindings[0];
        trace!(target: "rcdzc::lower", node = node.0, "destructure-let → single-arm match (materialize-once scrutinee + match reclaim)");
        return lower_match(db, init, &[(pat, body)]);
    }
    for (i, &(_name_occ, init)) in bindings.iter().enumerate() {
        if matches!(resolved_of(db, init), Resolved::Try { .. })
            && let Core::Break { value } = core_of(db, init)
        {
            // Every earlier init must be free of an OBSERVABLE EFFECT — its value is discarded when the
            // break fires, so an ordered side effect (a HOST CALL / perform) would be wrongly elided. A
            // host call IS observable (ordered in the trace), so bail to a later brick if any earlier init
            // reaches one. A TRAP is NOT observable when its value is unobserved: the OPERATOR RULED on §283
            // (concierge answer #622465) that "we don't emit the trap unless it's reachable; a detected
            // unreachable trap is a WARNING." The `?` short-circuits its binding's ONLY use, so an earlier
            // (or enclosing) init whose value flows only into the discarded continuation is UNOBSERVED —
            // its trap is ELIDED (the §285 laziness of an unselected branch), NOT forced. So the fold does
            // NOT gate on trap-freedom: `(let ((a (/ 1 0)) (x (try None))) (+ a x))` folds to `None` (with
            // a §285 CDZ0305 SHOULD-diagnose warning that the ÷0 is a detected-unreachable trap), NOT
            // CDZ0304. This makes the same-let, nested-let, and `if false` shapes all consistently elide,
            // aligning with the landed §283 DCE (`dead-binding-drops-a-defined-trap`).
            if bindings[..i]
                .iter()
                .all(|&(_, prev)| !subtree_reaches_host_call(db, prev))
            {
                return core_of(db, value);
            }
        }
    }
    // DIVERGING-INIT SHORT-CIRCUIT: a binding whose INIT is an unconditional `(trap …)` DIVERGES before the
    // binding is ever bound, so the body (and every later binding) is UNREACHABLE — the whole `let` IS the
    // trap. Lower it to the trap init directly. Without this the body is lowered against a binding whose type
    // is the trap's `Never`/`Any` (not a machine type): a `(let ((it (trap …))) (if (> it 0) …))` reaches the
    // comparison lowering, where `is_scalar(it)` reads FALSE (a bottom type is not Int/Bool) and mis-declines
    // "comparison of a compound value needs a heap walk" — a construct that appears nowhere in the program
    // (the check/compile divergence v-verification's reversed `@ensures`/`@requires` desugar surfaced: it
    // emits `(let ((it (trap "@requires…"))) (if (> it 0) it (trap "@ensures…")))` in the precondition-fail
    // branch). Pre-filtered to a `Prim::Trap`-headed init (like the `Resolved::Try` filter above) so a normal
    // binding's lowering decision is never perturbed. Every EARLIER binding must be host-call-free — its value
    // is discarded when the trap diverges, so an ordered host call would be wrongly elided (a trap in an
    // earlier init is itself the diverging point, handled by that earlier iteration; a pure/trapping earlier
    // init is fine — same soundness gate as the Try short-circuit, minus the trap-freedom the operator's §283
    // ruling already exempts). Only the FIRST diverging init short-circuits (the rest is dead).
    for (i, &(_name_occ, init)) in bindings.iter().enumerate() {
        if let Resolved::Apply { head, .. } = resolved_of(db, init)
            && matches!(
                crate::eval::meta_apply_of(db, head).or_else(|| crate::eval::prim_of(db, head)),
                Some(Prim::Trap)
            )
            && bindings[..i]
                .iter()
                .all(|&(_, prev)| !subtree_reaches_host_call(db, prev))
        {
            return core_of(db, init);
        }
    }
    let uses = enclosing_or_collected_region_uses(db, node, bindings, body);
    let mut kept: Vec<(StructId, StructId)> = Vec::new();
    for &(name_occ, init) in bindings.iter() {
        if should_keep_binding(db, name_occ, init, &uses) {
            // Record the keep BEFORE lowering the body/later inits — their references to this init read
            // `db.kept_bindings` to decide `LocalRef` vs follow-through.
            db.kept_bindings.insert(init);
            kept.push((init, init));
        }
    }
    // The body's core (its references to kept bindings now lower to `LocalRef`).
    if kept.is_empty() {
        // Ordinary erase — but (A) CASE2 (#5194): before erasing, force-evaluate the trap-possible scalar
        // arg computations of any dead heap-collection ctor (their traps must occur though the collection is
        // discarded), sequenced before the body. Every binding is dead here.
        let dead: Vec<StructId> = bindings.iter().map(|&(_n, init)| init).collect();
        let wrapped = wrap_body_with_strict_arg_eval(db, node, &dead, body);
        return core_of(db, wrapped);
    }
    // POST-FOLD DEAD-BINDING RECLAIM: `should_keep_binding` decides on the PRE-fold use count, but a use
    // can then FOLD AWAY (e.g. `List.len`/`List.at` over a `let`-bound constant list follow the binder and
    // fold to a constant — the `LocalRef` disappears from the lowered core). A binding all of whose uses
    // folded is now DEAD: the kept slot is read nowhere, yet its init (a pure `ListNew` build) would still
    // be emitted for nothing. Recount LIVE `LocalRef` reads over the now-lowered body AND every kept init
    // (a kept binding can be read by a LATER kept init, not only the body), then drop a binder with ZERO
    // live reads — UNLESS its init reaches a HOST CALL / perform (`subtree_reaches_host_call`), which is
    // OBSERVABLE and must run exactly once even when its value is unread (the same force-keep invariant
    // `should_keep_binding` applies at @host-call). This runs AFTER lowering, so it reads memoized
    // `core_of` results (no re-lowering, no lambda-lift/`captured_ref` hazard) and walks via the
    // maintained `core_child_ids` enumeration (no hand-rolled per-variant walk that could undercount →
    // wrongly drop a live binding). Dropping a dead pure binding changes no observable value (a Perceus
    // drop of an unread list is a net no-op). If every kept binding drops, the `let` erases to its body.
    let live = kept
        .iter()
        .filter(|(binder, _)| {
            let mut n = 0usize;
            // A lowered core can SHARE sub-nodes (a kept binding referenced twice, or a node reachable via
            // both the body and a later kept init), so a plain recursion re-descends the shared DAG as a
            // tree — exponential on a wide share (pom5: a same-field cascade sharing a state-field subtree).
            // The consumer is `n > 0` (presence, not multiplicity), so a visited set is SOUND: visiting a
            // shared node once still finds any reachable `LocalRef` for the binder, giving the identical
            // liveness verdict while making the walk linear and cycle-safe. Shared across the body and every
            // kept init for THIS binder — a node already visited via one has the same fixed verdict.
            let mut seen = std::collections::HashSet::new();
            count_localref_reads(db, body, *binder, &mut n, &mut seen);
            for &(other, _) in kept.iter() {
                count_localref_reads(db, other, *binder, &mut n, &mut seen);
            }
            n > 0 || subtree_reaches_host_call(db, *binder)
        })
        .copied()
        .collect::<Vec<_>>();
    if live.len() < kept.len() {
        // Some kept binding folded dead — un-record it so a stray later reference (there is none by
        // construction, but keep `db.kept_bindings` consistent with the emitted `Core::Let`) does not
        // lower to a `LocalRef` for a slot that no longer exists.
        let mut dropped_inits: Vec<StructId> = Vec::new();
        for (binder, _) in &kept {
            if !live.iter().any(|(b, _)| b == binder) {
                db.kept_bindings.remove(binder);
                dropped_inits.push(*binder);
            }
        }
        trace!(target: "rcdzc::lower", body = body.0, dropped = kept.len() - live.len(), "let: reclaimed dead bindings whose uses all folded");
        // (A) CASE2: force-evaluate the trap-possible scalar args of any DROPPED heap-collection ctor,
        // sequenced before the body (inside the surviving Let, so a live-binding a trap-arg reads is bound).
        // A dropped binding is NON-host-reaching (the `live` filter keeps host-reaching), so no effect moves.
        let body = wrap_body_with_strict_arg_eval(db, node, &dropped_inits, body);
        if live.is_empty() {
            return core_of(db, body);
        }
        return Core::Let {
            bindings: live.into(),
            body,
        };
    }
    trace!(target: "rcdzc::lower", body = body.0, kept = kept.len(), "let: A-normalized (named multi-use runtime bindings)");
    Core::Let {
        bindings: kept.into(),
        body,
    }
}

/// Recursively count the occurrences of `Core::LocalRef { binder }` reading `binder` within the node `id`
/// (and its whole subtree), adding to `n`. Walks the LOWERED core via the maintained `core_child_ids`
/// enumeration — so it is exhaustive over every `Core` variant without a hand-rolled per-variant match
/// (an undercount would wrongly reclaim a LIVE binding = miscompile). Reads memoized `core_of` results, so
/// it neither re-lowers nor perturbs `db.kept_bindings`/`captured_ref`. Used by `lower_let`'s post-fold
/// dead-binding reclaim to detect a kept binding whose every use folded away.
fn count_localref_reads(
    db: &mut Db,
    id: StructId,
    binder: StructId,
    n: &mut usize,
    seen: &mut std::collections::HashSet<StructId>,
) {
    // A shared sub-node is walked once — its `LocalRef` contribution is fixed, and the consumer only tests
    // `n > 0` (presence), so dedup preserves the liveness verdict while keeping the walk linear on a wide
    // shared DAG (a naive re-descent is exponential — the pom5 lowering-phase hang).
    if !seen.insert(id) {
        return;
    }
    if let Core::LocalRef { binder: b } = core_of(db, id)
        && b == binder
    {
        *n += 1;
    }
    for child in crate::backend::wasm::select::core_child_ids(db, id) {
        count_localref_reads(db, child, binder, n, seen);
    }
}

/// Whether the LOWERED CORE of `id` reaches a `Core::HostCall` anywhere in its subtree — walking the
/// maintained `core_child_ids` enumeration, so it FOLLOWS a call into its inlined callee body (unlike the
/// AST-based [`subtree_reaches_host_call`], whose List arm recurses only the syntactic children and so
/// MISSES a host call inside a nullary call's body — `(mk)` where `mk`'s body is `(host (io) (let ((v
/// (io.get))) …))` lowers to a `Core::Let` whose init IS the host call, reachable HERE but not through the
/// AST walk). adv-62b: a `let`-binding whose init is such a call must be FORCE-KEPT (materialized once), or
/// each use re-inlines the callee's `(host …)` block → the host op fires per use (double-fire trap). Reads
/// memoized `core_of` results (no re-lower, no `kept_bindings`/`captured_ref` perturbation). Bounded by a
/// visited set — a lowered core can share sub-nodes (a kept binding referenced twice), so a plain recursion
/// could re-walk; the set keeps it linear and cycle-safe.
fn core_reaches_host_call(
    db: &mut Db,
    id: StructId,
    seen: &mut std::collections::HashSet<StructId>,
) -> bool {
    if !seen.insert(id) {
        return false;
    }
    if matches!(core_of(db, id), Core::HostCall { .. }) {
        return true;
    }
    crate::backend::wasm::select::core_child_ids(db, id)
        .into_iter()
        .any(|child| core_reaches_host_call(db, child, seen))
}

/// The non-exhaustiveness fault of the match form `id`, if it has one — for the WELL-FORMEDNESS pass
/// (`compile::collect_faults` via `infer::collect_node`) to surface a CDZ0210 over EVERY match, not only
/// the ones the emit path lowers. `cdz check` runs `type_errors` on every def body but the reached-poison
/// (lowering) walk only on nullary EXPORTED bodies, so a non-exhaustive match on a function PARAMETER
/// (`(def (f (: c Color)) (match c …))`) — the common case — was silently missed by `check`/`--json`/`fix`
/// and an UNCALLED function's non-exhaustive match escaped emission entirely (dead, never laid out). This
/// closes both gaps by lowering just the match: exhaustiveness is a STRUCTURAL, value-independent verdict
/// (`build_tree` decides it from the scrutinee's TYPE and the arm patterns, before any constant fold), so
/// an unsubstituted parameter scrutinee gives the correct answer.
///
/// Returns ONLY a `Code::NonExhaustive` reject. A DECLINE (a not-yet-lowerable match — a runtime list, an
/// unsupported nested pattern) is dropped: those are not reported by `check` today, and surfacing them
/// here from a standalone (un-β-reduced) lowering would raise false alarms a call-site fold resolves. Any
/// OTHER coded reject (a shape/type fault in a pattern, CDZ0201/0203) is already produced by the
/// surrounding `collect_node` walk, so it is not re-raised here. SAFE to call during `check`: β-reduction
/// at a call site copies the callee body into FRESH nodes (`eval::beta_reduce`), so it never reads this
/// match node's own memoized core — pre-filling the slot for the unsubstituted body cannot corrupt an
/// emitted call site.
pub fn match_nonexhaustive_fault(db: &mut Db, id: StructId) -> Option<Reject> {
    match core_of(db, id) {
        Core::Poison(r) if r.code == Some(Code::NonExhaustive) => Some(r),
        _ => None,
    }
}

/// The CODED pattern-well-formedness fault of a `(match …)` — a mistyped variant pattern head
/// (`((C.Gren) …)` on `(type C Red Green)` → CDZ0201 "record has no field `Gren` — did you mean `Green`?"
/// carrying a replace fix on the key), a foreign-sum variant (CDZ0203), a payload-arity mismatch, or a
/// non-linear binder (CDZ0102). Surfaced for `type_errors` so `cdz check` catches it in EVERY body, not
/// only the nullary-EXPORTED ones the emit-path lowering walk (`collect_reached_poisons`) reaches — the
/// same "check missed what only lowering produced" hole `match_nonexhaustive_fault` closes for
/// exhaustiveness. Like a mistyped variant in VALUE position, a variant typo in a pattern is a
/// well-formedness fault independent of the function's parameter values, so surfacing it over an
/// unreached parameterized body is not a false alarm (the scrutinee's TYPE — the source of the variant
/// set — comes from its annotation, not a runtime value).
///
/// Returns the poison ONLY when it is a CODED pattern fault that is NOT the non-exhaustiveness CDZ0210
/// (which `match_nonexhaustive_fault` already reports — filter it here to avoid a double report). A
/// not-yet-lowerable DECLINE (an unbuilt compound scrutinee, an unsolved-`Any` scrutinee type) is
/// UNCODED, so it yields `None` and this adds no false alarm — the exact conservatism the exhaustiveness
/// accessor uses.
pub fn match_pattern_fault(db: &mut Db, id: StructId) -> Option<Reject> {
    match core_of(db, id) {
        Core::Poison(r) if r.code.is_some() && r.code != Some(Code::NonExhaustive) => Some(r),
        _ => None,
    }
}

/// The OPERATOR-SPECIFIC wrong-arity fault of an application whose head is a fixed-arity binary operator
/// applied to a number of operands other than 2 — an OVER-application (`(+ 1 2 3)`, `(< n 1 2)`, `(+ x 1.0
/// 2.0)`) or an UNDER-application (`(+ n)`, `(-)`) — the CDZ0201 "+ takes exactly 2 operands"
/// (`binop_arity_reject`), carrying a delete-surplus fix on the over-application. Surfaced for
/// `type_errors` so `cdz check` reports the CLEAR operator message in EVERY body, not only the
/// nullary-EXPORTED ones the emit-path lowering walk (`collect_reached_poisons`) reaches. Without this,
/// `check` on a PARAMETERIZED body reported only `infer::check_application`'s GENERIC CDZ0203 ("applied 3
/// arguments to a function of arity 2 — it is not a function after its arguments are consumed") for an
/// over-application, and NOTHING at all for an under-application — the same "check missed what only
/// lowering produced" hole [`match_pattern_fault`]/[`match_nonexhaustive_fault`] close for patterns
/// (`compile` rejects both, `check` was silent/vaguer).
///
/// Arity is a purely SYNTACTIC property of the application — `binop_arity_reject` fires on `args.len() != 2`
/// before touching either operand, and the only unary form of a binary operator is `(- e)` (negation, the
/// ML prefix `-<expr>`), which is EXEMPTED above (it lowers to `0 - e`, not a poison) — so surfacing this
/// over an unreached parameterized body is not a false
/// alarm, and reading `core_of` here is safe (β-reduction copies the callee body into fresh nodes, never
/// this node's memoized core). On an OVER-application the CDZ0201's delete-fix node matches the sibling
/// CDZ0203's, so `dedup_faults` keeps this operator-specific message and drops the generic one
/// (`operator_arity_fix_nodes`), reporting ONE primary error with the fix. On an UNDER-application there is
/// no CDZ0203 sibling (`check_application` stays silent) and no fix, so this is the sole report. A
/// well-formed 2-operand application lowers to real arithmetic, not a poison, so it yields `None`.
pub fn binop_arity_fault(db: &mut Db, id: StructId) -> Option<Reject> {
    let Resolved::Apply { head, args } = resolved_of(db, id) else {
        return None;
    };
    if args.len() == 2 {
        return None;
    }
    let prim = crate::eval::meta_apply_of(db, head)?;
    if !(prim.is_arith() || prim.is_comparison() || prim.is_float_arith()) {
        return None;
    }
    // The arity-1 `Sub` is UNARY NEGATION (the ML prefix `-<expr>`), NOT a wrong-arity subtraction — it
    // lowers to a type-directed `0 - e`, not a poison. Exempt it so a well-formed `(- e)` is not reported
    // as "takes exactly 2 operands". (A genuine wrong-arity `(- a b c)` / `(-)` still faults below.)
    if args.len() == 1 && prim == Prim::Sub {
        return None;
    }
    match core_of(db, id) {
        Core::Poison(r) if r.code == Some(Code::Malformed) => Some(r),
        _ => None,
    }
}

/// Lower a `(match scrutinee (pattern body)…)` over a SCALAR scrutinee. Each pattern classifies to a
/// [`Probe`] (an integer/boolean literal, a binder, or the wildcard `_`); a pattern that is none of
/// these declines (sum/tuple/record patterns walk the value heap — a separate path). If the scrutinee
/// FOLDS to a constant, select the first arm whose probe it satisfies and lower THAT arm's body (no
/// runtime match — like the const `if` fold). Otherwise the scrutinee is a runtime scalar: emit a
/// `Core::Match` the backend lowers to a probe chain.
///
//= spec/capabilities/core-semantics.md#matching-is-exhaustive-or-rejected
//# A match whose patterns do not cover every value of the scrutinee's type MUST be a compile-time error.
///
/// The arms are tried TOP-TO-BOTTOM: a constant scrutinee selects the FIRST arm whose probe it satisfies
/// and lowers only that arm's body, and a runtime scrutinee emits a probe chain that takes the first
/// matching arm — first-match-wins, as the corpus defines.
//= spec/capabilities/core-semantics.md#matching-is-exhaustive-or-rejected
//# A match MUST evaluate the branch of the first pattern that matches the scrutinee, as defined by the corpus.
///
/// A wildcard/binder tail covers the rest, and for an OPEN type (an integer) it is the only cover — no
/// finite literal set exhausts the integers. A FINITE type is exhausted by its literals instead: a Bool
/// scrutinee covered by both a `true` arm and a `false` arm needs no wildcard. A match that covers
/// neither way is rejected (CDZ0210), not compiled to a fallthrough with no defined value.
/// Whether `ty` is the EMPTY SUM — a `Ty::Sum` whose declaration has ZERO variants (an uninhabited
/// `Void`/`Never`, `type-system.md §Never Is The Empty Sum`). Such a type has no value, so a zero-arm
/// match on it is vacuously exhaustive. Reads the sum's declaration by its `decl` occurrence and checks
/// the variant count; a non-sum (or a sum with variants) is `false`.
///
/// A program declares one as an ordinary zero-variant sum — `(type Void)` — so the empty sum is a
/// NAMEABLE type in the universe (the dual of the empty tuple / unit), built by the same `type_decl` path
/// as any sum. It is UNINHABITED: `synthesize` builds a variant constructor per variant, and a zero-variant
/// declaration has none — there is no constructor and thus no value of the type.
//= spec/capabilities/type-system.md#never-is-the-empty-sum
//# The type universe MUST include the empty sum — a sum type with zero variants — as the dual of the unit type, which is the empty tuple, so that the zero of the sum constructor is a nameable type exactly as the zero of the product constructor is.
//= spec/capabilities/type-system.md#never-is-the-empty-sum
//# The empty sum MUST be uninhabited — it MUST have no constructor and no value — because a sum over zero variants offers no variant to construct.
fn is_empty_sum_ty(db: &mut Db, ty: &crate::ty::Ty) -> bool {
    if let crate::ty::Ty::Sum { decl, .. } = ty.strip_nominal() {
        return db
            .type_decl_by_occ(*decl)
            .is_some_and(|d| d.variants.is_empty());
    }
    false
}

/// Whether the node at `init` lowers to a COMPOUND heap value — a tuple or a record. These are the
/// values whose only-projected form folds through rather than being built on the heap.
fn is_compound_value(db: &mut Db, init: StructId) -> bool {
    matches!(core_of(db, init), Core::Tuple { .. } | Core::Record { .. })
}

/// Whether `init` REDUCES to a COMPOUND — a record / tuple / list — that CONTAINS a function value (a
/// lambda, directly or transitively through a nested compound). Uses only the eval-side reducers
/// (`reduce_to_record_id` / `reduce_to_tuple_elems` / `resolved_of` + `lambda_body`), which β-reduce/build
/// the compound VALUE but never LOWER it (`core_of`) — so, unlike the classification gates in
/// `should_keep_binding`, this does NOT speculatively lift a contained capturing lambda (which would
/// pollute `db.captured_ref` and miscompile the inline fold of the projected closure). Reducing (not just
/// matching `resolved_of`) is required because a record/tuple init is an `Apply` of the `RecordNew`/tuple
/// ctor prim, not a bare `Resolved::Record`/`Tuple`. Used to keep a projection-only
/// compound-holding-a-closure binding on the fold-through path (`should_keep_binding` returns `false`),
/// matching the inline-record and direct-let controls.
/// Whether `init` reduces to a compound (tuple/record/list) that holds a CURRIED function element — an
/// element that reduces to a lambda whose OWN BODY reduces to ANOTHER lambda, `(fn (a) (fn (b) …))`. This
/// is the narrow shape that MISCOMPILES when a projection-only compound folds through: projecting +
/// applying `((. t 0) 3)` folds `(. t 0)` to the outer lambda, β-reduces `a := 3`, YIELDS the inner
/// `(fn (b) …)`, then applies that — and inlining the returned inner lambda's body dangles its param `b`
/// (no closure frame for the returned fn) → "parameter reference has no local slot" (select.rs) at emit,
/// though `cdz check` passes (a check-vs-compile gap). A FLAT/SINGLE-arg fn element (`(fn (a) …)`, `(fn (a
/// b) …)`) folds correctly today (verified: single→4, flat-2→7, capturing-single→9), so it must NOT be
/// force-kept — only the CURRIED shape needs the compound materialized so the element lifts as a runtime
/// closure and applies via `call_indirect`. `lambda_body` reduces without lowering, so the probe is
/// lift-free (same discipline as `compound_contains_lambda`).
fn compound_contains_curried_lambda(db: &mut Db, init: StructId) -> bool {
    let elem_is_curried = |db: &mut Db, e: StructId| {
        // The element reduces to a lambda whose body ALSO reduces to a lambda = curried (arity ≥ 2 via
        // nesting). A nested compound is recursed (a curried fn stored inside a nested tuple/record).
        match crate::eval::lambda_body(db, e) {
            Some(body) => crate::eval::lambda_body(db, body).is_some(),
            None => compound_contains_curried_lambda(db, e),
        }
    };
    if let Some(elems) = crate::eval::reduce_to_tuple_elems(db, init) {
        return elems.iter().copied().any(|e| elem_is_curried(db, e));
    }
    if let Some(rec) = crate::eval::reduce_to_record_id(db, init)
        && let Resolved::Record { fields } = resolved_of(db, rec)
    {
        return fields.values().copied().any(|v| elem_is_curried(db, v));
    }
    false
}

fn compound_contains_lambda(db: &mut Db, init: StructId) -> bool {
    // A function-valued element (a bare lambda, or one reached through a nested compound). `lambda_body`
    // reduces without lowering, so the element probe stays lift-free.
    let elem_holds = |db: &mut Db, e: StructId| {
        crate::eval::lambda_body(db, e).is_some() || compound_contains_lambda(db, e)
    };
    // A TUPLE / LIST — its element occurrences (`reduce_to_tuple_elems` reduces the ctor `Apply`). `elems`
    // (an `Rc<[StructId]>`) is borrowed only by the iterator; each `elem_holds` mutably borrows `db`
    // (disjoint), so the walk needs no owned copy.
    if let Some(elems) = crate::eval::reduce_to_tuple_elems(db, init) {
        return elems.iter().copied().any(|e| elem_holds(db, e));
    }
    // A RECORD — reduce to the record occurrence, then read its field values (`fields` is the owned map of
    // the cloned `Resolved::Record`, so iterating it while `elem_holds` borrows `db` is disjoint).
    if let Some(rec) = crate::eval::reduce_to_record_id(db, init)
        && let Resolved::Record { fields } = resolved_of(db, rec)
    {
        return fields.values().copied().any(|v| elem_holds(db, v));
    }
    false
}

/// Whether `init` reduces to an `if`/`match` whose EVERY arm is a function value (a lambda, directly or
/// through a nested conditional). This is the control-flow analogue of [`compound_contains_lambda`]: a
/// conditionally-selected closure `(if b (fn …) (fn …))` / `(match c (p0 (fn …)) …)` must be
/// copy-propagated (not kept as a runtime `let` slot) so the case-of-case / case-of-match rewrite β-
/// reduces the selected arm inline against the call args, without ever LOWERING the conditional (which
/// would speculatively lift each arm's capturing lambda and pollute `db.captured_ref`, miscompiling the
/// inline fold). Uses only the eval-side reducers (`reduce_to_if` / `reduce_to_match` + `lambda_body`),
/// which β-reduce/select but never `core_of`-lower — so, like `compound_contains_lambda`, the detection
/// itself is lift-free. Requires EVERY arm to select a function (they share the conditional's type, so a
/// single lambda arm implies all are functions — the all-arms check just guards against a false positive
/// on a non-function `if`). Nested conditionals recurse. `None`-shaped (non-if, non-match) → `false`.
//
// TERMINATION (no visited-set / depth guard needed): the recursion descends only into arm BODIES, which
// are strictly-smaller arena nodes — the arena is append-only (`Arenas::push` assigns `len()` then never
// reassigns a slot; quote/metaprogramming uses the same append-only builder), so a child StructId is
// always < its parent and the raw AST is acyclic by construction. The only non-structural excursions are
// the eval-side reducers (`reduce_to_if`/`reduce_to_match`), whose `Apply` β-reduction runs under
// `Db::enter_reduction` (bounded by `REDUCE_DEPTH_LIMIT` + the cumulative `REDUCE_NODE_BUDGET`, declining
// rather than diverging) and whose `Ref` arm stops at any kept multi-use binding. So this walk terminates
// by construction. (amazon-q PR#556 flag — dismissed after verifying the arena invariant.)
fn if_or_match_selects_lambda(db: &mut Db, init: StructId) -> bool {
    // An arm body selects a function if it is a lambda, or itself a nested conditional of lambdas.
    let arm_selects = |db: &mut Db, body: StructId| {
        crate::eval::lambda_body(db, body).is_some() || if_or_match_selects_lambda(db, body)
    };
    if let Some((_, then_, else_)) = crate::eval::reduce_to_if(db, init) {
        return arm_selects(db, then_) && arm_selects(db, else_);
    }
    // A `match` — reduce to the `(match scrutinee (pat body)…)` form, then require every arm body to
    // select a function. `as_form` borrows the AST; collect the arm bodies first so `arm_selects` can
    // mutably borrow `db` in the walk. A malformed arm (not a `(pattern body)` pair) or an empty arm list
    // makes the whole probe FALSE (keep the binding) — the conservative side, never a vacuous pass.
    if let Some(match_form) = crate::eval::reduce_to_match(db, init) {
        let mut bodies: Vec<StructId> = Vec::new();
        match db.ast.as_form(match_form, "match") {
            Some([_scrutinee, arm_occs @ ..]) if !arm_occs.is_empty() => {
                for &arm in arm_occs {
                    match db.ast.get(arm) {
                        crate::ast::Struct::List(kv) if kv.len() == 2 => bodies.push(kv[1]),
                        _ => return false,
                    }
                }
            }
            _ => return false,
        }
        return bodies.into_iter().all(|b| arm_selects(db, b));
    }
    false
}

/// Whether the node at `init` lowers to a RUNTIME COMPUTATION — a core form that emits instructions
/// (arithmetic, comparison, conversion, a conditional, a runtime record), as opposed to a constant, a
/// unit, a bare local/param read, or a poison, which are free to duplicate. Reads the value's core
/// (the fold has already run, so a constant-folding binding reports `false` here).
fn is_runtime_computation(db: &mut Db, init: StructId) -> bool {
    let core = core_of(db, init);
    // A STATIC (fully-constant) tuple is NOT a runtime computation — keeping it would force a per-call
    // heap build (`arr-alloc`), pure waste for a value that never varies (`value-heap-runtime.md` §2d:
    // a static compound must not pay per-call construction). Leaving it UNKEPT lets each projection fold
    // straight through to the constant element (`reduce_to_tuple_elems` follows an unkept binding) — so
    // a constant tuple that is only projected emits ZERO heap ops, better than build-once. (A tuple with
    // a RUNTIME element genuinely allocates and IS kept — the H2b round-trip. The build-once-GLOBAL path
    // for a constant tuple that ESCAPES as a value activates with the first escape path — the renderer.)
    if matches!(core, Core::Tuple { .. }) && is_constant_compound(db, init) {
        return false;
    }
    matches!(
        core,
        Core::Arith { .. }
            | Core::Compare { .. }
            | Core::Convert { .. }
            | Core::If { .. }
            | Core::Record { .. }
            // A tuple with a runtime element constructs a heap value (an allocation), and a projection
            // reads one — both are runtime computations worth naming when used more than once. Keeping a
            // multi-use runtime tuple as a `Core::Let` binding is ALSO what makes its projection stay
            // runtime (the binding is opaque to the fold via `reduce_to_tuple_elems`) — the H2b round-trip.
            | Core::Tuple { .. }
            | Core::Proj { .. }
            // A list construction (`vec-empty` + a `vec-push` per element) is a genuine runtime
            // computation — an allocation per element — so a `let`-bound list used more than once is
            // worth NAMING (built once, the handle read by each use) rather than rebuilt at every use.
            // Unlike a tuple, a list has NO fold-through projection (a runtime-indexed `List.at` can't
            // fold to an element the way `(. t 0)` does), so `is_compound_value` deliberately does NOT
            // list `ListNew` — a list binding is always a whole-value use and simply keeps under the
            // >= 2-use rule below. (A single-use list still inlines: `n < 2`.)
            | Core::ListNew { .. }
            // A residual `Core::Call` (a recursive def that could not inline to a value) is a genuine
            // runtime computation — a real function call that may build heap structures, run a loop, etc.
            // A `let`-bound call used more than once must be NAMED (called ONCE, its result read by each
            // use) rather than copy-propagated into every use site — otherwise `(let ((xs (build …))) (+
            // (len xs) (* (len xs) 2)))` REBUILDS the whole list at each `xs`, an N× blow-up of the call
            // (and its transitive allocations). Sound under strict-`let` semantics (the binding evaluates
            // once at the `let`, as the kept `Arith`/`If`/… bindings already do); and strictly SAFER than
            // duplication for a call with any effect (calling it once, not N times). A single-use call
            // still inlines (`n < 2`).
            | Core::Call { .. }
            // A runtime `String.slice` / `String.to-bytes` (`StrSlice`/`StrToBytes`) builds a fresh leaf and
            // CONSUMES/dups its source — a genuine runtime computation — so a `let`-bound one used more than
            // once MUST be NAMED (evaluated once, the handle read by each use), NOT copy-propagated into every
            // use site. This is adv-54: `(let ((tail (Option.expect (String.slice s i j) "in")))
            // (let ((b (String.to-bytes tail))) (+ (Bytes.at b 0) (Bytes.at b 1))))` copy-propagated `b`
            // (neither op was in this list), RECOMPUTING the slice+to-bytes at each read (`String.slice`
            // returns `(Option String)`, so it is unwrapped before `String.to-bytes`);
            // the recompute CONSUMES the (borrowed) source, so the 2nd read saw freed
            // bytes → 0 — a wasm-only HIGH soundness miscompile (rust rebuilt correctly). Not multibyte-
            // specific: an ASCII RUNTIME slice fails too; the breaker's "ASCII OK" used a CONSTANT slice (which
            // const-folds, no runtime recompute). Keeping these two under the >= 2-use rule fixes it. (SCOPED
            // to these two: the other Bytes/List/Map/Set/BigInt/Rational/Bin heap ops share the SAME latent
            // copy-propagate shape, but forcing them kept regressed 3 cases [bin-parse invalid-component + a
            // rope-keyed-map bytes-compact] — their kept-binding EMIT has its own bug to fix first (follow-up)
            // before widening. Scalar READS — *Len/*Size/BytesAt/*Cmp/Contains/HasKeys — copy-propagate fine.)
            | Core::StrToBytes { .. }
            | Core::StrSlice { .. }
            // adv-54b: `Bytes.concat` is the same family — a fresh-leaf-building runtime op that CONSUMES/
            // dups its operands. A `let`-bound concat used more than once was COPY-PROPAGATED (it was not in
            // this list), RECOMPUTING the concat — and its slice-VIEW `String.to-bytes` operands — at each
            // read; each recompute consumed the borrowed view sources, so the 2nd read walked a freed buffer
            // → wasm OOB trap (rust rebuilt correctly → 200). Keeping it under the >= 2-use rule names `b`
            // once. This was the deferred wider-family widening the adv-54 note warned regressed 3 cases
            // ["their kept-binding EMIT has its own bug to fix first"]; that emit bug was adv-66 (#1610 —
            // `BytesConcat` mis-classified in `mark_binder_dups`, now CONSUMING), so the keep is regression-
            // free NOW (gate 0-fail with this arm; adv54b witness computes 200, traps OOB without it).
            | Core::BytesConcat { .. }
    )
}

/// Whether the node at `id` lowers to a fully COMPILE-TIME-CONSTANT compound (or scalar): a constant
/// scalar/unit, or a `Core::Tuple` all of whose elements are themselves constant (recursively). This is
/// the classification that routes a STATIC compound away from per-call construction (§2d): a constant
/// tuple has no runtime-varying part, so it need never be built at run time — its projections fold, and
/// (once an escape path exists) its materialization is a build-once global rather than a per-call alloc.
pub fn is_constant_compound(db: &mut Db, id: StructId) -> bool {
    match core_of(db, id) {
        Core::ConstInt(_) | Core::ConstBool(_) | Core::Unit => true,
        Core::Tuple { elems } => elems.iter().all(|&e| is_constant_compound(db, e)),
        Core::Record { fields } => fields.values().all(|&v| is_constant_compound(db, v)),
        // A `Core::ListNew` all of whose elements are constant is a constant compound too — a list-`let`
        // read only through element/rest pattern binders folds each read to the constant element
        // (`should_keep_binding`'s list short-circuit). (The tuple short-circuit in `is_runtime_computation`
        // gates on `Core::Tuple` first, so this addition does not change that path.)
        Core::ListNew { elems } => elems.iter().all(|&e| is_constant_compound(db, e)),
        _ => false,
    }
}

/// Whether the node at `id` is a per-node-IMMORTAL-MARKABLE constant compound — the §2d STATIC compound
/// (increment 6, tuple/record) hoist CANDIDATE. True for a `Core::Tuple` / `Core::Record` whose every
/// element is itself a per-node-markable constant (see [`is_markable_constant_elem`]). The compiler builds
/// EVERY node of such a tree explicitly (`arr-alloc` + a boxed leaf per element, recursively), so a
/// build-once init can mark each node IMMORTAL per node (`mark-immortal` is shallow, so the WHOLE tree must
/// be marked to be census-excluded + drop-safe).
///
/// DELIBERATELY EXCLUDES `Core::ListNew` and maps (and any compound CONTAINING one): a list's `vec-of-arr`
/// (for `> 32` elements) and a map's hashed insert build RRB / CHAMP INTERNAL nodes that have no
/// compile-time handle to `mark-immortal` — marking those needs a DEEP-mark op (deferred). Also excludes
/// `ConstFloat`/`ConstChar` leaves for this first slice (conservative — they box fine, a later widening).
/// A non-markable element makes the whole compound decline (it builds inline, per-eval, as before).
pub fn is_markable_constant_compound(db: &mut Db, id: StructId) -> bool {
    match core_of(db, id) {
        Core::Tuple { elems } => elems.iter().all(|&e| is_markable_constant_elem(db, e)),
        Core::Record { fields } => fields.values().all(|&v| is_markable_constant_elem(db, v)),
        _ => false,
    }
}

/// Whether the node at `id` is a fully-constant `Core::ListNew` that can hoist as a build-once immortal
/// static — ANY size (the `> 32` cap is gone). A list needs a SEPARATE predicate from
/// [`is_markable_constant_compound`] (which excludes EVERY list): a `(list e0…e{n-1})` literal lowers to
/// `arr-alloc(n)` + n×`arr-set` + ONE `vec-of-arr`, and `vec-of-arr` for `> 32` builds a 2-level RRB trie
/// whose INTERNAL nodes are minted inside the op with NO compile-time handle. The build-once emit marks the
/// list ROOT with `mark-immortal-DEEP` (op 96, `#4301`), which transitively marks the whole structure
/// (header + arr leaf for ≤32; spine + all trie leaves + every element for `> 32`), so those handleless trie
/// internals ARE reached — which is why the size cap could be lifted.
///
/// Two shapes stay EXCLUDED (they would leak — see the emit arm): an EMPTY list (the shared `vec-empty`
/// SINGLETON — unsound to mark a shared node immortal), and an ALL-`Bool` list (`vec-of-arr` PACKS it into a
/// fresh bit-leaf while dropping the source `arr` that holds the shallow-marked element boxes → those
/// immortal boxes are orphaned). Element markability reuses [`is_markable_constant_elem`], which DOES
/// recurse into a nested `Core::ListNew`/`MapNew`/`SetOf` (PR #4989/#4994) — so a list whose element is
/// itself a (non-bool, non-empty) list / map / set IS markable, the root's `mark-immortal-DEEP` reaching
/// the nested collection's whole structure transitively — as is a list of markable scalars / `Bytes` /
/// `String` / `Tuple` / `Record` / `SumNew` of any size.
pub fn is_markable_constant_list(db: &mut Db, id: StructId) -> bool {
    match core_of(db, id) {
        Core::ListNew { elems } => {
            // NON-empty, every element markable, AND not ALL-`Bool`. NO size cap: the build-once emit marks the
            // list root with `mark-immortal-DEEP` (op 96), which transitively marks the whole structure — so a
            // `> 32` list's RRB trie internals (minted inside `vec-of-arr` with no compile-time handle) ARE
            // reached, unlike a per-node shallow mark. Two `vec-of-arr` shapes stay excluded because they'd leak:
            // an EMPTY list returns the shared `vec-empty` SINGLETON (marking a shared singleton immortal is
            // unsound), and an ALL-`Bool` list is PACKED into a fresh bit-leaf while the source `arr` (holding
            // the shallow-marked element boxes) is dropped → those immortal boxes are ORPHANED (never freed, not
            // in the packed leaf) = a leak. A mixed / non-bool list of any size is admitted (its elements are
            // reused (≤32) or drained into trie leaves (>32), never orphaned).
            !elems.is_empty()
                && !elems
                    .iter()
                    .all(|&e| matches!(core_of(db, e), Core::ConstBool(_)))
                && elems.iter().all(|&e| is_markable_constant_elem(db, e))
        }
        _ => false,
    }
}

/// Whether the node at `id` is a fully-constant `Core::MapNew` that can hoist as a build-once immortal
/// static. NON-empty, every entry's KEY and VALUE per-node-buildable via [`is_markable_constant_elem`]. Like
/// the list case this needs the DEEP mark (op 96): the build-once emit builds the CHAMP (`map-empty` +
/// per-entry `map-insert`, which CONSUMES the map/key/value) then `mark-immortal-DEEP`s the final root, which
/// transitively marks the whole HAMT spine + every data-entry key/value handle. Because `map-insert` consumes
/// (moves, never copies) its arguments, there is no orphan-leak hazard (contrast the all-`Bool` list pack).
///
/// EMPTY is excluded (the `map-empty` shared singleton — unsound to mark immortal). A nested `List`/`Map`/
/// `Set` key or value IS markable (PR #4989/#4994: [`is_markable_constant_elem`] recurses `ListNew`/`MapNew`/
/// `SetOf`), built inline by the immortal MapNew builder (`emit`) or its own `emit_immortal_static` arm and
/// covered by the root's `mark-immortal-DEEP`.
pub fn is_markable_constant_map(db: &mut Db, id: StructId) -> bool {
    match core_of(db, id) {
        Core::MapNew { entries, .. } => {
            !entries.is_empty()
                && entries.iter().all(|&(k, v)| {
                    is_markable_constant_elem(db, k) && is_markable_constant_elem(db, v)
                })
        }
        _ => false,
    }
}

/// Whether the node at `id` is a fully-constant `Core::SetOf` that can hoist as a build-once immortal
/// static — the set analogue of [`is_markable_constant_map`] (a CHAMP-minus-value-column). NON-empty, every
/// element per-node-buildable via [`is_markable_constant_elem`]. Built once (`set-empty` + per-element
/// `set-insert`, which CONSUMES set+element — moves in, no copy) then `mark-immortal-DEEP` on the root, which
/// transitively marks the whole HAMT + element handles. EMPTY excluded (`set-empty` shared singleton); a
/// nested `List`/`Map`/`Set` element IS markable (PR #4989/#4994: [`is_markable_constant_elem`] recurses
/// the collection predicates; the root deep-mark covers the nested structure).
pub fn is_markable_constant_set(db: &mut Db, id: StructId) -> bool {
    match core_of(db, id) {
        Core::SetOf { elems, .. } => {
            !elems.is_empty() && elems.iter().all(|&e| is_markable_constant_elem(db, e))
        }
        _ => false,
    }
}

/// Whether the node at `id` is a NULLARY variant of a MIXED sum — a build-once-immortal candidate (the
/// rsl1 leak-1 class; v-runtime root-cause, v-memory-safety soundness-confirmed 2026-08-28). A nullary
/// variant (`(Z)`/`(Nil)`/`[]`) of a sum that has ≥1 COMPOUND variant does NOT get the all-nullary
/// enum-discriminant representation (`is_enum_disc`), so it is a REAL rc-counted heap node built FRESH
/// every construction, and on a recursive-sum walk (`suml`/`depth-tail`) its terminal leaks. Building it
/// ONCE as an immortal shared node (`mark-immortal` → `op_dup`/`op_drop` no-op → census-excluded) fixes the
/// leak regardless of the over-ref site (construction OR walk), with NO wrong-target-drop risk — the
/// principled fix (a nullary constant SHOULD be build-once-shared, not fresh-per-construction).
///
/// SOUNDNESS GATE (v-memory-safety, critical): match ONLY `Core::SumNew` with EMPTY payloads over a MIXED
/// sum. EXCLUDE (a) an all-nullary enum-discriminant sum (`is_enum_disc` — no heap node; the value IS its
/// discriminant, marking it is a no-op / double-handle hazard) and (b) any COMPOUND variant (non-empty
/// payloads — a varying payload cannot be shared). A nullary variant carries no payload, so it is trivially
/// constant (no `is_markable_constant_elem` recursion needed).
pub fn is_markable_constant_sum_nullary(db: &mut Db, id: StructId) -> bool {
    match core_of(db, id) {
        Core::SumNew { payloads, .. } if payloads.is_empty() => {
            // MIXED sum only: the type is a `Ty::Sum` whose decl is NOT an all-nullary enum-disc.
            let ty = crate::infer::type_of(db, id);
            match ty.strip_nominal() {
                crate::ty::Ty::Sum { decl, .. } => !db.is_enum_disc(*decl),
                _ => false,
            }
        }
        _ => false,
    }
}

/// Whether the node at `id` is a PAYLOADED variant of a MIXED sum whose payloads are ALL markable constants
/// — a build-once-immortal candidate, the payloaded extension of [`is_markable_constant_sum_nullary`]
/// (v-core-opt greenlit 2026-08-28; SAME const-list deep-mark + FBIP-strict-rc==1 soundness pattern, no new
/// invariant). A `(Some 5)` / `(Cons 1 (list …))` reaches the runtime `Core::SumNew` emit and is rebuilt
/// FRESH every construction; building it once immortal (`mark-immortal-DEEP` — the payloads are heap
/// children) makes every use a `global.get`, cutting the per-eval alloc.
///
/// GATE (v-core-opt's tightness reqs): (a) MIXED sum only (`!is_enum_disc` — an all-nullary enum has no heap
/// node); (b) EVERY payload `is_markable_constant_elem` (a payload with ANY runtime value must NOT hoist — it
/// is not a constant); (c) implicitly fully-constant (a non-constant payload fails (b)). `is_markable_constant_elem`
/// DOES recurse into a nested `Core::SumNew` (and `ListNew`/`MapNew`/`SetOf`/`Tuple`/`Record`), so a NESTED
/// recursive-sum payload (`Cons`-of-`Cons`) IS markable — the root's `mark-immortal-DEEP` covers the whole
/// spine; a variant wrapping scalars / `Bytes` / `String` / `Tuple` / `Record` / `List` / `Map` / `Set` IS.
pub fn is_markable_constant_sum_payloaded(db: &mut Db, id: StructId) -> bool {
    let payloads = match core_of(db, id) {
        Core::SumNew { payloads, .. } if !payloads.is_empty() => payloads,
        _ => return false,
    };
    // MIXED sum only (an all-nullary enum-disc has no heap node — but a payloaded variant is never enum-disc
    // anyway; the guard mirrors the nullary predicate for symmetry + safety).
    let mixed = match crate::infer::type_of(db, id).strip_nominal() {
        crate::ty::Ty::Sum { decl, .. } => !db.is_enum_disc(*decl),
        _ => false,
    };
    mixed && payloads.iter().all(|&p| is_markable_constant_elem(db, p))
}

/// Whether an ELEMENT of a candidate static compound is per-node-markable (see
/// [`is_markable_constant_compound`]): a constant MACHINE-int (`Ty::Int`) or `Bool`/`Unit` scalar (boxes to
/// ONE markable heap node via `box-int`/`box-bool` — `Unit` is the inline `IMM_UNIT` sentinel, already
/// rc-free), a constant `Bytes`/`String` leaf, or a NESTED markable `Tuple`/`Record`/`SumNew`/`ListNew`/
/// `MapNew`/`SetOf` (recursed via the collection predicates — PR #4989/#4994; the root `mark-immortal-DEEP`
/// covers the whole nested tree).
///
/// The `Ty::Int` guard on `ConstInt` is LOAD-BEARING: `(BigInt.of 10)` folds to a `Core::ConstInt` typed
/// `Ty::BigInt`, but a BigInt const materializes as a `bigint-of-*` HEAP LEAF (not a `box-int` node), which
/// the compound builder's scalar path (`box_op` → `None` for a handle) would leave UNMARKED → a census leak
/// (a real bug caught by `06-numeric-model` "a tuple key with an ARITH-built BigInt leaf": expected 0 got 1).
/// So a BigInt/Rational-typed const (and `ConstFloat`/`ConstChar`, or a runtime value) is NOT markable — the
/// compound declines and builds inline, per-eval, as before.
fn is_markable_constant_elem(db: &mut Db, id: StructId) -> bool {
    match core_of(db, id) {
        Core::ConstBool(_) | Core::Unit => true,
        // Only a MACHINE int (`box-int` → one node), NOT a BigInt/Rational-typed const (a heap leaf the
        // scalar path would not mark → leak).
        Core::ConstInt(_) => matches!(
            crate::infer::type_of(db, id).strip_nominal(),
            crate::ty::Ty::Int(_)
        ),
        Core::Tuple { .. } | Core::Record { .. } => is_markable_constant_compound(db, id),
        // A NESTED constant mixed-sum variant — a recursive-sum literal (`(Cons 1 (Cons 2 Nil))`) or a sum
        // nested in a tuple/record/list (`(list (Some 1) (Some 2))`). Recurse into the sum predicates (nullary
        // OR payloaded); the build-once-immortal `mark-immortal-DEEP` on the ROOT covers the whole nested tree
        // (nested-collection immortals — the payloaded-sum #4814 extended one level; this closes the recursion).
        // Terminates: a finite constant literal bottoms out at a nullary terminal / scalar leaf.
        Core::SumNew { .. } => {
            is_markable_constant_sum_nullary(db, id) || is_markable_constant_sum_payloaded(db, id)
        }
        // A NESTED constant LIST — a list element of a tuple/record/list/sum-payload (`(list (list 1) (list 2))`,
        // a `Cons` payload, a record field). Recurse into `is_markable_constant_list` (non-empty, not all-`Bool`,
        // every element itself markable); the ROOT's `mark-immortal-DEEP` transitively covers the nested list's
        // whole structure (header/arr or spine + trie leaves + element handles). `emit_immortal_elem` has a
        // matching `ListNew` arm (→ `emit_immortal_static`), and the map/set KEY canonicalize ops a nested list
        // needs (`value-canonicalize`/`bytes-compact`) are force-imported into the static-compound init.
        // Terminates: a finite constant literal bottoms out at a scalar / `Bytes` / `String` leaf.
        Core::ListNew { .. } => is_markable_constant_list(db, id),
        // A NESTED constant MAP/SET — a collection element of a tuple/record/list/sum-payload (a map as a
        // record field, a set in a list, a `Cons` payload). Recurse into `is_markable_constant_map`/`_set`
        // (non-empty, every key/value/element itself markable); the ROOT's `mark-immortal-DEEP` transitively
        // covers the nested CHAMP (HAMT spine + entry handles). `emit_immortal_elem` has matching `MapNew`/
        // `SetOf` arms (→ `emit_immortal_static`); the START-init declares the canonicalize scratch locals a
        // nested list-keyed map needs (`static_compound_init_locals`). Terminates: a finite constant literal
        // bottoms out at a scalar / `Bytes` / `String` leaf.
        Core::MapNew { .. } => is_markable_constant_map(db, id),
        Core::SetOf { .. } => is_markable_constant_set(db, id),
        _ => is_constant_bytes(db, id) || constant_string_value(db, id).is_some(),
    }
}

/// Whether the node at `id` is a fully COMPILE-TIME-CONSTANT `Bytes` value — the static-once CANDIDATE
/// predicate for §2d STATIC bytes (`DESIGN-static-data.md`). Covers BOTH representations of a constant
/// byte sequence, exactly as the backend emit does: a `Core::BytesOf` whose every element folds to a
/// constant byte in `0..=255` (a `b"…"` literal / `Bytes.of` of constants) AND a baked `Core::ConstBytes`
/// leaf (the `Ast.encode` / `Blake3.of` / const-transform fold). Such a value never varies across
/// evaluations, so once the build-once machinery is active it is materialized ONCE into a module global
/// and read with `global.get` at each use — rather than the per-call `bytes-alloc` + a `bytes-set` per
/// byte both arms emit today. A `BytesOf` with even one RUNTIME element (a `UInt8` param, `(UInt8.wrap n)`)
/// is NOT constant — it must build per call — so a single runtime element disqualifies the whole literal.
///
/// Kept DELIBERATELY SEPARATE from [`is_constant_compound`] rather than folded into it: recognizing
/// bytes there would flip the `Core::Tuple` short-circuit in `is_runtime_computation` (a tuple holding
/// only a constant bytes literal would stop being kept as a `Core::Let` binding, so each projection would
/// re-emit the construction — a behavior change AND a pessimization until the build-once global exists).
pub fn is_constant_bytes(db: &mut Db, id: StructId) -> bool {
    const_byte_slice(db, id).is_some()
}

/// The BYTE CONTENT of a fully-constant `Bytes` value at `id` — `Some(bytes)` when [`is_constant_bytes`]
/// holds, else `None`. This is the material the build-once path (`DESIGN-static-data.md` increment 2)
/// needs beyond the boolean predicate: the byte sequence is BOTH the interning KEY (two literals with
/// identical content share one module global — the constant-CSE the design calls for) AND the `(data …)`
/// segment PAYLOAD the init materializes the handle from. Returns the same bytes both constant-`Bytes`
/// emit arms (`Core::BytesOf` of constants, `Core::ConstBytes`) would `bytes-set` into a fresh buffer per
/// call — so a build-once global reading these bytes is observably identical, just allocated once.
///
/// Delegates to [`const_byte_slice`], the single canonical constant-`Bytes` reader every fold site uses,
/// so the two representations stay interchangeable here exactly as they are at every fold. `None` for any
/// node that is not a fully-constant `Bytes` (a scalar, a runtime-element `BytesOf`, a non-bytes compound).
pub fn constant_bytes_value(db: &mut Db, id: StructId) -> Option<Vec<u8>> {
    const_byte_slice(db, id)
}

/// The UTF-8 BYTES of a fully-constant `String` at `id` — `Some(bytes)` when `core_of(id)` is a
/// `Core::ConstStr` (a string literal / a compile-time-folded string), else `None`. Groundwork for §2d
/// STATIC strings (`DESIGN-static-data.md` increment 5): a Cadenza `String` value IS a flat UTF-8
/// byte-leaf, materialized by the backend with the SAME `bytes-alloc` + a `bytes-set` per byte a `Bytes`
/// value uses (the `Core::ConstStr` handle emit; see the wrapper `mem_leaf` note "a Cadenza String value IS
/// a flat UTF-8 byte-leaf, built exactly by bytes-alloc + bytes-set"). So a constant string that escapes as
/// a runtime handle re-allocates per evaluation exactly as a constant `Bytes` does, and is hoisted by the
/// identical build-once-global mechanism — its payload is these UTF-8 bytes (the same interning key /
/// data-segment payload `constant_bytes_value` returns for `Bytes`). Since the runtime rep of a constant
/// String and a constant `Bytes` with the same bytes is the SAME flat leaf, they can even share one global
/// (an interning refinement for the string increment). `None` for a runtime string or any non-string node.
pub fn constant_string_value(db: &mut Db, id: StructId) -> Option<Vec<u8>> {
    match core_of(db, id) {
        Core::ConstStr(s) => Some(s.as_bytes().to_vec()),
        _ => None,
    }
}

/// The CANONICAL BINARY VALUE FORM of a fully-constant compound at `id` — the bytes the resource escape
/// path's `encode()` returns (`DESIGN-value-heap-rcdzc.md` §3a; `contracts/deterministic-value-form.md`).
/// Reconstructs the s-expression `(: <value> <type>)` as ordinary AST (the value from the constant core,
/// the type from the solved `type_of`) and encodes it with the shared codec — the SAME bytes the corpus
/// value form uses, so the host decodes + pretty-prints them to the recorded text. Returns `None` if the
/// node is not a compile-time-constant compound (a runtime compound's bytes are built by the real
/// handle-walking encoder, R2 — this constant path is R1's proof that the resource+`encode()`+decode
/// pipeline crosses correctly before the walk exists). The type is baked as constant bytes (the runtime
/// is name-free); this does NO in-wasm formatting — it is a compile-time serialization.
pub fn constant_value_form(db: &mut Db, id: StructId) -> Option<Vec<u8>> {
    let mut b = crate::ast::Builder::new();
    let colon = b.name(":");
    let value = const_value_ast(db, &mut b, id)?;
    let ty = crate::infer::type_of(db, id);
    let type_ast = type_ast(&mut b, &ty, &db.name_ctx())?;
    let root = b.list(vec![colon, value, type_ast]);
    Some(crate::codec::encode(&b.finish(root)))
}

/// The BARE canonical value document of a fully-constant compound at `id` — [`constant_value_form`] WITHOUT
/// the `(: value type)` frame: just the value node (`(tuple …)` / `(record …)` / `(list …)` / a variant / a
/// scalar leaf), codec-encoded. This is the form the runtime `value-encode` op emits for a value crossing the
/// REDUCER/PROVIDER boundary — that boundary is BARE both directions (the kernel's `build_event_document`
/// reads/writes the bare value form; `value-encode` frames only a `Named`/`Framed` descriptor root, so a bare
/// `bare_shape_descriptor` root yields the bare doc — v-ah+v-runtime ruling 2026-08-12). So for a member whose
/// RESULT is a compile-time constant, this is EXACTLY the wire bytes the per-event value-encode walk would
/// produce, precomputed once at compile time (pre-encode / Axis 2, provider path): emit these as a `(data)`
/// segment + memcpy at the boundary, skipping the runtime build AND the per-event encode. `None` when the node
/// is not a compile-time-constant compound (`const_value_ast` declines — a runtime value keeps the walking
/// encoder). The codec is canonical (one byte encoding per value), so these bytes are byte-IDENTICAL to the
/// runtime bare `value-encode` output for the same constant.
pub fn constant_value_form_bare(db: &mut Db, id: StructId) -> Option<Vec<u8>> {
    let mut b = crate::ast::Builder::new();
    let value = const_value_ast(db, &mut b, id)?;
    Some(crate::codec::encode(&b.finish(value)))
}

/// Build a member-access form `(. <operand-name> <key>)` — the canonical shape the reader normalizes a
/// dotted name `Operand.key` to (an alphabetic-segment postfix desugar). Used to bake a unit/quantity
/// value form so it re-reads to the same tree the corpus records (`(. Qty of)`, `(. Unit base)`).
fn member_access(b: &mut crate::ast::Builder, operand: &str, key: &str) -> StructId {
    // M2: the `(. operand key)` head is the native MEMBER leaf kind (recognized by kind), not the `.` name.
    // Keeps the compiler-emitted Qty/Symbol/Unit value-construction member forms native head-first, matching
    // op62's runtime encode (byte-EQ) and the reader's native `.` flip.
    let op = b.name(operand);
    let k = b.name(key);
    b.member(op, k)
}

/// Reconstruct a TYPE s-expression into `b`, matching `Ty::render_name`'s surface exactly so the host
/// prints the recorded type: `Int64`/`UInt8`/… as a name atom, `Bool`/`Unit` likewise, a tuple as
/// `(Tuple T…)`, a record as `(record (name T)…)`. `None` for a type with no value-form surface (a
/// function/type-value/unsolved variable can never be a runtime value crossing the boundary).
///
/// `pub(crate)` so the Cadenza backend (`backend::cadenza`) reuses the SAME canonical type-surface when
/// it re-emits a parameter's `(: name Ty)` ascription — sharing this one renderer keeps the re-emitted
/// type byte-identical to lower's value-form surface (a divergent copy would break round-trip identity).
pub(crate) fn type_ast(
    b: &mut crate::ast::Builder,
    ty: &crate::ty::Ty,
    ncx: &crate::ty::NameCtx,
) -> Option<StructId> {
    use crate::ty::Ty;
    match ty {
        // A scalar's type surface is its name atom. `String`/`Char`/`Symbol`/`BigInt`/`Rational` are
        // monomorphic named types too, so their surface is the bare `String`/…/`Rational` atom
        // (`render_name`).
        Ty::Int(_)
        | Ty::Bool
        | Ty::Unit
        | Ty::String
        | Ty::Char
        | Ty::Symbol
        | Ty::BigInt
        | Ty::Rational => Some(b.name(ty.render_name(ncx))),
        // A sum's type surface: the bare NAME for a monomorphic sum (`(: (Neg unit) Sign)`), or the
        // STRUCTURED application `(Option Int64)` for a generic instantiation — a `(NAME arg…)` list, so
        // the args round-trip as separate nodes (not one spaced-out name atom). Matches `render_name`'s
        // surface but built as real structure so the codec + host reader see the parameterized type.
        Ty::Sum { decl, args, .. } => {
            let name = ncx.name_of(*decl)?.to_string();
            if args.is_empty() {
                Some(b.name(name))
            } else {
                let head = b.name(name);
                let mut children = vec![head];
                for a in args.iter() {
                    children.push(type_ast(b, a, ncx)?);
                }
                Some(b.list(children))
            }
        }
        Ty::Tuple(elems) => {
            let head = b.name("Tuple");
            let mut children = vec![head];
            for t in elems.iter() {
                children.push(type_ast(b, t, ncx)?);
            }
            Some(b.list(children))
        }
        Ty::Record(fields) => {
            // The TYPE head is capitalized `Record` (like `Tuple`); the VALUE head is lowercase `record`
            // (see `const_value_ast`). Each field is the canonical `(: name T)` ASCRIPTION node (RT3,
            // DESIGN-record-type-syntax) — the corpus writes `(Record (: a Int64) …)` for the type,
            // matching `render_name`/`encode_ty`.
            let head = b.name("Record");
            let mut children = vec![head];
            for (name, t) in fields.iter() {
                let colon = b.name(":");
                let fname = b.name(&*name.name);
                let fty = type_ast(b, t, ncx)?;
                children.push(b.list(vec![colon, fname, fty]));
            }
            Some(b.list(children))
        }
        // A list's type surface is `(List Elem)` — matches `render_name`.
        Ty::List(elem) => {
            let head = b.name("List");
            let ety = type_ast(b, elem, ncx)?;
            Some(b.list(vec![head, ety]))
        }
        // A map's type surface is `(Map Key Value)` — matches `render_name` (key first).
        Ty::Map(k, v) => {
            let head = b.name("Map");
            let kty = type_ast(b, k, ncx)?;
            let vty = type_ast(b, v, ncx)?;
            Some(b.list(vec![head, kty, vty]))
        }
        // A set's type surface is `(Set Elem)` — one element type parameter (matches `render_name`).
        Ty::Set(elem) => {
            let head = b.name("Set");
            let ety = type_ast(b, elem, ncx)?;
            Some(b.list(vec![head, ety]))
        }
        // A bytes value's type surface is the bare name `Bytes` (a leaf, like a scalar) — matches
        // `render_name`; its VALUE renders `b"…"` (built in `const_value_ast` / the escape walker).
        Ty::Bytes => Some(b.name("Bytes")),
        // A still-free type variable in an escaping value's type has NO defined serialization — a bare
        // `(None)` : `Option ?0` or an empty `(list)` : `List ?0` whose payload/element nothing pins. It
        // is NOT rendered (no honest concrete surface exists): returning `None` here makes
        // `constant_value_form`/`sum_form_template` decline, so the escape falls through to the
        // AMBIGUOUS-TYPE guard in `backend/wasm/mod.rs` (`has_free_var` → CDZ0203, "annotate it") rather
        // than crossing with an invented type. type-system.md §An Escaping Value MUST Have A Fully
        // Determined Type; corpus 07 "an escaped value with an unresolved payload type is rejected".
        // A float's type surface is its aliased width name `Float32`/`Float64` (a leaf, like a scalar) —
        // matches `render_name`; its VALUE renders as the float literal. Needed when a float is NESTED in
        // a compound value form (`(tuple 1.0 2.0)`) whose type annotation the escape bakes.
        Ty::Float(ft) => Some(b.name(format!("Float{}", ft.ground_width()))),
        // A quantity's type surface is `(Qty <inner> <unit>)` — matches `render_name`. The inner type is
        // built recursively; the unit is built as REAL structure by `unit_value_ast` (`Unit.one`,
        // `(Unit.base #"n")`, `(Unit.^ (Unit.base #"n") k)`, left-nested `(Unit.* …)`), so the rendered
        // type re-reads to the same unit (a name atom carrying parens would not round-trip). The corpus
        // surface `(Qty Float64 (Unit.base #"meter"))`.
        Ty::Qty { inner, unit } => {
            let head = b.name("Qty");
            let ity = type_ast(b, inner, ncx)?;
            let uty = unit_value_ast(b, unit);
            Some(b.list(vec![head, ity, uty]))
        }
        // A nominal's type surface is its declared NAME atom (`(: (Mk 42) UserId)`) — its identity is
        // the name, not its underlying shape (like a monomorphic sum). The value itself renders as the
        // underlying value form (built by the value walker, which sees through the tag).
        Ty::Nominal { decl, .. } => Some(b.name(ncx.name_of(*decl)?)),
        // The TYPE OF TYPES — the type surface of a type-VALUE (`(: Int64 Type)`). A type is a first-class
        // value that can be returned and inspected at run time (core-semantics.md §Types Are First-Class
        // Values), so a bare type-value crosses the boundary; its TYPE node is the atom `Type` (the value
        // node is the concrete type's name, built by `const_value_ast`'s `Ty::Type` arm). `render_name`.
        Ty::Type => Some(b.name("Type")),
        // A function has no boundary value form, so no type surface. A still-free type variable / `Any`
        // has no determined serialization. A program that would escape one declines before the escape.
        // A reified continuation has no value-form surface (like a function) — it never crosses as a
        // rendered value.
        Ty::Fn(_, _) | Ty::Cont { .. } | Ty::Var(_) | Ty::Any => None,
    }
}

/// Conditional-constant-propagation helper: if `branch` reduces to an inner `(if c' A B)` whose
/// condition `c'` is EQUIVALENT to the enclosing `cond` (via `core_equiv` — a pure-core structural
/// match), return the occurrence of the arm the enclosing branch's known truth of `cond` selects — `A`
/// when `cond_is_true` (the then-branch, where `cond` holds), `B` otherwise (the else-branch, where it
/// does not). Also handles the NEGATED case: when `c'` is the boolean negation of `cond` (`(not cond)`,
/// or `cond` is `(not c')`), the known truth of `cond` implies the OPPOSITE truth of `c'`, so the FLIPPED
/// arm is selected — `(if c A (if (not c) B D))` takes `B` in the else-branch (where `c` is false, so
/// `(not c)` is true). `None` if `branch` is not such a nested `if` (leave it unchanged). The returned
/// occurrence is REUSED as-is (no synthesis); it was resolved in the same scope, so lowering it in the
/// branch's place is sound. `reduce_to_if` chases refs/annotations and stops at a kept multi-use binding,
/// so a `let`-named inner `if` is not peeled (its value lives in a slot). Only the DIRECT nested `if` is
/// collapsed here; deeper propagation happens because the rewritten branch re-lowers and can collapse
/// again.
fn collapse_repeated_cond(
    db: &mut Db,
    cond: StructId,
    branch: StructId,
    cond_is_true: bool,
) -> Option<StructId> {
    let (inner_cond, inner_then, inner_else) = crate::eval::reduce_to_if(db, branch)?;
    if core_equiv(db, cond, inner_cond) {
        // `c'` == `cond`: same truth → `cond_is_true` picks the inner then, else the inner else.
        Some(if cond_is_true { inner_then } else { inner_else })
    } else if is_negation_of(db, cond, inner_cond) {
        // `c'` == `!cond`: OPPOSITE truth → flip which arm survives.
        Some(if cond_is_true { inner_else } else { inner_then })
    } else {
        None
    }
}

/// Whether the cores at `a` and `b` are boolean NEGATIONS of each other — one is `Core::Not { operand }`
/// with `operand` `core_equiv` to the other. Both orders are tried (`a` is `(not b)` or `b` is `(not a)`).
/// `not` is total and pure, and `core_equiv` matches only pure cores, so a matched pair is two pure
/// booleans of exactly opposite truth. Used by `collapse_repeated_cond` to propagate a known condition
/// into a nested `if` guarded by that condition's negation.
fn is_negation_of(db: &mut Db, a: StructId, b: StructId) -> bool {
    let one_way = |db: &mut Db, x: StructId, y: StructId| -> bool {
        matches!(core_of(db, x), Core::Not { operand } if core_equiv(db, operand, y))
    };
    one_way(db, a, b) || one_way(db, b, a)
}

/// Lower an ARITHMETIC application: FOLD it when its operands fold to constants — evaluate at compile
/// time with a CHECKED operation, so a provable overflow is a build error (CDZ0304 poison) rather than
/// a shipped runtime trap (`reference-compiler.md` §A Compile-Provable Trap Fails The Build). An
/// operand that is not a constant stays a runtime `Arith` (its wasm op selected from the solved width
/// at selection); a poison operand propagates.
/// Whether the arithmetic application at `id` is over QUANTITIES whose inner numeric type is a FLOAT —
/// so `+`/`-`/`*`/`/` must run the float operation on the erased inner values (a quantity's operator is
/// polymorphic over its inner numeric). Reads the node's solved type: `+`/`-`/comparison keep the
/// operands' unit so the RESULT is `(Qty Float …)`; `*`/`/` compose units so the result is still a
/// quantity — either way a `Ty::Qty { inner: Float }` result marks the float case. Falls back to the
/// first operand's type when the result is not itself a quantity (a `Qty.value`-peeled position).
fn quantity_inner_is_float(db: &mut Db, id: StructId, args: &[StructId]) -> bool {
    let is_qty_float = |t: &crate::ty::Ty| matches!(t, crate::ty::Ty::Qty { inner, .. } if matches!(**inner, crate::ty::Ty::Float(_)));
    if is_qty_float(&crate::infer::type_of(db, id)) {
        return true;
    }
    // The result may not be a quantity (a comparison yields Bool); check the first operand.
    args.first()
        .map(|&a| is_qty_float(&crate::infer::type_of(db, a)))
        .unwrap_or(false)
}

/// Whether the operation is over a quantity with a RATIONAL inner magnitude — `(Qty Rational u)`. Checked
/// (like `quantity_inner_is_float`) on the result then the first operand, so a comparison (Bool result)
/// still routes by its operand. Such an op runs EXACT RATIONAL arithmetic on the erased inner magnitudes.
fn quantity_inner_is_rational(db: &mut Db, id: StructId, args: &[StructId]) -> bool {
    let is_qty_rat = |t: &crate::ty::Ty| matches!(t, crate::ty::Ty::Qty { inner, .. } if matches!(**inner, crate::ty::Ty::Rational));
    if is_qty_rat(&crate::infer::type_of(db, id)) {
        return true;
    }
    args.first()
        .map(|&a| is_qty_rat(&crate::infer::type_of(db, a)))
        .unwrap_or(false)
}

/// Whether this `+`/`-`/`*`/`/` is over a quantity with a BIGINT inner magnitude — a `(Qty BigInt u)`.
/// A BigInt inner is a heap HANDLE (i32), not a fixnum, so its arithmetic must route to the runtime
/// `bigint-*` ops (`lower_bigint_arith`), exactly as a bare-BigInt `+` does — NOT the default integer
/// path (which treats the operand as an i64 fixnum → an i32/i64 miscompile). The plain `bigint_operand`
/// check misses this because a quantity's solved type is `Ty::Qty { inner: BigInt }`, not `Ty::BigInt`;
/// this peels the quantity to see the inner, mirroring `quantity_inner_is_rational`.
fn quantity_inner_is_bigint(db: &mut Db, id: StructId, args: &[StructId]) -> bool {
    let is_qty_big = |t: &crate::ty::Ty| matches!(t, crate::ty::Ty::Qty { inner, .. } if matches!(**inner, crate::ty::Ty::BigInt));
    if is_qty_big(&crate::infer::type_of(db, id)) {
        return true;
    }
    args.first()
        .map(|&a| is_qty_big(&crate::infer::type_of(db, a)))
        .unwrap_or(false)
}

/// Whether the two operands are quantities of the SAME dimension at DIFFERENT scales — a mixed-unit
/// combine that must convert to the reference (`1 km + 500 m`). `false` when either is not a quantity,
/// they differ in dimension (that is CDZ0501, reported in `infer`), or the scales are equal (the common
/// same-unit case — no conversion, the ordinary arith path). Reads the operands' solved units.
fn quantity_scales_differ(db: &mut Db, args: &[StructId]) -> bool {
    let (a, b) = (
        crate::infer::type_of(db, args[0]),
        crate::infer::type_of(db, args[1]),
    );
    match (&a, &b) {
        (crate::ty::Ty::Qty { unit: ua, .. }, crate::ty::Ty::Qty { unit: ub, .. }) => {
            ua.same_dimension(ub) && ua.scale() != ub.scale()
        }
        _ => false,
    }
}

/// Whether the two operands are quantities of the SAME unit — same dimension AND same scale (`meter` vs
/// `meter`, NOT `km` vs `m`). Routes a same-unit quantity COMPARISON through the erased-magnitude compare
/// (the units are identical, so no conversion). The both-quantity complement of `quantity_scales_differ`
/// (which routes the DIFFERENT-scale case through conversion); a cross-DIMENSION pair is neither (CDZ0501
/// in `infer`). Reads the operands' solved units.
fn quantity_same_unit_pair(db: &mut Db, args: &[StructId]) -> bool {
    let (a, b) = (
        crate::infer::type_of(db, args[0]),
        crate::infer::type_of(db, args[1]),
    );
    match (&a, &b) {
        (crate::ty::Ty::Qty { unit: ua, .. }, crate::ty::Ty::Qty { unit: ub, .. }) => {
            ua.same_dimension(ub) && ua.scale() == ub.scale()
        }
        _ => false,
    }
}

/// Lower a MIXED-UNIT combine `(op a b)` where `a` and `b` are quantities of one dimension at different
/// scales: convert EACH operand to the dimension's REFERENCE unit by its exact scale (`value * num /
/// den` in the inner type T), then apply `op` at the reference. Folds the CONSTANT case — the operands
/// erase to a `Core::ConstInt`/`ConstFloat`, each scaled exactly (Int) or by round-to-nearest (Float),
/// per spec §48 ("los[es] precision only where the underlying numeric type is itself inexact"). A
/// non-constant operand DECLINES (the runtime scale-multiply is a later increment). `+`/`-` fold to the
/// converted numeric (rendered back as `(Qty <sum> <reference-unit>)` by the value form); a comparison
/// folds to a `ConstBool`.
fn lower_quantity_combine(
    db: &mut Db,
    id: StructId,
    op: Prim,
    lhs: StructId,
    rhs: StructId,
) -> Core {
    // Each operand's scale to the reference (num/den) — read off its solved unit.
    let scale_of = |db: &mut Db, arg: StructId| -> Option<(i128, i128)> {
        match crate::infer::type_of(db, arg) {
            crate::ty::Ty::Qty { unit, .. } => Some(unit.scale()),
            _ => None,
        }
    };
    let (ln, ld) = match scale_of(db, lhs) {
        Some(s) => s,
        None => return Core::Poison(Reject::decline("mixed-unit combine: non-quantity operand")),
    };
    let (rn, rd) = match scale_of(db, rhs) {
        Some(s) => s,
        None => return Core::Poison(Reject::decline("mixed-unit combine: non-quantity operand")),
    };
    // The inner numeric type decides how conversion + the op run.
    let inner_is_float = matches!(
        crate::infer::type_of(db, lhs),
        crate::ty::Ty::Qty { inner, .. } if matches!(*inner, crate::ty::Ty::Float(_))
    );
    let lc = core_of(db, lhs);
    let rc = core_of(db, rhs);
    if inner_is_float {
        // FLOAT: convert each to the reference by `v * num / den` (rounding), then run the op.
        if let (Some(lv), Some(rv)) = (float_of_core(&lc), float_of_core(&rc)) {
            // CONSTANT operands — fold exactly at compile time.
            let l = lv * (ln as f64) / (ld as f64);
            let r = rv * (rn as f64) / (rd as f64);
            return fold_float_combine(op, l, r);
        }
        // RUNTIME operand(s) — synthesize the scale conversion as real float arithmetic and lower it.
        return lower_runtime_combine(db, op, lhs, (ln, ld), rhs, (rn, rd), true);
    }
    // RATIONAL inner: convert each operand to the reference EXACTLY — `v * (num/den)` =
    // `(vn*num)/(vd*den)`, renormalized — then combine (`+`/`-`/`*`/`/` or a comparison) exactly. This is
    // THE mixing case done without rounding: `1 inch + 1 mm` = 127/5000 + 1/1000 = 33/1250 m, exact.
    let inner_is_rational = matches!(
        crate::infer::type_of(db, lhs),
        crate::ty::Ty::Qty { inner, .. } if matches!(*inner, crate::ty::Ty::Rational)
    );
    if inner_is_rational {
        if let (Core::ConstRational(lvn, lvd), Core::ConstRational(rvn, rvd)) = (&lc, &rc) {
            let l = normalized_rational(
                lvn.mul(&IntValue::from_i128(ln)),
                lvd.mul(&IntValue::from_i128(ld)),
            );
            let r = normalized_rational(
                rvn.mul(&IntValue::from_i128(rn)),
                rvd.mul(&IntValue::from_i128(rd)),
            );
            // Fold the op over the two converted rationals directly (they are constant `ConstRational`s).
            let (Core::ConstRational(ln2, ld2), Core::ConstRational(rn2, rd2)) = (&l, &r) else {
                return Core::Poison(Reject::decline("mixed-unit rational conversion trapped"));
            };
            return match op {
                Prim::Add => normalized_rational(ln2.mul(rd2).add(&rn2.mul(ld2)), ld2.mul(rd2)),
                Prim::Sub => normalized_rational(ln2.mul(rd2).sub(&rn2.mul(ld2)), ld2.mul(rd2)),
                Prim::Mul => normalized_rational(ln2.mul(rn2), ld2.mul(rd2)),
                Prim::Div => normalized_rational(ln2.mul(rd2), ld2.mul(rn2)),
                // A comparison of the two converted rationals: `a/b <=> c/d ⇔ a·d <=> c·b`.
                Prim::Lt | Prim::Gt | Prim::Le | Prim::Ge | Prim::Eq => {
                    let ord = ln2.mul(rd2).cmp(&rn2.mul(ld2));
                    Core::ConstBool(compare_ord(op, ord))
                }
                _ => Core::Poison(Reject::decline("mixed-unit rational: unsupported operator")),
            };
        }
        // RUNTIME — convert each operand to the reference by `value * (Rational.of num den)` (an EXACT
        // rational multiply, no rounding) via `convert_operand_ast_rational`, then run the combine op on
        // the two converted Rationals (which lowers through the runtime `rational-*` ops). Mirrors the
        // BigInt runtime arm below; the exact-rational analogue of the Int/BigInt runtime scale multiply.
        let lconv = match convert_operand_ast_rational(db, lhs, ln, ld) {
            Some(n) => n,
            None => {
                return Core::Poison(Reject::decline(
                    "runtime mixed-unit Rational combine over a non-Qty.of operand is not supported",
                ));
            }
        };
        let rconv = match convert_operand_ast_rational(db, rhs, rn, rd) {
            Some(n) => n,
            None => {
                return Core::Poison(Reject::decline(
                    "runtime mixed-unit Rational combine over a non-Qty.of operand is not supported",
                ));
            }
        };
        let head = db.push_name(combine_op_name(op));
        let app = db.push_list(vec![head, lconv, rconv]);
        return core_of(db, app);
    }
    // BIGINT inner: convert each operand to the reference by `value * num / den` in UNBOUNDED bigint
    // arithmetic (the heap-handle magnitudes can't take the i128 fold below — that path is for fixnum
    // Int). A constant pair folds exactly over `IntValue` bignum (mul + truncating divmod); a runtime one
    // synthesizes `value * (BigInt.of num) / (BigInt.of den)` per operand (`convert_operand_ast_bigint`)
    // so the conversion `*`/`/` route to the runtime bigint ops, then the combine op runs on the two
    // converted BigInt values. This is the mixed-scale analogue of the BigInt Unit.in arm.
    let inner_is_bigint = matches!(
        crate::infer::type_of(db, lhs),
        crate::ty::Ty::Qty { inner, .. } if matches!(*inner, crate::ty::Ty::BigInt)
    );
    if inner_is_bigint {
        // Convert `v * n / d` over IntValue bignum (exact mul, truncating divmod). `None` if a value is
        // not a constant BigInt (→ runtime path below).
        let conv_const = |v: &IntValue, n: i128, d: i128| -> Option<IntValue> {
            let scaled = v.mul(&IntValue::from_i128(n));
            scaled.divmod(&IntValue::from_i128(d)).map(|(q, _)| q)
        };
        if let (Core::ConstInt(lv), Core::ConstInt(rv)) = (&lc, &rc)
            && let (Some(l), Some(r)) = (conv_const(lv, ln, ld), conv_const(rv, rn, rd))
        {
            // Both converted to reference-unit BigInts — fold the op (arith → a BigInt ConstInt;
            // comparison → a ConstBool via the exact bignum compare).
            return match op {
                Prim::Add => Core::ConstInt(l.add(&r)),
                Prim::Sub => Core::ConstInt(l.sub(&r)),
                Prim::Lt | Prim::Gt | Prim::Le | Prim::Ge | Prim::Eq => {
                    Core::ConstBool(compare_ord(op, l.cmp(&r)))
                }
                _ => Core::Poison(Reject::decline("mixed-unit bigint: unsupported operator")),
            };
        }
        // RUNTIME — synthesize `(op (value_l * (BigInt.of ln) / (BigInt.of ld)) (value_r * …))` and lower.
        let lconv = match convert_operand_ast_bigint(db, lhs, ln, ld) {
            Some(n) => n,
            None => {
                return Core::Poison(Reject::decline(
                    "runtime mixed-unit BigInt combine over a non-Qty.of operand is not supported",
                ));
            }
        };
        let rconv = match convert_operand_ast_bigint(db, rhs, rn, rd) {
            Some(n) => n,
            None => {
                return Core::Poison(Reject::decline(
                    "runtime mixed-unit BigInt combine over a non-Qty.of operand is not supported",
                ));
            }
        };
        let head = db.push_name(combine_op_name(op));
        let app = db.push_list(vec![head, lconv, rconv]);
        return core_of(db, app);
    }
    // INT (and other exact inner): convert each by `v * num / den` over i128 (exact; truncates on a
    // non-whole ratio, per opting into integer math).
    if let (Some(lv), Some(rv)) = (int_of_core(&lc), int_of_core(&rc)) {
        // CONSTANT operands — fold.
        let conv = |v: i128, n: i128, d: i128| -> Option<i128> { v.checked_mul(n).map(|x| x / d) };
        let (l, r) = match (conv(lv, ln, ld), conv(rv, rn, rd)) {
            (Some(l), Some(r)) => (l, r),
            _ => {
                return Core::Poison(Reject::coded(
                    Code::ConstTrap,
                    "mixed-unit conversion overflows the machine range",
                ));
            }
        };
        // Fold the op over the two reference-converted magnitudes, THEN range-check an arithmetic
        // result against the quantity's INNER integer width — exactly as `lower_arith` does for a bare
        // `+`/`-`. `fold_int_combine` evaluates over i128, so a NARROW overflow whose true result still
        // fits i128 (`100 + 100 = 200` over `(Qty Int8 u)`) folds to a valid `ConstInt` and would
        // otherwise slip through to a backend CDZ0302 ("a literal that doesn't fit") — and, because that
        // gate lives only in the backend, `cdz check` would MISS it entirely. It is a constant OPERATION
        // whose defined outcome is a TRAP (the sum overflows the type), NOT an out-of-range literal, so it
        // is CDZ0304 (`ConstTrap`) — the SAME code the bare `(+ (Int8.of 100) (Int8.of 100))` gets. The
        // inner width is read off `id`'s solved `Ty::Qty { inner: Int … }` (a comparison result is a Bool,
        // unaffected). Units are erased, so a quantity's arithmetic obeys the inner numeric type's rule.
        let folded = fold_int_combine(op, l, r);
        if let Core::ConstInt(ref r) = folded
            && let crate::ty::Ty::Qty { inner, .. } = crate::infer::type_of(db, id)
            && let crate::ty::Ty::Int(it) = *inner
            && !r.fits_width(it.ground_signed(), it.ground_width())
        {
            trace!(target: "rcdzc::fold", node = id.0, "mixed/same-unit Int combine result overflows the inner narrow width → CDZ0304");
            return Core::Poison(Reject::coded(
                Code::ConstTrap,
                "this constant arithmetic operation overflows its integer type (a \
                 compile-provable overflow traps)",
            ));
        }
        return folded;
    }
    // RUNTIME operand(s) — synthesize the scale conversion as real integer arithmetic and lower it.
    lower_runtime_combine(db, op, lhs, (ln, ld), rhs, (rn, rd), false)
}

/// The runtime path of a mixed-unit combine: synthesize `(op (convert lhs) (convert rhs))` as ordinary
/// arithmetic over the operands' ERASED magnitudes and lower it — the scale multiply the source denotes
/// by naming two units, emitted as real code (units-of-measure.md §A Unit Conversion Is The Arithmetic
/// The Source Denotes: "the scale multiply reaches the emitted component only when a magnitude is a
/// runtime value"). Each operand converts to the reference by `value * num / den` (float: `*.`/`/.`;
/// int: `*`/`/`), built from the quantity's value occurrence + synthesized constant factors. A
/// non-`Qty.of` operand (no reusable value occurrence) declines.
#[allow(clippy::too_many_arguments)]
fn lower_runtime_combine(
    db: &mut Db,
    op: Prim,
    lhs: StructId,
    (ln, ld): (i128, i128),
    rhs: StructId,
    (rn, rd): (i128, i128),
    is_float: bool,
) -> Core {
    let lconv = match convert_operand_ast(db, lhs, ln, ld, is_float) {
        Some(n) => n,
        None => {
            return Core::Poison(Reject::decline(
                "runtime mixed-unit combine over a non-Qty.of operand is not supported",
            ));
        }
    };
    let rconv = match convert_operand_ast(db, rhs, rn, rd, is_float) {
        Some(n) => n,
        None => {
            return Core::Poison(Reject::decline(
                "runtime mixed-unit combine over a non-Qty.of operand is not supported",
            ));
        }
    };
    // Build `(op-name lconv rconv)` with the ONE arithmetic/comparison operator (a float inner routes it
    // to float arithmetic by operand type) so it lowers through the ordinary arith/comparison path — the
    // converted operands are bare numerics.
    let op_name = combine_op_name(op);
    let head = db.push_name(op_name);
    let app = db.push_list(vec![head, lconv, rconv]);
    core_of(db, app)
}

/// The erased magnitude of a quantity `operand` as a reusable arena node. A directly-written `(Qty.of x
/// u)` yields its value occurrence `x` (`qty_value_occ`); ANY OTHER quantity expression — a `*`/`/`-
/// computed quantity, a let-bound one — is not a literal `Qty.of`, so fall back to `(Qty.value operand)`,
/// the explicit unwrap that re-lowers `operand` and erases the unit. Both are the erased inner numeric.
/// This is the shared fallback the mixed-scale `convert_operand_ast*` helpers and `lower_qty_pow` use so
/// a runtime combine/conversion works over a COMPUTED quantity operand, not only a literal `Qty.of` (the
/// literal-only `qty_value_occ` alone declined those — e.g. `(+ (* (Qty n km) 2N) (Qty 500 m))`). Total:
/// a non-quantity operand still resolves through `Qty.value`'s own lowering (which faults if inapt).
fn qty_magnitude_occ(db: &mut Db, operand: StructId) -> StructId {
    match crate::eval::qty_value_occ(db, operand) {
        Some(v) => v,
        None => {
            let dot = db.push_name(".");
            let qty = db.push_name("Qty");
            let value_key = db.push_name("value");
            let qty_value_head = db.push_list(vec![dot, qty, value_key]);
            db.push_list(vec![qty_value_head, operand])
        }
    }
}

/// Synthesize an arena node for a quantity operand's magnitude CONVERTED to the reference: `value * num
/// / den`, using the ordinary numeric operators (float `*.`/`/.` for a float inner, int `*`/`/`
/// otherwise). `value` is the quantity's magnitude (`qty_magnitude_occ` — a literal `Qty.of`'s value
/// occurrence, else `(Qty.value operand)`). When the scale is 1/1 the value passes through unconverted.
fn convert_operand_ast(
    db: &mut Db,
    operand: StructId,
    num: i128,
    den: i128,
    is_float: bool,
) -> Option<StructId> {
    let value = qty_magnitude_occ(db, operand);
    // Scale 1/1 — no conversion, use the value as-is.
    if num == 1 && den == 1 {
        return Some(value);
    }
    // The ONE arithmetic operator `*`/`/` — a float `num.0` operand routes it to float arithmetic by the
    // operand type (there is no distinct `*.`/`/.`); `is_float` only picks the literal spelling.
    // `(* value num)` — multiply by the scale numerator (a `num.0` float literal for a float inner).
    let mut node = value;
    if num != 1 {
        let n_lit = num_literal(db, num, is_float);
        let mul_head = db.push_name("*");
        node = db.push_list(vec![mul_head, node, n_lit]);
    }
    // `(/ … den)` — divide by the denominator.
    if den != 1 {
        let d_lit = num_literal(db, den, is_float);
        let div_head = db.push_name("/");
        node = db.push_list(vec![div_head, node, d_lit]);
    }
    Some(node)
}

/// The runtime BigInt analogue of [`convert_operand_ast`]: synthesize `value * (BigInt.of num) /
/// (BigInt.of den)` for a `Unit.in` over a BigInt-magnitude quantity. The value occurrence is q's erased
/// BigInt magnitude (a heap handle); the scale factors are `(BigInt.of …)` so the `*`/`/` see a BigInt
/// operand and route to the runtime `bigint-*` ops (`bigint_operand` dispatch). Division TRUNCATES toward
/// zero (integer/bigint division), matching the fixed-Int arm. `None` if q's value occurrence is not
/// recoverable (a non-`Qty.of` runtime magnitude — a later increment).
fn convert_operand_ast_bigint(
    db: &mut Db,
    operand: StructId,
    num: i128,
    den: i128,
) -> Option<StructId> {
    let value = qty_magnitude_occ(db, operand);
    // `(BigInt.of <n>)` — a bigint scale literal. `BigInt.of` is member access `(. BigInt of)`.
    let bigint_of = |db: &mut Db, n: i128| -> StructId {
        let dot = db.push_name(".");
        let bigint = db.push_name("BigInt");
        let of = db.push_name("of");
        let head = db.push_list(vec![dot, bigint, of]);
        let lit = db.push_atom(crate::ast::Leaf::Int {
            value: IntValue::from_i128(n),
            radix: crate::ast::Radix::Dec,
        });
        db.push_list(vec![head, lit])
    };
    let mut node = value;
    if num != 1 {
        let n_big = bigint_of(db, num);
        let mul_head = db.push_name("*");
        node = db.push_list(vec![mul_head, node, n_big]);
    }
    if den != 1 {
        let d_big = bigint_of(db, den);
        let div_head = db.push_name("/");
        node = db.push_list(vec![div_head, node, d_big]);
    }
    Some(node)
}

/// The runtime Rational analogue: synthesize `value * (Rational.of num den)` for a `Unit.in` over a
/// Rational-magnitude quantity. The value occurrence is q's erased Rational handle; the scale is a SINGLE
/// exact rational literal `(Rational.of num den)`, so the `*` is one runtime `rational-mul` (routed by
/// `rational_operand`, which peels Qty) — EXACT, no rounding and no separate divide (a rational carries
/// its own denominator). `Unit.in` UNWRAPS → a bare Rational. Scale 1/1 is the identity (value unchanged).
/// `None` if q's value occurrence is not recoverable (a non-`Qty.of` runtime magnitude — a later increment).
fn convert_operand_ast_rational(
    db: &mut Db,
    operand: StructId,
    num: i128,
    den: i128,
) -> Option<StructId> {
    let value = qty_magnitude_occ(db, operand);
    if num == 1 && den == 1 {
        return Some(value);
    }
    // `(Rational.of <num> <den>)` — an exact rational scale literal. `Rational.of` is member access.
    let dot = db.push_name(".");
    let rational = db.push_name("Rational");
    let of = db.push_name("of");
    let head = db.push_list(vec![dot, rational, of]);
    let n_lit = db.push_atom(crate::ast::Leaf::Int {
        value: IntValue::from_i128(num),
        radix: crate::ast::Radix::Dec,
    });
    let d_lit = db.push_atom(crate::ast::Leaf::Int {
        value: IntValue::from_i128(den),
        radix: crate::ast::Radix::Dec,
    });
    let scale = db.push_list(vec![head, n_lit, d_lit]);
    let mul_head = db.push_name("*");
    Some(db.push_list(vec![mul_head, value, scale]))
}

/// A synthesized numeric literal node for a machine integer `v` — a float decimal `v.0` when `is_float`,
/// else an integer literal. Used for the constant scale factors a runtime conversion multiplies by.
fn num_literal(db: &mut Db, v: i128, is_float: bool) -> StructId {
    if is_float {
        // Build the exact decimal for `v` (a whole number, always finite).
        match crate::ast::Decimal::from_f64(v as f64) {
            Some(d) => db.push_atom(crate::ast::Leaf::Float(d)),
            // Unreachable for a whole scale factor; fall back to an integer literal.
            None => db.push_atom(crate::ast::Leaf::Int {
                value: IntValue::from_i128(v),
                radix: crate::ast::Radix::Dec,
            }),
        }
    } else {
        db.push_atom(crate::ast::Leaf::Int {
            value: IntValue::from_i128(v),
            radix: crate::ast::Radix::Dec,
        })
    }
}

/// The ordinary arithmetic/comparison operator NAME for a mixed-unit combine `op` — the ONE operator
/// spelling per prim (`+`/`-`/`<`/…). A float inner routes `+`/`-` to float arithmetic by the operand
/// type at lowering (no distinct `+.`), so the spelling is inner-type-independent, exactly as the
/// comparisons already were.
fn combine_op_name(op: Prim) -> &'static str {
    match op {
        Prim::Add => "+",
        Prim::Sub => "-",
        Prim::Lt => "<",
        Prim::Gt => ">",
        Prim::Le => "<=",
        Prim::Ge => ">=",
        Prim::Eq => "=",
        // Only additive/comparison ops reach a mixed-unit combine.
        _ => "+",
    }
}

/// The `f64` a constant float/int core holds (a quantity's erased inner), for the float conversion fold.
fn float_of_core(c: &Core) -> Option<f64> {
    match c {
        Core::ConstFloat(d) => Some(f64::from_bits(d.to_f64_bits())),
        _ => None,
    }
}

/// The `i128` a constant int core holds (a quantity's erased inner), for the integer conversion fold.
fn int_of_core(c: &Core) -> Option<i128> {
    match c {
        Core::ConstInt(v) => v.to_i128(),
        _ => None,
    }
}

/// Lower `(Unit.in target q)` — convert q's erased magnitude from its unit to `target` by
/// `value * (q.scale / target.scale)` in the inner type T (Float rounds, Int exact/truncates). A no-op
/// when the scales are equal. Folds the constant case; a runtime magnitude declines (the emitted runtime
/// scale-multiply is a later increment). The dimensional check (target vs q dimension) is
/// `check_application`'s (CDZ0501); here q is assumed same-dimension.
///
/// The conversion is exactly the SCALE ARITHMETIC the source denotes by naming the two units — the ratio
/// of the operand's scale to the target's, nothing the dimensional layer adds. A constant magnitude is
/// converted at compile time (folded to a `ConstFloat`/`ConstInt`, no runtime arithmetic), and the result
/// is a BARE numeric core — the `Ty::Qty` dimension is erased whether or not the scale multiply survives.
//= spec/capabilities/units-of-measure.md#a-unit-conversion-is-the-arithmetic-the-source-denotes
//# A conversion between two units of one dimension MUST be the scale arithmetic the source denotes by naming those units, not additional arithmetic the dimensional layer introduces, so that the emitted arithmetic is what the program means rather than an overhead the check imposes.
//= spec/capabilities/units-of-measure.md#a-unit-conversion-is-the-arithmetic-the-source-denotes
//# A unit conversion whose operands are compile-time constants MUST be computed at compile time, so that a conversion between constant quantities contributes no runtime arithmetic.
//= spec/capabilities/units-of-measure.md#a-unit-conversion-is-the-arithmetic-the-source-denotes
//# The dimension a quantity carries MUST be erased whether or not a scale conversion is emitted, so that the type-level dimensional information never survives into the component even when the scale arithmetic does.
fn lower_unit_in(db: &mut Db, target: StructId, q: StructId) -> Core {
    // q's scale to the reference (read off its solved unit); the target's from `unit_of`.
    let (qn, qd) = match crate::infer::type_of(db, q) {
        crate::ty::Ty::Qty { unit, .. } => unit.scale(),
        _ => return Core::Poison(Reject::decline("Unit.in of a non-quantity")),
    };
    let (tn, td) = match crate::eval::unit_of(db, target) {
        Some(u) => u.scale(),
        None => return Core::Poison(Reject::decline("Unit.in target is not a unit")),
    };
    // The conversion factor is `q.scale / target.scale` = `(qn/qd) / (tn/td)` = `(qn*td) / (qd*tn)`.
    let inner_is_float = matches!(
        crate::infer::type_of(db, q),
        crate::ty::Ty::Qty { inner, .. } if matches!(*inner, crate::ty::Ty::Float(_))
    );
    // The conversion factor is `q.scale / target.scale` = `(qn*td) / (qd*tn)` — one combined ratio.
    let num = match qn.checked_mul(td) {
        Some(n) => n,
        None => {
            return Core::Poison(Reject::coded(
                Code::ConstTrap,
                "Unit.in conversion overflows",
            ));
        }
    };
    let den = match qd.checked_mul(tn) {
        Some(d) if d != 0 => d,
        _ => {
            return Core::Poison(Reject::coded(
                Code::ConstTrap,
                "Unit.in conversion overflows",
            ));
        }
    };
    let qc = core_of(db, q);
    // A RATIONAL magnitude converts EXACTLY: `value * (num/den)` = `(vn*num)/(vd*den)`, renormalized. This
    // is the whole point of a rational-magnitude unit (`1 inch in meter` = exactly 127/5000 m, no rounding).
    let inner_is_rational = matches!(
        crate::infer::type_of(db, q),
        crate::ty::Ty::Qty { inner, .. } if matches!(*inner, crate::ty::Ty::Rational)
    );
    if inner_is_rational {
        if let Core::ConstRational(vn, vd) = &qc {
            let scaled_num = vn.mul(&IntValue::from_i128(num));
            let scaled_den = vd.mul(&IntValue::from_i128(den));
            return normalized_rational(scaled_num, scaled_den);
        }
        // RUNTIME Rational — synthesize `value * (Rational.of num den)` and re-lower; the `*` sees a
        // Rational operand and routes to the runtime `rational-*` op (`quantity_inner_is_rational` /
        // `rational_operand` dispatch, both peel Qty). EXACT (rational multiply, no rounding); `Unit.in`
        // UNWRAPS → a bare Rational. Scale 1/1 is the identity (the value unchanged). Mirrors the BigInt
        // runtime arm.
        match convert_operand_ast_rational(db, q, num, den) {
            Some(node) => return core_of(db, node),
            None => {
                return Core::Poison(Reject::decline(
                    "Unit.in over a runtime non-Qty.of Rational magnitude is not supported",
                ));
            }
        }
    }
    // A BIGINT magnitude converts as `value * num / den` in UNBOUNDED bigint arithmetic — the value is a
    // heap handle, so the scale factors are materialized as `BigInt.of` and the `*`/`/` route to the
    // runtime bigint ops (`quantity_inner_is_bigint`/`bigint_operand` dispatch). `Unit.in` UNWRAPS, so the
    // result is a bare BigInt. A CONSTANT bigint folds via `IntValue` bignum (exact mul + truncating
    // divmod); a runtime one emits the bigint ops. A non-whole ratio TRUNCATES (integer division, the
    // same rule the fixed-Int arm uses). Scale 1/1 (reference→reference) is the identity — the value
    // unchanged.
    let inner_is_bigint = matches!(
        crate::infer::type_of(db, q),
        crate::ty::Ty::Qty { inner, .. } if matches!(*inner, crate::ty::Ty::BigInt)
    );
    if inner_is_bigint {
        if num == 1 && den == 1 {
            // Identity conversion — the erased BigInt value unchanged (still a bare BigInt).
            return qc;
        }
        if let Core::ConstInt(v) = &qc {
            // CONSTANT bigint — fold `v * num / den` exactly over IntValue bignum (mul is exact; div
            // truncates toward zero). Stays a BigInt-typed ConstInt (the emit choke-point materializes it).
            let scaled = v.mul(&IntValue::from_i128(num));
            return match scaled.divmod(&IntValue::from_i128(den)) {
                Some((quotient, _rem)) => Core::ConstInt(quotient),
                None => Core::Poison(Reject::coded(
                    Code::ConstTrap,
                    "Unit.in conversion divides by zero",
                )),
            };
        }
        // RUNTIME bigint — synthesize `(/ (* value (BigInt.of num)) (BigInt.of den))` and re-lower; the
        // `*`/`/` see a BigInt operand and route to the runtime bigint ops.
        match convert_operand_ast_bigint(db, q, num, den) {
            Some(node) => return core_of(db, node),
            None => {
                return Core::Poison(Reject::decline(
                    "Unit.in over a runtime non-Qty.of BigInt magnitude is not supported",
                ));
            }
        }
    }
    if inner_is_float {
        if let Some(v) = float_of_core(&qc) {
            // CONSTANT float magnitude — fold the conversion.
            let converted = v * (num as f64) / (den as f64);
            return match crate::ast::Decimal::from_f64(converted) {
                Some(d) => Core::ConstFloat(d),
                None => Core::Poison(Reject::decline("Unit.in float result has no finite form")),
            };
        }
    } else if let Some(v) = int_of_core(&qc) {
        // CONSTANT int magnitude — fold `v * num / den` (exact/truncating).
        return match v.checked_mul(num) {
            Some(scaled) => Core::ConstInt(IntValue::from_i128(scaled / den)),
            None => Core::Poison(Reject::coded(
                Code::ConstTrap,
                "Unit.in conversion overflows",
            )),
        };
    }
    // RUNTIME magnitude — synthesize `value * num / den` as real arithmetic over q's value occurrence
    // and lower it (the same scale-multiply the constant path folds, emitted as code).
    match convert_operand_ast(db, q, num, den, inner_is_float) {
        Some(node) => core_of(db, node),
        None => Core::Poison(Reject::decline(
            "Unit.in over a runtime non-Qty.of magnitude is not supported",
        )),
    }
}

/// Lower `(Qty.pow q n)` — raise q's erased magnitude to the `n`th power over the inner numeric type.
/// The unit is a compile-time concern (the solved `Ty::Qty` already carries `unit^n`); at runtime this
/// is just `value * value * … ` (`|n|` factors), synthesized as ordinary arithmetic over q's value
/// occurrence and re-lowered (so the constant case FOLDS through the normal arith path and a runtime
/// magnitude emits the multiplies). `n = 0` is the dimensionless `1` (the multiplicative identity in the
/// inner type). A NEGATIVE `n` is the reciprocal `1 / value^|n|` (an inverse unit like a frequency
/// `second⁻¹`) — the division runs in the inner type, so Float divides and Int TRUNCATES (`1 / 8 = 0`),
/// the documented "precision loss only where the numeric type is itself inexact / integer truncates".
fn lower_qty_pow(db: &mut Db, q: StructId, exp: StructId) -> Core {
    let inner = match crate::infer::type_of(db, q) {
        crate::ty::Ty::Qty { inner, .. } => *inner,
        other => other,
    };
    let inner_is_float = matches!(inner, crate::ty::Ty::Float(_));
    let inner_is_bigint = matches!(inner, crate::ty::Ty::BigInt);
    let inner_is_rational = matches!(inner, crate::ty::Ty::Rational);
    let n = match crate::resolve::resolved_of(db, exp) {
        crate::resolved::Resolved::Int(v) => match v.to_i64() {
            Some(n) => n,
            None => return Core::Poison(Reject::decline("Qty.pow exponent out of range")),
        },
        _ => {
            return Core::Poison(Reject::decline(
                "Qty.pow exponent is not a compile-time integer",
            ));
        }
    };
    // The multiplicative identity `1` in q's INNER numeric type — `1.0` for a float inner, `(BigInt.of 1)`
    // for a BigInt inner, `(Rational.of 1 1)` for a Rational inner, else the fixed-width int `1`. The inner
    // type matters because the negative-exponent reciprocal `1 / value^|n|` (and the `n = 0` identity) must
    // be in the SAME numeric type as `value`: a bare Int `1` over a BigInt/Rational `value` is a numeric
    // mismatch (`Int64` vs `BigInt`), which slips past the check inside a quantity and surfaces as a backend
    // ownership error on the reciprocal divide (`1` and the heap handle disagree). Build `1` in-type here.
    let inner_one = |db: &mut Db| -> StructId {
        if inner_is_bigint {
            let dot = db.push_name(".");
            let bigint = db.push_name("BigInt");
            let of = db.push_name("of");
            let head = db.push_list(vec![dot, bigint, of]);
            let lit = db.push_atom(crate::ast::Leaf::Int {
                value: IntValue::from_i128(1),
                radix: crate::ast::Radix::Dec,
            });
            db.push_list(vec![head, lit])
        } else if inner_is_rational {
            let dot = db.push_name(".");
            let rational = db.push_name("Rational");
            let of = db.push_name("of");
            let head = db.push_list(vec![dot, rational, of]);
            let n_lit = db.push_atom(crate::ast::Leaf::Int {
                value: IntValue::from_i128(1),
                radix: crate::ast::Radix::Dec,
            });
            let d_lit = db.push_atom(crate::ast::Leaf::Int {
                value: IntValue::from_i128(1),
                radix: crate::ast::Radix::Dec,
            });
            db.push_list(vec![head, n_lit, d_lit])
        } else {
            num_literal(db, 1, inner_is_float)
        }
    };
    // `n = 0` — the dimensionless identity `1` (in the inner type).
    if n == 0 {
        let one = inner_one(db);
        return core_of(db, one);
    }
    // The erased magnitude to raise: a literal `(Qty.of x u)` yields its value occurrence `x`, ANY OTHER
    // quantity expression (a `/`-computed velocity, a let-bound quantity) falls back to `(Qty.value q)`
    // (`qty_magnitude_occ` — the shared literal-or-unwrap the mixed-scale converters also use). Both are
    // the erased inner numeric; `Qty.pow` raises that.
    let value = qty_magnitude_occ(db, q);
    // Build `value^|n|` = `(* (* … value value) value)` — `|n|` copies, left-nested — with the ONE
    // multiply operator `*`; a float `value` routes it to float arithmetic by the operand type at
    // lowering (no distinct `*.`), so the spelling is inner-type-independent.
    let mut node = value;
    for _ in 1..n.unsigned_abs() {
        let mul_head = db.push_name("*");
        node = db.push_list(vec![mul_head, node, value]);
    }
    // A negative exponent is the reciprocal `1 / value^|n|` (the ONE `/` operator, float-dispatched by
    // its operand type); a positive one is the power itself. Lower the synthesized node through the
    // ordinary arith path (so the constant case folds and a runtime magnitude emits the multiplies/div).
    if n < 0 {
        let one = inner_one(db);
        let div_head = db.push_name("/");
        node = db.push_list(vec![div_head, one, node]);
    }
    core_of(db, node)
}

/// Apply `op` to two converted FLOAT reference values, producing the result core: `+`/`-` a
/// `ConstFloat`, a comparison a `ConstBool`.
fn fold_float_combine(op: Prim, l: f64, r: f64) -> Core {
    match op {
        Prim::Add | Prim::Sub => {
            let v = if matches!(op, Prim::Add) {
                l + r
            } else {
                l - r
            };
            match crate::ast::Decimal::from_f64(v) {
                Some(d) => Core::ConstFloat(d),
                None => Core::Poison(Reject::decline(
                    "mixed-unit float result has no finite form",
                )),
            }
        }
        Prim::Lt => Core::ConstBool(l < r),
        Prim::Gt => Core::ConstBool(l > r),
        Prim::Le => Core::ConstBool(l <= r),
        Prim::Ge => Core::ConstBool(l >= r),
        Prim::Eq => Core::ConstBool(l == r),
        _ => Core::Poison(Reject::decline("unexpected op in mixed-unit float combine")),
    }
}

/// Apply `op` to two converted INT reference values, producing the result core: `+`/`-` a `ConstInt`, a
/// comparison a `ConstBool`.
fn fold_int_combine(op: Prim, l: i128, r: i128) -> Core {
    let arith = |v: Option<i128>| match v {
        Some(n) => Core::ConstInt(IntValue::from_i128(n)),
        None => Core::Poison(Reject::coded(
            Code::ConstTrap,
            "mixed-unit result overflows",
        )),
    };
    match op {
        Prim::Add => arith(l.checked_add(r)),
        Prim::Sub => arith(l.checked_sub(r)),
        Prim::Lt => Core::ConstBool(l < r),
        Prim::Gt => Core::ConstBool(l > r),
        Prim::Le => Core::ConstBool(l <= r),
        Prim::Ge => Core::ConstBool(l >= r),
        Prim::Eq => Core::ConstBool(l == r),
        _ => Core::Poison(Reject::decline("unexpected op in mixed-unit int combine")),
    }
}

/// The wrong-arity CDZ0201 reject shared by the fixed-arity BINARY operators — integer arithmetic
/// (`lower_arith`), FLOAT arithmetic (`lower_float_arith`), and COMPARISON (`lower_comparison`). All three
/// take exactly 2 operands; an OVER-application (`(+ 1 2 3)`, `(< 1 2 3)`, `(+ 1.0 2.0 3.0)`) has a
/// mechanical repair: DELETE the first surplus operand (`args[2]`) — the fixpoint removes each extra until
/// exactly 2 remain. A TOO-FEW application (`(+ 1)`) has nothing to delete → no fix. Carrying the delete
/// fix on THIS authoritative CDZ0201 is what lets `dedup_faults` drop the sibling CDZ0203 over-application
/// (which anchors at the same surplus node), so a binary operator over-application reports ONCE, with the
/// fix — the parity `lower_arith` had but `lower_comparison`/`lower_float_arith` lacked (they double-reported).
fn binop_arity_reject(op: Prim, args: &[StructId]) -> Reject {
    let mut reject = Reject::coded(
        Code::Malformed,
        format!("{} takes exactly 2 operands", intrinsic_name(op)),
    );
    if args.len() > 2 {
        reject = reject.with_fix(crate::diag::Fix::delete_heuristic(
            args[2],
            "remove the extra operand",
        ));
    }
    reject
}

/// Build a NORMALIZED `Core::ConstRational` from a raw numerator/denominator pair, or `Core::Poison`
/// (a runtime TRAP) on a zero denominator. Normalization (numeric-model.md §An Exact Rational Has A
/// Canonical Normalized Form): divide both by `gcd(|num|, |den|)`, force the denominator strictly
/// positive (flip both signs if it is negative), so the sign lives on the numerator and equal values
/// share one byte form. `0/d` normalizes to `0/1`.
fn normalized_rational(num: crate::ast::IntValue, den: crate::ast::IntValue) -> Core {
    use crate::ast::IntValue;
    if den.is_zero() {
        // A zero denominator has no value — a provable constant trap (CDZ0304, the rational analogue of a
        // constant ÷0). Name the repair like the sibling divide-by-zero CDZ0304 ("use a nonzero divisor")
        // rather than dead-ending at the bare fault: a rational `n/d` denotes a number only when `d` is
        // nonzero. The `contains("zero denominator")` lead stays stable for any matcher; the actionable
        // tail is additive. (`trap_kind` does not classify this reason, so no runtime-trap grading depends
        // on the exact text.)
        return Core::Poison(Reject::coded(
            Code::ConstTrap,
            "a rational with a zero denominator has no value — use a nonzero denominator",
        ));
    }
    if num.is_zero() {
        return Core::ConstRational(IntValue::zero(), IntValue::from_i64(1));
    }
    // Reduce to lowest terms.
    let g = num.gcd(&den); // non-negative
    let (mut n, mut d) = match (num.divmod(&g), den.divmod(&g)) {
        (Some((qn, _)), Some((qd, _))) => (qn, qd),
        _ => (num, den), // g is nonzero here (num,den both nonzero), so this never fires
    };
    // Force the denominator strictly positive: if it is negative, flip BOTH signs.
    if d.negative {
        n = n.neg();
        d = d.neg();
    }
    Core::ConstRational(n, d)
}

/// Fold a numeric LITERAL to a normalized `Core::ConstRational` — the value an annotation `(: lit
/// Rational)` (and, later, the `R` literal suffix) grounds to. An integer `k` is the exact `k/1`; a
/// decimal `significand·10^exp` is the exact `significand / 10^|exp|` (LOSSLESS — a `Decimal` captures
/// the source exactly, so `0.5` is precisely `1/2`, never a rounded `f64`): a non-negative exponent
/// scales the numerator (`12·10^2 / 1`), a negative one is the denominator `10^|exp|` (`5 / 10^1` for
/// `0.5`). `normalized_rational` reduces to lowest terms + puts the sign on the numerator. Returns
/// `None` for a non-literal expression (the annotation then erases normally). Only reached when the
/// annotation type is `Rational` (checked by the `Annot` arm).
/// If `id` is a numeric literal WRITTEN in a `(pragma default-fraction Rational)` module (recorded in the
/// load-time `default_fraction_literals` map, keyed by the ORIGINAL node so it survives β-copy), ground it
/// to a `Core::ConstRational` — the lowering side of the fraction default, reusing the annotation path's
/// [`rational_from_literal`]. `None` for a literal with no fraction default, so the caller keeps the
/// ordinary `ConstInt`/`ConstFloat`. The map only holds a `<T>` that reduced to `Rational` at load; we
/// re-check nothing here (the map's presence IS the "this literal defaults to Rational" fact — the same
/// map `infer` consulted to type it `Ty::Rational`, so the type and value stay in lockstep).
fn default_fraction_rational(db: &mut Db, id: StructId) -> Option<Core> {
    if !db.default_fraction_literals.contains_key(&id) {
        return None;
    }
    // GUARD against an annotation override: the map records every numeric literal WRITTEN in the module,
    // including one inside `(: 5 Int64)`. For an annotated literal the `Annot` node fixes the type to
    // Int64, so grounding the inner literal to a `ConstRational` here would emit a rational VALUE for an
    // Int64-typed node — a miscompile (an invalid component). Fold to a rational ONLY when the literal's
    // SOLVED type is actually `Rational` (the unconstrained case the default governs); an annotation that
    // constrained it away from Rational leaves `type_of` ≠ Rational, so we keep the ordinary const.
    if !matches!(crate::infer::type_of(db, id), crate::ty::Ty::Rational) {
        return None;
    }
    rational_from_literal(db, id)
}

fn rational_from_literal(db: &mut Db, expr: StructId) -> Option<Core> {
    use crate::ast::IntValue;
    // 10^k as an IntValue (k ≥ 0), by repeated multiply — no bignum dep, and `k` is a literal's decimal
    // digit count, so it is small.
    fn ten_pow(k: u64) -> IntValue {
        let ten = IntValue::from_i64(10);
        let mut acc = IntValue::from_i64(1);
        for _ in 0..k {
            acc = acc.mul(&ten);
        }
        acc
    }
    match crate::resolve::resolved_of(db, expr) {
        // An integer literal `k` grounds to `k/1`.
        crate::resolved::Resolved::Int(v) => Some(normalized_rational(v, IntValue::from_i64(1))),
        // A decimal `significand·10^exp` grounds to the exact fraction. The significand shares
        // `IntValue`'s big-endian magnitude representation, so it lifts directly (its sign is on `d.negative`).
        crate::resolved::Resolved::Float(d) => {
            let sig = IntValue {
                negative: d.negative,
                magnitude: d.significand.clone(),
            };
            let (num, den) = if d.exponent >= 0 {
                (sig.mul(&ten_pow(d.exponent as u64)), IntValue::from_i64(1))
            } else {
                (sig, ten_pow((-d.exponent) as u64))
            };
            Some(normalized_rational(num, den))
        }
        _ => None,
    }
}

/// Lower `Rational.of n d` — fold a constant numerator/denominator pair to a normalized
/// `Core::ConstRational` (or a zero-denominator trap), else emit the RUNTIME `Core::RationalOfInts`
/// (widen each int to a BigInt + `rational-of`, which normalizes + traps on a zero denominator at run
/// time — R3b). A poison operand propagates.
fn lower_rational_of(db: &mut Db, num: StructId, den: StructId) -> Core {
    match (core_of(db, num), core_of(db, den)) {
        (Core::Poison(r), _) | (_, Core::Poison(r)) => Core::Poison(r),
        (Core::ConstInt(n), Core::ConstInt(d)) => normalized_rational(n, d),
        _ => Core::RationalOfInts { num, den },
    }
}

/// Lower a RUNTIME `Rational.truncate r` as a DERIVATION over existing prims — no new runtime op. Synthesize
/// `(let ((__rtrunc r)) (Int64.of (/ ((. Rational numerator) __rtrunc) ((. Rational denominator) __rtrunc))))`
/// and lower THAT: `numerator`/`denominator` read the pair as BigInts, BigInt `/` truncates toward zero
/// (dividend-signed, matching the const `IntValue::divmod` fold), and `Int64.of` checked-narrows the small
/// quotient (TRAPS on overflow). `r` is referenced twice, so it is LET-BOUND once (`__rtrunc`) — computed a
/// single time, read by two occurrences (the `__` prefix cannot collide with a source name). The operand
/// node `r` is spliced verbatim into the binding (it is already resolved/typed in this scope; the original
/// `Rational.truncate` apply node is fully replaced by this subtree's core, so the splice reparents cleanly
/// exactly as `partial_ctor_eta_closure` splices its supplied args). `resolve_subtree` classifies the synth
/// nodes; `core_of` on the root produces the derivation's Core (the same shape a hand-written source
/// expression lowers to — verified to compile + run: `7/2 → 3`, `-7/2 → -3`).
fn lower_rational_truncate(db: &mut Db, r: StructId) -> Core {
    let bind_name = "__rtrunc";
    // `(let ((__rtrunc r)) body)`.
    let binder_occ = db.push_name(bind_name);
    let binding = db.push_list(vec![binder_occ, r]);
    let bindings = db.push_list(vec![binding]);
    // `((. Rational numerator) __rtrunc)` and `((. Rational denominator) __rtrunc)` — a member access
    // applied to a fresh reference of the let binder (the `.` head form, per `(. List len)` precedent).
    let num_call = rational_member_call(db, "numerator", bind_name);
    let den_call = rational_member_call(db, "denominator", bind_name);
    // `(/ num_call den_call)` — BigInt truncating division (both operands are `Ty::BigInt`).
    let div_head = db.push_name("/");
    let quotient = db.push_list(vec![div_head, num_call, den_call]);
    // `(Int64.of quotient)` — the checked BigInt→Int64 narrowing (traps on overflow).
    let dot = db.push_name(".");
    let int64_mod = db.push_name("Int64");
    let of_key = db.push_name("of");
    let int64_of = db.push_list(vec![dot, int64_mod, of_key]);
    let narrowed = db.push_list(vec![int64_of, quotient]);
    let let_head = db.push_name("let");
    let derivation = db.push_list(vec![let_head, bindings, narrowed]);
    crate::resolve::resolve_subtree(db, derivation);
    core_of(db, derivation)
}

/// Build `((. Rational <member>) <bind_name>)` — a `Rational` module member access (`numerator`/
/// `denominator`) applied to a fresh reference of the let-bound name. Helper for `lower_rational_truncate`
/// (and the later floor/ceil/round derivations) so each reads the rational's component through the same
/// member surface the source would use. A FRESH name occurrence per call (a node has one parent).
fn rational_member_call(db: &mut Db, member: &str, bind_name: &str) -> StructId {
    let dot = db.push_name(".");
    let rational_mod = db.push_name("Rational");
    let member_key = db.push_name(member);
    let member_access = db.push_list(vec![dot, rational_mod, member_key]);
    let arg_ref = db.push_name(bind_name);
    db.push_list(vec![member_access, arg_ref])
}

/// Build `(BigInt.of <n>)` — a BigInt-typed integer literal for a synthesized derivation (the `0`/`1` a
/// floor/ceil/round comparison + `±1` adjustment compares/combines with the BigInt numerator/remainder).
/// A fresh node each call.
fn bigint_lit(db: &mut Db, n: i64) -> StructId {
    let lit = db.push_atom(crate::ast::Leaf::Int {
        value: crate::ast::IntValue::from_i64(n),
        radix: crate::ast::Radix::Dec,
    });
    let dot = db.push_name(".");
    let bigint_mod = db.push_name("BigInt");
    let of_key = db.push_name("of");
    let bigint_of = db.push_list(vec![dot, bigint_mod, of_key]);
    db.push_list(vec![bigint_of, lit])
}

/// Lower a RUNTIME `Rational.floor`/`ceil r` as a DERIVATION — `truncate` adjusted by ±1 off the remainder
/// sign. Synthesize (for FLOOR — ceil flips the comparison to `>` and the adjustment to `+`):
/// ```text
/// (let ((__rfc r))
///   (Int64.of
///     (let ((__q (/ (num __rfc) (den __rfc))))
///       (if (and (< (num __rfc) 0N) (not (= (% (num __rfc) (den __rfc)) 0N)))
///           (- __q 1N)
///           __q))))
/// ```
/// The toward-zero quotient `__q` is one too HIGH for a floor exactly when the value is NEGATIVE with a
/// nonzero remainder (`n < 0 ∧ rem ≠ 0`), so subtract 1 there; ceil is the mirror (positive + nonzero rem →
/// add 1). All BigInt ops (`/`, `%`, `<`/`>`, `=`) + the checked `Int64.of` narrowing already exist → no new
/// runtime op (hash-neutral). `__q` is let-bound (read twice); `num`/`den`/`rem` reads are fresh
/// `rational_member_call`s (each a distinct occurrence). Matches the const-fold arms above and the verified
/// formula (`floor(-7/2) = -4`, `ceil(7/2) = 4`).
fn lower_rational_floor_ceil(db: &mut Db, r: StructId, is_floor: bool) -> Core {
    let outer = "__rfc";
    let q_name = "__q";
    // `(/ (num __rfc) (den __rfc))` — the toward-zero truncating quotient, let-bound to `__q`.
    let q_div = {
        let num = rational_member_call(db, "numerator", outer);
        let den = rational_member_call(db, "denominator", outer);
        let div_head = db.push_name("/");
        db.push_list(vec![div_head, num, den])
    };
    let q_binder = db.push_name(q_name);
    let q_binding = db.push_list(vec![q_binder, q_div]);
    let q_bindings = db.push_list(vec![q_binding]);
    // `(<cmp> (num __rfc) 0N)` — floor: `n < 0`; ceil: `n > 0`.
    let sign_test = {
        let num = rational_member_call(db, "numerator", outer);
        let zero = bigint_lit(db, 0);
        let cmp = db.push_name(if is_floor { "<" } else { ">" });
        db.push_list(vec![cmp, num, zero])
    };
    // `(not (= (% (num __rfc) (den __rfc)) 0N))` — a nonzero remainder (the fraction is not whole).
    let rem_nonzero = {
        let num = rational_member_call(db, "numerator", outer);
        let den = rational_member_call(db, "denominator", outer);
        let rem_head = db.push_name("%");
        let rem = db.push_list(vec![rem_head, num, den]);
        let zero = bigint_lit(db, 0);
        let eq_head = db.push_name("=");
        let is_zero = db.push_list(vec![eq_head, rem, zero]);
        let not_head = db.push_name("not");
        db.push_list(vec![not_head, is_zero])
    };
    let and_head = db.push_name("and");
    let cond = db.push_list(vec![and_head, sign_test, rem_nonzero]);
    // `(<±> __q 1N)` — floor subtracts 1, ceil adds 1 — on the adjustment branch; else plain `__q`.
    let adjusted = {
        let q_ref = db.push_name(q_name);
        let one = bigint_lit(db, 1);
        let op = db.push_name(if is_floor { "-" } else { "+" });
        db.push_list(vec![op, q_ref, one])
    };
    let q_else = db.push_name(q_name);
    let if_head = db.push_name("if");
    let if_expr = db.push_list(vec![if_head, cond, adjusted, q_else]);
    let inner_let_head = db.push_name("let");
    let inner_let = db.push_list(vec![inner_let_head, q_bindings, if_expr]);
    // `(Int64.of <inner_let>)` — checked narrowing (traps on overflow).
    let dot = db.push_name(".");
    let int64_mod = db.push_name("Int64");
    let of_key = db.push_name("of");
    let int64_of = db.push_list(vec![dot, int64_mod, of_key]);
    let narrowed = db.push_list(vec![int64_of, inner_let]);
    // `(let ((__rfc r)) <narrowed>)`.
    let outer_binder = db.push_name(outer);
    let outer_binding = db.push_list(vec![outer_binder, r]);
    let outer_bindings = db.push_list(vec![outer_binding]);
    let outer_let_head = db.push_name("let");
    let derivation = db.push_list(vec![outer_let_head, outer_bindings, narrowed]);
    crate::resolve::resolve_subtree(db, derivation);
    core_of(db, derivation)
}

/// Lower a RUNTIME `Rational.round r` as a DERIVATION — NEAREST integer, ties HALF-AWAY-FROM-ZERO. Synthesize:
/// ```text
/// (let ((__rr r))
///   (Int64.of
///     (let ((__num (numerator __rr)) (__den (denominator __rr)))
///       (let ((__q (/ __num __den)) (__rem (% __num __den)))
///         (let ((__abs (if (< __rem 0N) (- 0N __rem) __rem)))
///           (if (>= (* 2N __abs) __den)
///               (if (< __rem 0N) (- __q 1N) (+ __q 1N))
///               __q))))))
/// ```
/// The toward-zero quotient `__q` is adjusted AWAY from zero (by the sign of the remainder, which equals the
/// value's sign) when twice the |remainder| is ≥ the denominator — i.e. the fractional part is ≥ ½. Using
/// `≥` (not `>`) makes an exact-half tie round AWAY (`2·|rem| = __den` at ½), the settled half-away ruling.
/// Multi-use values (`__num`/`__den`/`__q`/`__rem`/`__abs`) are LET-BOUND (each read 2–3×). All BigInt ops
/// (`/`, `%`, `*`, `-`, `<`, `>=`) + the checked `Int64.of` narrowing already exist → hash-neutral. Matches
/// the const-fold arm + the verified formula (`1/2 → 1`, `-1/2 → -1`, `3/2 → 2`, `5/2 → 3`, `7/3 → 2`).
fn lower_rational_round(db: &mut Db, r: StructId) -> Core {
    let outer = "__rr";
    // `(let ((__num (numerator __rr)) (__den (denominator __rr))) …)`.
    let num_call = rational_member_call(db, "numerator", outer);
    let den_call = rational_member_call(db, "denominator", outer);
    let num_binding = {
        let b = db.push_name("__num");
        db.push_list(vec![b, num_call])
    };
    let den_binding = {
        let b = db.push_name("__den");
        db.push_list(vec![b, den_call])
    };
    let numden_bindings = db.push_list(vec![num_binding, den_binding]);
    // `(let ((__q (/ __num __den)) (__rem (% __num __den))) …)`.
    let q_binding = {
        let div_head = db.push_name("/");
        let n = db.push_name("__num");
        let d = db.push_name("__den");
        let div = db.push_list(vec![div_head, n, d]);
        let b = db.push_name("__q");
        db.push_list(vec![b, div])
    };
    let rem_binding = {
        let rem_head = db.push_name("%");
        let n = db.push_name("__num");
        let d = db.push_name("__den");
        let rem = db.push_list(vec![rem_head, n, d]);
        let b = db.push_name("__rem");
        db.push_list(vec![b, rem])
    };
    let qrem_bindings = db.push_list(vec![q_binding, rem_binding]);
    // `(let ((__abs (if (< __rem 0N) (- 0N __rem) __rem))) …)` — the remainder's magnitude.
    let abs_binding = {
        let rem_ref = db.push_name("__rem");
        let zero = bigint_lit(db, 0);
        let lt = db.push_name("<");
        let neg_test = db.push_list(vec![lt, rem_ref, zero]);
        let zero2 = bigint_lit(db, 0);
        let rem_ref2 = db.push_name("__rem");
        let sub = db.push_name("-");
        let negated = db.push_list(vec![sub, zero2, rem_ref2]);
        let rem_ref3 = db.push_name("__rem");
        let if_head = db.push_name("if");
        let abs = db.push_list(vec![if_head, neg_test, negated, rem_ref3]);
        let b = db.push_name("__abs");
        db.push_list(vec![b, abs])
    };
    let abs_bindings = db.push_list(vec![abs_binding]);
    // The tie test `(>= (* 2N __abs) __den)`.
    let tie_test = {
        let two = bigint_lit(db, 2);
        let abs_ref = db.push_name("__abs");
        let mul = db.push_name("*");
        let doubled = db.push_list(vec![mul, two, abs_ref]);
        let den_ref = db.push_name("__den");
        let ge = db.push_name(">=");
        db.push_list(vec![ge, doubled, den_ref])
    };
    // The away-from-zero adjustment `(if (< __rem 0N) (- __q 1N) (+ __q 1N))`.
    let adjusted = {
        let rem_ref = db.push_name("__rem");
        let zero = bigint_lit(db, 0);
        let lt = db.push_name("<");
        let neg_test = db.push_list(vec![lt, rem_ref, zero]);
        let q1 = db.push_name("__q");
        let one1 = bigint_lit(db, 1);
        let sub = db.push_name("-");
        let minus = db.push_list(vec![sub, q1, one1]);
        let q2 = db.push_name("__q");
        let one2 = bigint_lit(db, 1);
        let add = db.push_name("+");
        let plus = db.push_list(vec![add, q2, one2]);
        let if_head = db.push_name("if");
        db.push_list(vec![if_head, neg_test, minus, plus])
    };
    let q_else = db.push_name("__q");
    let if_head = db.push_name("if");
    let tie_if = db.push_list(vec![if_head, tie_test, adjusted, q_else]);
    // Nest the lets: abs over (q,rem) over (num,den).
    let let_head3 = db.push_name("let");
    let abs_let = db.push_list(vec![let_head3, abs_bindings, tie_if]);
    let let_head2 = db.push_name("let");
    let qrem_let = db.push_list(vec![let_head2, qrem_bindings, abs_let]);
    let let_head1 = db.push_name("let");
    let numden_let = db.push_list(vec![let_head1, numden_bindings, qrem_let]);
    // `(Int64.of <numden_let>)` — checked narrowing (traps on overflow).
    let dot = db.push_name(".");
    let int64_mod = db.push_name("Int64");
    let of_key = db.push_name("of");
    let int64_of = db.push_list(vec![dot, int64_mod, of_key]);
    let narrowed = db.push_list(vec![int64_of, numden_let]);
    // `(let ((__rr r)) <narrowed>)`.
    let outer_binder = db.push_name(outer);
    let outer_binding = db.push_list(vec![outer_binder, r]);
    let outer_bindings = db.push_list(vec![outer_binding]);
    let outer_let_head = db.push_name("let");
    let derivation = db.push_list(vec![outer_let_head, outer_bindings, narrowed]);
    crate::resolve::resolve_subtree(db, derivation);
    core_of(db, derivation)
}

/// The two `IntValue`s of a constant rational OPERAND (already normalized), or `None` if `id` did not fold
/// to a compile-time constant (a runtime rational — the caller then emits the runtime `rational-*` op, R3b).
///
/// A `Core::ConstInt(n)` counts as the rational `n/1`: this function only classifies operands of a RATIONAL
/// op (`lower_rational_arith`/`lower_rational_cmp`), so an integer-valued operand there IS a constant
/// rational whose denominator is 1 — the whole number n as an exact rational. This matters because a
/// projection/const-eval fold of an INTEGER-VALUED Rational field folds to a bare `Core::ConstInt` (its
/// numerator), not a `Core::ConstRational` (e.g. #3543's nullary-record field fold: `(. (rect) w)` with a
/// Rational field `w = 4` folds to `ConstInt(4)`). Without this arm the constant pair `(* 4 3)` failed to
/// recognize its operands as rationals, fell through to the RUNTIME `RationalBinOp`, and — since a bare
/// `ConstInt` has no heap ownership class at the borrowing-op emit — DECLINED ("borrowing op operand has an
/// ownership this backend cannot yet prove"; the notebook Rational-field regression breaker bisected to
/// #3543). Folding to `n/1` here reduces the whole op to a `Core::ConstRational`, so no runtime op (and no
/// borrow classification) is reached at all — the fully-general const-fold outcome.
fn const_rational_of(
    db: &mut Db,
    id: StructId,
) -> Option<(crate::ast::IntValue, crate::ast::IntValue)> {
    match core_of(db, id) {
        Core::ConstRational(n, d) => Some((n, d)),
        // An integer-valued rational operand: `n` as the exact rational `n/1` (denominator 1).
        Core::ConstInt(n) => Some((n, crate::ast::IntValue::from_i64(1))),
        _ => None,
    }
}

/// Lower a rational `+`/`-`/`*`/`/` — fold a CONSTANT pair to a normalized `Core::ConstRational` via
/// exact `IntValue` bignum arithmetic, or (R3b) emit the runtime `Core::RationalBinOp` when an operand is
/// RUNTIME-valued (the runtime `rational-*` op computes + normalizes on the limb library). A poison
/// propagates. The constant formulas keep the result normalized (`normalized_rational` re-reduces): `a/b +
/// c/d = (ad+cb)/(bd)`, `a/b - c/d = (ad-cb)/(bd)`, `a/b * c/d = (ac)/(bd)`, `a/b ÷ c/d = (ad)/(bc)`
/// (division by `0/1` → a zero denominator → trap, exactly `Rational.of`'s zero-denom trap).
///
/// `Rational` is a declared-EXACT numeric type, and this arithmetic loses NO precision on EITHER path: the
/// constant fold works over `IntValue` bignum numerators/denominators and the runtime op over the runtime
/// BigInt limb library — no fixed width to overflow, no rounding — so an exact rational operation's result
/// is the exact number whether it folds or runs.
//= spec/capabilities/numeric-model.md#exact-arithmetic-is-exact
//# An operation on values of a numeric type declared exact MUST NOT lose precision.
fn lower_rational_arith(db: &mut Db, op: Prim, lhs: StructId, rhs: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, lhs) {
        return Core::Poison(r);
    }
    if let Core::Poison(r) = core_of(db, rhs) {
        return Core::Poison(r);
    }
    let (Some((a, b)), Some((c, d))) = (const_rational_of(db, lhs), const_rational_of(db, rhs))
    else {
        // A RUNTIME operand — emit the runtime `rational-*` op (R3b). The op computes + normalizes on the
        // runtime limb library; a poison operand already returned above.
        let rat_op = match op {
            Prim::Add => crate::core::RationalOp::Add,
            Prim::Sub => crate::core::RationalOp::Sub,
            Prim::Mul => crate::core::RationalOp::Mul,
            Prim::Div => crate::core::RationalOp::Div,
            _ => return Core::Poison(Reject::decline("not a Rational arithmetic op")),
        };
        return Core::RationalBinOp {
            op: rat_op,
            lhs,
            rhs,
        };
    };
    let (num, den) = match op {
        Prim::Add => (a.mul(&d).add(&c.mul(&b)), b.mul(&d)),
        Prim::Sub => (a.mul(&d).sub(&c.mul(&b)), b.mul(&d)),
        Prim::Mul => (a.mul(&c), b.mul(&d)),
        Prim::Div => (a.mul(&d), b.mul(&c)),
        _ => return Core::Poison(Reject::decline("not a Rational arithmetic op")),
    };
    normalized_rational(num, den)
}

/// Lower a rational comparison `<`/`>`/`<=`/`>=`/`=` — fold a CONSTANT pair to a `Core::ConstBool` by
/// comparing the two normalized rationals EXACTLY: `a/b <=> c/d` ⇔ `a*d <=> c*b` (both denominators are
/// strictly positive after normalization, so cross-multiplication preserves the order direction), or
/// (R3b) emit `Core::RationalCmp` (the runtime `rational-cmp` op + a fixed compare-with-zero) when an
/// operand is RUNTIME-valued. A poison propagates.
fn lower_rational_cmp(db: &mut Db, op: Prim, lhs: StructId, rhs: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, lhs) {
        return Core::Poison(r);
    }
    if let Core::Poison(r) = core_of(db, rhs) {
        return Core::Poison(r);
    }
    let (Some((a, b)), Some((c, d))) = (const_rational_of(db, lhs), const_rational_of(db, rhs))
    else {
        // A RUNTIME operand — emit `Core::RationalCmp` (`rational-cmp` + a fixed compare-with-zero, R3b).
        return Core::RationalCmp { op, lhs, rhs };
    };
    let ord = a.mul(&d).cmp(&c.mul(&b));
    Core::ConstBool(compare_ord(op, ord))
}

/// True iff either operand of a binary op has solved type `Ty::Rational` — the signal to route `+`/`-`/
/// `*`/`/`/comparison to the exact rational fold. (A `Rational`/other mix never reaches lowering —
/// `check_application` rejected it CDZ0301 — so if ONE operand is a Rational the other is too.)
fn rational_operand(db: &mut Db, args: &[StructId]) -> bool {
    // A bare `Rational` OR a quantity over a Rational magnitude — a `(Qty Rational u)` erases to its
    // inner Rational core, so a comparison of two such quantities folds through the same rational path
    // (they are same-dimension same-reference-unit under the model, a same-unit compare of the erased
    // rationals). Peel `Ty::Qty` to its inner; without this a `(< (Qty Rational) (Qty Rational))` fell to
    // the scalar compare and DECLINED ("compound needs a heap walk").
    args.iter().any(|&a| {
        matches!(
            peel_qty_inner_ty(crate::infer::type_of(db, a)),
            crate::ty::Ty::Rational
        )
    })
}

/// The inner numeric type of a `(Qty T u)`, or the type itself when not a quantity — so a quantity's
/// magnitude arithmetic/comparison routes by its ERASED inner numeric (a quantity erases to its inner
/// value's core). Shared by `rational_operand`/`bigint_operand` so a `(Qty Rational/BigInt u)` takes the
/// exact rational/bigint path rather than the fixnum scalar path.
fn peel_qty_inner_ty(ty: crate::ty::Ty) -> crate::ty::Ty {
    match ty {
        crate::ty::Ty::Qty { inner, .. } => *inner,
        other => other,
    }
}

/// True iff either operand of a binary op has solved type `Ty::BigInt` — the signal to route `+`/`-`/
/// `*`/`/` to the runtime BigInt arithmetic instead of the fixed-width int fold. (A `BigInt`/fixed mix
/// never reaches lowering — `check_application` rejected it CDZ0301 — so if ONE operand is a BigInt the
/// other is too.)
fn bigint_operand(db: &mut Db, args: &[StructId]) -> bool {
    // A bare `BigInt` OR a quantity over a BigInt magnitude (`(Qty BigInt u)` erases to its inner BigInt
    // handle) — peel `Ty::Qty` so a `(< (Qty BigInt) (Qty BigInt))` routes to the bigint comparison
    // (`bigint-cmp`) rather than declining as a compound scalar compare. The arithmetic `+`/`-`/`*`/`/`
    // over a BigInt-inner quantity is dispatched separately (`quantity_inner_is_bigint`); this covers the
    // COMPARISON path, which reads `bigint_operand` in `lower_comparison`.
    args.iter().any(|&a| {
        matches!(
            peel_qty_inner_ty(crate::infer::type_of(db, a)),
            crate::ty::Ty::BigInt
        )
    })
}

/// True iff either operand of a binary op has solved type `Ty::Float` — the signal to remap `+`/`-`/`*`/
/// `/` to the float prim (`FAdd`…) and route to `lower_float_arith` instead of the integer fold. There is
/// no distinct `+.`; floating-point arithmetic is dispatched here on the operand type, like the BigInt/
/// Rational operands. (A `Float`/int mix never reaches lowering — `check_application` rejected it CDZ0301
/// — so if ONE operand is a Float the other is too.)
fn float_operand(db: &mut Db, args: &[StructId]) -> bool {
    // Peel `Ty::Qty` before reading the inner type — a `(Qty Float64 u)` erases to a bare f64, so a
    // comparison of two same-unit Float-inner quantities is a plain scalar float compare and must route to
    // the float path (`lower_comparison`'s FloatCompare), NOT decline as "a compound value needs a heap
    // walk". Without the peel a `(< (Qty x meter) (Qty 5.0 meter))` saw `Ty::Qty` (not `Ty::Float`), missed
    // this dispatch, and fell through to the compound-comparison decline — a gap masked until runtime float
    // ordering landed (before that even the bare float compare declined). Mirrors `bigint_operand`/
    // `rational_operand`, which already peel `Ty::Qty` for the same reason. (A mixed-scale quantity
    // comparison is handled earlier by `lower_quantity_combine`, which converts to the reference; this
    // covers the SAME-scale case that falls through to the generic comparison dispatch.)
    args.iter().any(|&a| {
        matches!(
            peel_qty_inner_ty(crate::infer::type_of(db, a)),
            crate::ty::Ty::Float(_)
        )
    })
}

/// Lower a BigInt `+`/`-`/`*`/`/` to a runtime `Core::BigIntBinOp` (the runtime `bigint-*` op). Unlike
/// fixed-width arithmetic, this does NOT constant-fold: exact BigInt arithmetic needs compiler-side
/// bignum (rcdzc deliberately has no bignum crate — `IntValue` carries the value but not arithmetic), so
/// the unbounded arithmetic runs at RUN TIME via the runtime `Big` limb library (B3a). A poison operand
/// propagates. `div` traps on a zero divisor at run time (numeric-model — an unbounded range gives `n/0`
/// no value); the never-trapping add/sub/mul grow the magnitude as needed.
///
/// `BigInt` represents every integer with no bound, so this arithmetic never overflows, and the runtime
/// `bigint-*` limb ops grow the representation as the result requires rather than wrapping or trapping on
/// magnitude (only `n/0` — no value — traps).
//= spec/capabilities/numeric-model.md#an-arbitrary-precision-integer-has-unbounded-range
//# An arbitrary-precision integer type MUST represent every integer with no maximum or minimum bound, so that an arithmetic operation on it never overflows.
//= spec/capabilities/numeric-model.md#an-arbitrary-precision-integer-has-unbounded-range
//# An arithmetic operation on arbitrary-precision integers MUST NOT trap for the magnitude of its result, growing its representation as the result requires rather than wrapping or trapping.
fn lower_bigint_arith(db: &mut Db, op: Prim, lhs: StructId, rhs: StructId) -> Core {
    let lc = core_of(db, lhs);
    let rc = core_of(db, rhs);
    if let Core::Poison(r) = lc {
        return Core::Poison(r);
    }
    if let Core::Poison(r) = rc {
        return Core::Poison(r);
    }
    // A CONSTANT BigInt pair could fold exactly here (the `IntValue` bignum is available), BUT it
    // DELIBERATELY does NOT: the repeated-squaring idiom `a_i = (* a_{i-1} a_{i-1})` DOUBLES the bit-width
    // each level, so folding a depth-k chain computes a 2^k-bit number at COMPILE TIME — a
    // compile-time-blowup / hang on a small program (the `a_repeated_squaring_bigint_chain_diagnoses_in_
    // bounded_time` regression). Exact unbounded arithmetic is a RUNTIME op (the runtime grows the
    // magnitude lazily, and only if the value is actually demanded); the compiler stays bounded. The ONE
    // exception is a program that EXPORTS a single constant BigInt result — but that path is served by the
    // boundary value-form on a `Core::ConstInt` (a plain widened literal), not by folding an arithmetic
    // chain. So a runtime `bigint-*` op is emitted for EVERY BigInt arithmetic, constant operands or not.
    let big_op = match op {
        Prim::Add => crate::core::BigIntOp::Add,
        Prim::Sub => crate::core::BigIntOp::Sub,
        Prim::Mul => crate::core::BigIntOp::Mul,
        Prim::Div => crate::core::BigIntOp::Div,
        Prim::Rem => crate::core::BigIntOp::Rem,
        _ => return Core::Poison(Reject::decline("not a BigInt arithmetic op")),
    };
    Core::BigIntBinOp {
        op: big_op,
        lhs,
        rhs,
    }
}

/// Lower a BigInt comparison `<`/`>`/`<=`/`>=`/`=` to either a constant `Bool` fold or a runtime
/// `Core::BigIntCmp` (the runtime `bigint-cmp` op + a fixed compare-with-zero). A CONSTANT pair (both
/// operands `Core::ConstInt` — the shape a folded `(BigInt.of <constant>)` leaves) folds when both values
/// fit `i128` (`to_i128` reads the exact value; every constant a program is likely to compare fits, and
/// the runtime op covers the rest), comparing at 128-bit precision. A poison operand propagates. Otherwise
/// (a runtime operand) emit `Core::BigIntCmp`; the emit borrows both operands and applies the operator's
/// signed compare against the three-way `-1`/`0`/`1` result.
fn lower_bigint_cmp(db: &mut Db, op: Prim, lhs: StructId, rhs: StructId) -> Core {
    let lc = core_of(db, lhs);
    let rc = core_of(db, rhs);
    match (lc, rc) {
        (Core::Poison(r), _) | (_, Core::Poison(r)) => Core::Poison(r),
        // A constant BigInt pair — both carry the exact `IntValue`. Fold at 128-bit precision when both
        // fit; a value beyond i128 (astronomically large) falls through to the runtime op.
        (Core::ConstInt(a), Core::ConstInt(b)) => match (a.to_i128(), b.to_i128()) {
            (Some(x), Some(y)) => {
                let r = compare_ord(op, x.cmp(&y));
                trace!(target: "rcdzc::fold", op = intrinsic_name(op), result = r, "folded constant BigInt comparison (i128)");
                Core::ConstBool(r)
            }
            _ => Core::BigIntCmp { op, lhs, rhs },
        },
        _ => Core::BigIntCmp { op, lhs, rhs },
    }
}

/// Lower UNARY NEGATION `(- e)` (the ML prefix `-<expr>`, canonicalized to the arity-1 subtraction) as
/// `0 - e` at the operand's numeric type. Rather than a new negate op, synthesize a typed ZERO operand
/// and route to the SAME binary-subtraction machinery each numeric type already uses — so an `Int N`
/// gets the checked `x == MIN` overflow trap (the identical path the `x * -1 → (- 0 x)` strength
/// reduction takes), a `Float`/`Rational`/`BigInt` its own arithmetic, and a `Qty` negates its erased
/// magnitude while `type_of(id)` (the negation node, typed `(Qty …)` by `infer`) preserves the unit.
///
/// A FLOAT is negated by `-1.0 * e` (via `lower_float_arith`'s multiply), NOT `0.0 - e`: IEEE
/// `0.0 - (+0.0)` is `+0.0`, but negation must flip the sign of a zero (`-(+0.0) = -0.0`,
/// core-semantics.md §Floating-Point Equality Follows The Canonical Byte Form distinguishes them), and
/// `-1.0 * x` flips the sign bit for zero/inf/finite alike. Integer/Rational/BigInt `0 - e` is exact,
/// so those keep the subtraction form (and the int `MIN` trap).
fn lower_negate(db: &mut Db, id: StructId, operand: StructId) -> Core {
    use crate::ty::Ty;
    // Propagate a poison operand (its own fault is the report).
    if let Core::Poison(r) = core_of(db, operand) {
        return Core::Poison(r);
    }
    let t = crate::infer::type_of(db, id);
    // The inner numeric type — for a `Qty` it is the erased magnitude's type; otherwise the type itself.
    let inner = match &t {
        Ty::Qty { inner, .. } => (**inner).clone(),
        other => other.clone(),
    };
    // For a `Qty` operand, negate the ERASED magnitude occurrence (`Qty.of`'s value); a bare numeric
    // operand negates directly. A runtime non-`Qty.of` quantity magnitude has no erased occurrence to
    // read — decline (the same gap `lower_qty_pow`/`lower_quantity_combine` decline on).
    let value = if matches!(t, Ty::Qty { .. }) {
        match crate::eval::qty_value_occ(db, operand) {
            Some(v) => v,
            None => {
                return Core::Poison(Reject::decline(
                    "negation of a runtime non-Qty.of quantity magnitude is not supported",
                ));
            }
        }
    } else {
        operand
    };
    match &inner {
        // FLOAT — `-1.0 * e` (sign-correct for ±0.0/inf), folded/emitted by `lower_float_arith`.
        Ty::Float(_) => {
            let neg_one = synth_core(
                db,
                Core::ConstFloat(match crate::ast::Decimal::from_f64(-1.0) {
                    Some(d) => d,
                    None => return Core::Poison(Reject::decline("-1.0 has no decimal form")),
                }),
                inner.clone(),
            );
            lower_float_arith(db, id, Prim::FMul, &[neg_one, value])
        }
        // BIGINT — `0 - e` via the runtime `bigint-sub` (never folds; grows as needed).
        Ty::BigInt => {
            let zero = synth_core(db, Core::ConstInt(IntValue::zero()), Ty::BigInt);
            lower_bigint_arith(db, Prim::Sub, zero, value)
        }
        // RATIONAL — `0 - e`, folded exactly (constant) or the runtime `rational-sub`.
        Ty::Rational => {
            let zero = synth_core(
                db,
                Core::ConstRational(IntValue::zero(), IntValue::from_i64(1)),
                Ty::Rational,
            );
            lower_rational_arith(db, Prim::Sub, zero, value)
        }
        // FIXED-WIDTH INTEGER — `0 - e` via `lower_arith`, which folds a constant (a `0 - MIN` overflow →
        // CDZ0304) and otherwise emits the checked runtime subtract (its `x == MIN` guard is negation's).
        Ty::Int(_) => {
            let zero = synth_core(db, Core::ConstInt(IntValue::zero()), inner.clone());
            lower_arith(db, id, Prim::Sub, &[zero, value])
        }
        // Not a numeric type — `infer` already rejected this (CDZ0201 "negation is not defined on …"); a
        // residual `Any` (an operand that faulted elsewhere) declines rather than fabricating a value.
        _ => Core::Poison(Reject::decline(
            "negation of a non-numeric operand (a fault is reported at inference)",
        )),
    }
}

fn lower_arith(db: &mut Db, id: StructId, op: Prim, args: &[StructId]) -> Core {
    if args.len() != 2 {
        return Core::Poison(binop_arity_reject(op, args));
    }
    let lhs = core_of(db, args[0]);
    let rhs = core_of(db, args[1]);
    match (lhs, rhs) {
        (Core::ConstInt(a), Core::ConstInt(b)) => {
            // OVERFLOW POLICY (STAGE 2b, numeric-model §Overflow Behavior Is Configurable By Policy,
            // #5313/#5337): under a `(pragma overflow (signed|unsigned wrap))` module a CONSTANT `+`/`-`/`*`
            // that overflows must WRAP (two's-complement, mod 2^width) — NOT reject CDZ0304. `overflow_mode_of`
            // resolves this node's authoritative mode (module pragma by operand signedness > global manifest >
            // Trap; post-mono, #5686) — the SAME decision the backend codegen (2b runtime half) + the oracle
            // read, so const-fold cannot drift from runtime. A Wrap node folds to the EXACT result truncated
            // to the operand's solved width via `IntValue::wrap_to` (identical to what the runtime wrapping op
            // yields). Only `+`/`-`/`*` carry a policy (the spec's configurable ops); `/`/`%`/shift/bitwise
            // keep their existing folds. A non-overflowing op wraps to itself (no-op), so this changes nothing
            // except at overflow. `Trap` mode does NOT match here → falls through to the existing
            // overflow→CDZ0304 logic below, unchanged.
            if matches!(op, Prim::Add | Prim::Sub | Prim::Mul)
                && crate::infer::overflow_mode_of(db, id) == crate::db::OverflowMode::Wrap
                && let crate::ty::Ty::Int(it) = peel_qty_inner_ty(crate::infer::type_of(db, id))
            {
                let exact = match op {
                    Prim::Add => a.add(&b),
                    Prim::Sub => a.sub(&b),
                    _ => a.mul(&b), // Mul (the `matches!` guard admits only Add/Sub/Mul)
                };
                let wrapped = exact.wrap_to(it.ground_signed(), it.ground_width());
                trace!(target: "rcdzc::fold", node = id.0, op = intrinsic_name(op), "constant +/-/* WRAPS mod 2^width (pragma overflow wrap) instead of trapping");
                return Core::ConstInt(wrapped);
            }
            // ALGEBRAIC IDENTITY FIRST — before the i64 fold. `fold_arith` evaluates over `i64`, so an
            // operand at/above `2^63` (a legitimate `UInt64` constant, e.g. `UInt64.max = 2^64-1`) has no
            // `i64` and `fold_arith` rejects it CDZ0304 ("constant operand does not fit the integer width")
            // — a SPURIOUS reject of valid unsigned arithmetic (`(+ (: 18446744073709551615 UInt64) 0)`
            // declined instead of folding to the operand). The width-agnostic identities (`x+0`/`0+x`/`x-0`/
            // `x*1`/`1*x`/`x*0`) return an OPERAND unchanged (or a trap-free `0`), so they are correct at
            // ANY width WITHOUT an i64 evaluation — and the both-constant case never reached them before (it
            // dispatched straight to `fold_arith`; the `arith_identity` call below is only in the
            // not-both-constant fallthrough). Trying them here first folds the big-UInt64 identity cleanly.
            // Only fires when at least one operand is out of i64 range (the case `fold_arith` mishandles); an
            // in-range both-constant op keeps its exact `fold_arith` result (identical, so no behavior change
            // for the common case). A NON-identity big-UInt64 op (`(+ u64max 1)`) still falls to `fold_arith`
            // and its CDZ0304 — a genuine unsigned-overflow fold is a separable follow-up.
            if a.to_i64().is_none() || b.to_i64().is_none() {
                let lc = Core::ConstInt(a.clone());
                let rc = Core::ConstInt(b.clone());
                if let Some(simplified) = arith_identity(db, op, args[0], &lc, args[1], &rc) {
                    trace!(target: "rcdzc::fold", op = intrinsic_name(op), "big-unsigned constant identity folded (operand out of i64 range — bypassing the i64 fold)");
                    return simplified;
                }
                // GENERAL WIDE FOLD (an operand ≥ 2^63 has no `i64`, so `fold_arith`'s i64 path would
                // spuriously reject it CDZ0304 — but the SOLVED type is a wide UInt64 whose range the result
                // may well fit). Evaluate `Add`/`Sub`/`Mul`/`Div`/`Rem` over EXACT arbitrary-precision
                // `IntValue` (no i64 truncation), THEN range-check the result against the operands' solved
                // width — the SAME `fits_width` check the i64 path applies below. `(/ (: u64max-1 UInt64) 2)`
                // → the true quotient (fits UInt64 → folds); a genuine overflow (`(+ u64max 1)` → 2^64,
                // doesn't fit u64) → CDZ0304, exactly as the i64 path would for a narrow overflow. Trap
                // semantics: `divmod` returns `None` on a zero divisor (→ CDZ0304 divide-by-zero); UNSIGNED
                // has no `MIN/-1` trap (signed-only, and `IntValue::divmod` has no such special case), so an
                // unsigned `/`/`%` never spuriously traps. Shifts/bitwise stay on the i64 path below (a wide
                // shift-count/mask is out of scope here; they fall through to `fold_arith`). Only reached
                // when an operand exceeds i64 AND no identity fired, so the in-range fast path is untouched.
                let wide = match op {
                    Prim::Add => Some(a.add(&b)),
                    Prim::Sub => Some(a.sub(&b)),
                    Prim::Mul => Some(a.mul(&b)),
                    Prim::Div => a.divmod(&b).map(|(q, _)| q),
                    Prim::Rem => a.divmod(&b).map(|(_, r)| r),
                    _ => None, // shifts/bitwise/other — fall through to the i64 fold_arith path below
                };
                match (op, wide) {
                    // A defined result — range-check against the solved width (peeling `Ty::Qty` as the i64
                    // path does), fold if it fits, else the same OPERATION-overflow CDZ0304.
                    (Prim::Add | Prim::Sub | Prim::Mul | Prim::Div | Prim::Rem, Some(r)) => {
                        return match peel_qty_inner_ty(crate::infer::type_of(db, id)) {
                            crate::ty::Ty::Int(it)
                                if !r.fits_width(it.ground_signed(), it.ground_width()) =>
                            {
                                trace!(target: "rcdzc::fold", node = id.0, op = intrinsic_name(op), "wide constant arithmetic result overflows the solved width → CDZ0304");
                                Core::Poison(Reject::coded(
                                    Code::ConstTrap,
                                    "this constant arithmetic operation overflows its integer type (a \
                                     compile-provable overflow traps)",
                                ))
                            }
                            _ => {
                                trace!(target: "rcdzc::fold", op = intrinsic_name(op), "wide constant arithmetic folded exactly over IntValue (operand out of i64 range)");
                                Core::ConstInt(r)
                            }
                        };
                    }
                    // `Div`/`Rem` by a zero divisor — `divmod` gave `None`; the constant operation always
                    // traps (the same CDZ0304 the i64 path's `checked_div`/`checked_rem` → `None` produces).
                    (Prim::Div | Prim::Rem, None) => {
                        trace!(target: "rcdzc::fold", op = intrinsic_name(op), "wide constant divide/rem by zero → CDZ0304");
                        return Core::Poison(Reject::coded(
                            Code::ConstTrap,
                            format!(
                                "`{}` by the constant 0 always traps (divide by zero) — guard the divisor \
                                 or remove the division",
                                intrinsic_name(op)
                            ),
                        ));
                    }
                    // A shift/bitwise op (with a wide operand) is NOT handled in this wide-arith `match` —
                    // it falls through to the unified shift/bitwise fold just below (which covers BOTH the
                    // wide-operand and the small-operand-wide-result cases uniformly).
                    _ => {}
                }
            }
            // SHIFT / BITWISE over a fixed width, folded over the SOLVED width (NOT `fold_arith`'s i64 path).
            // Covers BOTH (a) a ≥2^63 UNSIGNED operand (`& (: u64max UInt64) …`, `>> big-u64 1`) — no i64, so
            // the i64 path would spuriously reject — AND (b) a SMALL-operand shift whose RESULT exceeds i64 but
            // fits the unsigned width (`(<< (: 1 UInt64) 63)` = 2^63; `checked_shl_i64` overflow-checked against
            // Int64 → spurious CDZ0304). `fold_shift_bitwise_at_width` range-checks against the SOLVED width and
            // handles UNSIGNED only (a SIGNED type returns `None` → the i64 path folds it, preserving arithmetic
            // sign-extending `>>`). Returns `None` for non-shift/bitwise ops → the i64 arith path handles
            // Add/Sub/Mul/Div/Rem. A shift/bitwise whose result fits i64 folds identically either way, so this
            // is behavior-preserving for the common narrow case and only fixes the previously-wrong unsigned
            // wide-result `<<`/`>>`/`&`/`|`/`^` outcomes.
            if let crate::ty::Ty::Int(it) = peel_qty_inner_ty(crate::infer::type_of(db, id))
                && let Some(folded) =
                    fold_shift_bitwise_at_width(op, &a, &b, it.ground_signed(), it.ground_width())
            {
                return folded;
            }
            // A CONSTANT `+`/`-`/`*`/`/`/`%` whose SOLVED type is WIDER than i64 (an unsigned 64-bit
            // type, whose max `2^64-1` is not i64-representable) where BOTH operands fit i64 but the exact
            // RESULT overflows i64 while still fitting the solved width — `(+ (: Int64.max UInt64) 2)` =
            // 2^63+1, a valid UInt64. `fold_arith`'s i64 `checked_add` would trap on the i64-overflowing
            // result → a SPURIOUS "overflows Int64" CDZ0304, even though the value is in range for UInt64.
            // (The ≥2^63-OPERAND wide-fold path above handles the case where an OPERAND lacks an i64; this
            // is its twin for a SMALL-OPERAND, WIDE-RESULT op — mirroring the shift/bitwise wide-result fix
            // just above.) Compute the result EXACTLY over `IntValue` (no i64 truncation) and range-check
            // against the solved width: fold if it fits, else the same OPERATION-overflow CDZ0304 the i64
            // path emits. Fires ONLY for a solved type that does NOT fit i64 (u64) — every narrower/signed
            // type keeps the i64 path unchanged (identical result when it fits, correct CDZ0304 when it
            // genuinely overflows the type), so this is behavior-preserving for the common case. A `/`/`%`
            // by zero (`divmod` → `None`) falls through to `fold_arith`'s divide-by-zero CDZ0304.
            if let crate::ty::Ty::Int(it) = peel_qty_inner_ty(crate::infer::type_of(db, id))
                && !it.fits_within(crate::ty::IntTy::i64())
            {
                let wide = match op {
                    Prim::Add => Some(a.add(&b)),
                    Prim::Sub => Some(a.sub(&b)),
                    Prim::Mul => Some(a.mul(&b)),
                    Prim::Div => a.divmod(&b).map(|(q, _)| q),
                    Prim::Rem => a.divmod(&b).map(|(_, r)| r),
                    _ => None, // shift/bitwise handled above; anything else falls to the i64 path
                };
                if let Some(r) = wide {
                    return if r.fits_width(it.ground_signed(), it.ground_width()) {
                        trace!(target: "rcdzc::fold", node = id.0, op = intrinsic_name(op), "wide-result constant arithmetic folded exactly over IntValue (result overflows i64 but fits the solved u64 width)");
                        Core::ConstInt(r)
                    } else {
                        trace!(target: "rcdzc::fold", node = id.0, op = intrinsic_name(op), "wide constant arithmetic result overflows the solved width → CDZ0304");
                        Core::Poison(Reject::coded(
                            Code::ConstTrap,
                            "this constant arithmetic operation overflows its integer type (a \
                             compile-provable overflow traps)",
                        ))
                    };
                }
            }
            // Fold over i64, THEN range-check the result against the op's SOLVED width. `fold_arith`
            // evaluates at the Stage i64 width, so a NARROW overflow whose true result still fits i64
            // (`255 + 1 = 256` over UInt8, `100 * 2 = 200` over Int8) folds to a valid `ConstInt` and
            // would otherwise slip through to a backend CDZ0302 ("a literal that doesn't fit"). But this
            // is a constant OPERATION whose defined outcome is a TRAP (the value overflows the type),
            // NOT an out-of-range literal — so it is CDZ0304 (`ConstTrap`), the SAME code the wide
            // `(+ Int64.max 1)` gets and the reject-don't-miscompile discipline the const-overflow /
            // List.update-OOB path already follows. (A direct out-of-range LITERAL `(: 256 UInt8)` is
            // still CDZ0302 at its own annotation — it is a literal, not an operation result.)
            match fold_arith(op, a, b) {
                // Peel `Ty::Qty` before reading the width: a same-unit quantity `+`/`-`/`*`/`/` over a
                // narrow-Int inner (`(Qty Int8 u)`) falls through to this generic arith path (its scales
                // are equal, so it is NOT a mixed-unit combine), and its solved type is `Ty::Qty { inner:
                // Int … }`, not a bare `Ty::Int`. Without the peel the width-check below never fired for a
                // quantity, so an inner-narrow overflow (`100 + 100` over `(Qty Int8 u)`) slipped through to
                // a backend CDZ0302 ("a literal that doesn't fit") — a wrong code (it is an OPERATION
                // overflow, not a literal), and one `cdz check` MISSED (the gate lives only in the backend).
                // Units are erased, so a quantity's arithmetic obeys the inner integer type's overflow rule.
                Core::ConstInt(r) => match peel_qty_inner_ty(crate::infer::type_of(db, id)) {
                    crate::ty::Ty::Int(it)
                        if !r.fits_width(it.ground_signed(), it.ground_width()) =>
                    {
                        trace!(target: "rcdzc::fold", node = id.0, op = intrinsic_name(op), "constant arithmetic result overflows the narrow width → CDZ0304");
                        Core::Poison(Reject::coded(
                            Code::ConstTrap,
                            "this constant arithmetic operation overflows its integer type (a \
                             compile-provable overflow traps)",
                        ))
                    }
                    _ => Core::ConstInt(r),
                },
                other => other,
            }
        }
        (Core::Poison(r), _) | (_, Core::Poison(r)) => Core::Poison(r),
        // ALGEBRAIC IDENTITY: one operand is a constant whose value makes the op a NO-OP or a constant
        // result — the whole checked operation (and its overflow guard) is eliminated at lowering. Only
        // the identities that are SAFE at every width and never trap are applied (see `arith_identity`);
        // the RESULT keeps the op's solved type because the runtime operand shares it (binary-op
        // unification), and a `0`/`1` constant grounds to that width at selection.
        // A CONSTANT-ZERO DIVISOR with a RUNTIME numerator — `(/ n 0)` / `(% n 0)`. The divisor is the
        // compile-time literal `0`, so the operation ALWAYS traps regardless of `n` — there is no runtime
        // value of `n` that makes it valid. Reject CDZ0304 (the same code the both-constant `(/ 10 0)`
        // gets), rather than emitting a component that traps at run time (`numeric-model.md` §A Constant
        // Operation With No Value Is Rejected At Compile Time). This inherits the const-trap machinery's
        // BRANCH SHIELDING: the reached-poison walk does not descend an untaken `if` branch, so `(if false
        // (/ n 0) 1)` is NOT rejected (the trap is unreachable), exactly as the both-constant case is
        // shielded there. Distinct from `(/ n z)` with a runtime `z` that HAPPENS to be 0 at a call (a
        // genuine runtime trap — `z` is a variable, not the literal `0`, so this never fires for it).
        (_, Core::ConstInt(ref b)) if matches!(op, Prim::Div | Prim::Rem) && b.is_zero() => {
            trace!(target: "rcdzc::lower", op = intrinsic_name(op), "divide by a constant zero → CDZ0304 (always traps)");
            Core::Poison(Reject::coded(
                Code::ConstTrap,
                format!(
                    "`{}` by the constant 0 always traps (divide by zero) — guard the divisor or remove \
                     the division",
                    intrinsic_name(op)
                ),
            ))
        }
        (lc, rc) => {
            if let Some(simplified) = arith_identity(db, op, args[0], &lc, args[1], &rc) {
                trace!(target: "rcdzc::lower", op = intrinsic_name(op), "arithmetic identity simplified (op elided)");
                return simplified;
            }
            // OVERFLOW POLICY (STAGE 2c, numeric-model §Overflow Behavior Is Configurable By Policy — the
            // RUNTIME twin of the 2b const-fold above): under a `(pragma overflow (signed|unsigned wrap))`
            // module a RUNTIME `+`/`-`/`*` that overflows must WRAP (two's-complement, mod 2^width) — NOT
            // trap. `overflow_mode_of` resolves this node's authoritative mode off the SAME predicate the
            // both-const arm read (so const/runtime cannot drift; signedness-selective per #5686). We do NOT
            // emit a new backend op: the `WrappingAdd`/`WrappingSub`/`WrappingMul` prims already lower to the
            // raw machine op + a narrow re-normalize (mask/sign-extend, NO overflow guard) in every backend
            // (wasm/rust/cadenza) and are treated first-class by every pass (`is_trap_free`, `arith_identity`,
            // `const_eval`). So rewrite `+`/`-`/`*` to its wrapping twin here — identical value+width to what
            // the 2b const path yields, and to `UInt8.wrapping-add` etc. `Trap` mode / no-pragma leaves the op
            // unchanged (→ the checked-arith emit that traps, as before). Guarded to fixed-width `Int`
            // (peeling `Qty`) to match the const half exactly — BigInt/Rational lower elsewhere and never wrap.
            let op = if matches!(op, Prim::Add | Prim::Sub | Prim::Mul)
                && crate::infer::overflow_mode_of(db, id) == crate::db::OverflowMode::Wrap
                && matches!(
                    peel_qty_inner_ty(crate::infer::type_of(db, id)),
                    crate::ty::Ty::Int(_)
                ) {
                let w = match op {
                    Prim::Add => Prim::WrappingAdd,
                    Prim::Sub => Prim::WrappingSub,
                    _ => Prim::WrappingMul, // Mul (the `matches!` guard admits only Add/Sub/Mul)
                };
                trace!(target: "rcdzc::lower", node = id.0, op = intrinsic_name(op), "runtime +/-/* rewritten to its WRAPPING twin (pragma overflow wrap) — raw machine op, no trap guard");
                w
            } else {
                op
            };
            trace!(target: "rcdzc::lower", op = intrinsic_name(op), "arithmetic stays runtime (operand not constant)");
            Core::Arith {
                op,
                lhs: args[0],
                rhs: args[1],
            }
        }
    }
}

/// Lower a FLOAT arithmetic application (`+.`/`-.`/`*.`/`/.`). FOLDS two constant floats at the solved
/// float WIDTH (the `Decimal` operands round to the width's IEEE format, the op runs, the result rounds
/// back — round-to-nearest-even, the fixed deterministic mode); a non-constant operand DECLINES (runtime
/// float ops emit the machine `f64.add`/… in a later increment). Unlike integer arithmetic there is NO
/// checked-trap: an IEEE overflow yields an infinity — but a NON-FINITE result has no written value form
/// (the float-literal-overflow rule), so a fold to `±inf`/NaN DECLINES rather than producing a bad value.
fn lower_float_arith(db: &mut Db, id: StructId, op: Prim, args: &[StructId]) -> Core {
    if args.len() != 2 {
        return Core::Poison(binop_arity_reject(op, args));
    }
    let lhs = core_of(db, args[0]);
    let rhs = core_of(db, args[1]);
    match (lhs, rhs) {
        (Core::Poison(r), _) | (_, Core::Poison(r)) => Core::Poison(r),
        (Core::ConstFloat(a), Core::ConstFloat(b)) => {
            // The result WIDTH is the application's solved type (both operands unify to it). Fold at that
            // width: round each operand to the width's format, compute, round the result back.
            let width = match crate::infer::type_of(db, id) {
                crate::ty::Ty::Float(ft) => ft.ground_width(),
                _ => crate::ty::DEFAULT_FLOAT_WIDTH,
            };
            let fold_at = |x: f64, y: f64| -> f64 {
                let r = match op {
                    Prim::FAdd => x + y,
                    Prim::FSub => x - y,
                    Prim::FMul => x * y,
                    Prim::FDiv => x / y,
                    _ => f64::NAN,
                };
                // A `Float32` result rounds through binary32 (`as f32 as f64`), the fixed narrower mode;
                // `Float64` computes directly. Both round-to-nearest-even (the IEEE default wasm uses).
                if width == 32 { r as f32 as f64 } else { r }
            };
            let (x, y) = if width == 32 {
                (
                    f64::from_bits(a.to_f64_bits()) as f32 as f64,
                    f64::from_bits(b.to_f64_bits()) as f32 as f64,
                )
            } else {
                (
                    f64::from_bits(a.to_f64_bits()),
                    f64::from_bits(b.to_f64_bits()),
                )
            };
            let result = fold_at(x, y);
            match crate::ast::Decimal::from_f64(result) {
                Some(d) => {
                    trace!(target: "rcdzc::lower", op = intrinsic_name(op), width, "folded constant float op");
                    Core::ConstFloat(d)
                }
                // A non-finite result (overflow → ±inf, 0.0/.0 → NaN) has no written value form — decline
                // rather than emit an unrepresentable constant (the float-literal-overflow discipline).
                None => Core::Poison(Reject::decline(
                    "a floating-point operation whose result is not finite has no value form yet",
                )),
            }
        }
        // A runtime float operand — emit the machine `f64.add`/`f32.add`/… at selection (the op's width
        // read off the solved type there, like the integer `Core::Arith`). Float ops never trap, so no
        // overflow guard — just the two operands + the machine op. A poison operand already returned above.
        _ => Core::Arith {
            op,
            lhs: args[0],
            rhs: args[1],
        },
    }
}

/// Lower a `Float64.of-int` / `Float32.of-int` — the TOTAL int→float conversion `Int64 → (Float N)`.
/// FOLD a constant integer to a `Core::ConstFloat` (the value as f64/f32, rounding to the nearest
/// representable float at the target width — total, never trapping); a runtime integer emits
/// `Core::Convert{op: FloatOfInt}` (select → `f{64,32}.convert_i64_s`). The target width is the node's
/// solved `Ty::Float`. No implicit promotion — the conversion is always written (numeric-model.md §A
/// Conversion Involving A Floating-Point Type Is Explicit).
fn lower_float_of_int(db: &mut Db, id: StructId, args: &[StructId]) -> Core {
    if args.len() != 1 {
        return Core::Poison(Reject::coded(
            Code::Malformed,
            "of-int takes exactly 1 operand".to_string(),
        ));
    }
    let width = match crate::infer::type_of(db, id) {
        crate::ty::Ty::Float(ft) => ft.ground_width(),
        _ => {
            return Core::Poison(Reject::decline(
                "a float conversion target is not a definite float type",
            ));
        }
    };
    match core_of(db, args[0]) {
        Core::Poison(r) => Core::Poison(r),
        Core::ConstInt(v) => {
            // Fold: the integer's value as a float at the target width (round-to-nearest-even). A value
            // beyond the finite float range would round to ±inf — but an `Int64` is always finite in f64
            // (|Int64| < 2^63 ≪ f64 max), and f32 of an Int64 is finite too, so this never overflows.
            let Some(i) = v.to_i64() else {
                // A BigInt-magnitude constant (>i64) has no Int64 conversion source here — decline.
                return Core::Poison(Reject::decline(
                    "of-int of a value wider than Int64 is not supported",
                ));
            };
            let f = if width == 32 {
                i as f32 as f64
            } else {
                i as f64
            };
            match crate::ast::Decimal::from_f64(f) {
                Some(d) => {
                    trace!(target: "rcdzc::lower", width, "folded constant of-int to a float");
                    Core::ConstFloat(d)
                }
                None => Core::Poison(Reject::decline(
                    "a float conversion whose result is not finite has no value form",
                )),
            }
        }
        // A runtime integer operand — emit the machine int→float convert at selection (target width read
        // off the solved type there). Total, so no guard.
        _ => Core::Convert {
            op: Prim::FloatOfInt,
            operand: args[0],
        },
    }
}

/// Lower a `Float64.of` / `Float32.of` — the TOTAL float-WIDTH conversion `Float M → (Float N)` (promote
/// / demote / identity). FOLD a constant float in TWO steps: (1) read the source AT THE SOURCE OPERAND'S
/// OWN width (`const_float_bits_at_operand_width` — a `Float32` source literal IS its binary32 value, so
/// demote before converting; a `Float64` source is unchanged), then (2) round to the TARGET width (this
/// node's solved `Ty::Float`): narrowing rounds to nearest under the fixed mode, a promote/identity of
/// the source-width value is exact. Step (1) is what makes a WIDEN of a `Float32` source correct
/// (`(Float64.of (: 0.1 Float32))` promotes `0.1f32`, not the un-demoted f64 literal — the adv-61
/// fold-precision class in the width conversion). A runtime float emits `Core::Convert{op:FloatOf}`
/// (select → demote/promote/nothing). Total — a float always has an image at another float width.
fn lower_float_of(db: &mut Db, id: StructId, args: &[StructId]) -> Core {
    if args.len() != 1 {
        return Core::Poison(Reject::coded(
            Code::Malformed,
            "of takes exactly 1 operand".to_string(),
        ));
    }
    let width = match crate::infer::type_of(db, id) {
        crate::ty::Ty::Float(ft) => ft.ground_width(),
        _ => {
            return Core::Poison(Reject::decline(
                "a float conversion target is not a definite float type",
            ));
        }
    };
    match core_of(db, args[0]) {
        Core::Poison(r) => Core::Poison(r),
        Core::ConstFloat(d) => {
            // Read the source value AT THE SOURCE OPERAND'S OWN width first — a `Float32` source literal IS
            // its binary32 value, so demote before converting (else a widen/identity `Float64.of` of a
            // `Float32` promotes the un-demoted f64 payload the reader parsed, diverging from the runtime
            // `f32.promote` of the real f32 slot: `(Float64.of (: 0.1 Float32))` → 0.1 instead of
            // 0.10000000149011612 — the adv-61 fold-precision class, here in the width conversion).
            let src = f64::from_bits(const_float_bits_at_operand_width(
                db,
                args[0],
                d.to_f64_bits(),
            ));
            // Then round the source value to the TARGET width: `as f32 as f64` for a Float32 target
            // (narrowing rounds to nearest binary32), the value unchanged for a Float64 target
            // (a promote/identity is exact). Rounding once at the target width matches the runtime op.
            let rounded = if width == 32 { src as f32 as f64 } else { src };
            match crate::ast::Decimal::from_f64(rounded) {
                Some(nd) => {
                    trace!(target: "rcdzc::lower", width, "folded constant float-width conversion");
                    Core::ConstFloat(nd)
                }
                None => Core::Poison(Reject::decline(
                    "a float conversion whose result is not finite has no value form",
                )),
            }
        }
        // A runtime float operand — emit the machine demote/promote at selection (source + target widths
        // read off the solved types there). Total, no guard.
        _ => Core::Convert {
            op: Prim::FloatOf,
            operand: args[0],
        },
    }
}

/// Apply a SAFE algebraic identity to a runtime arithmetic op with ONE constant operand, returning the
/// simplified core (the runtime operand's own core, or a constant) — or `None` when no identity applies
/// and the op stays a runtime `Arith`. `lc`/`rc` are the already-lowered operand cores; `lhs`/`rhs`
/// their AST occurrences. Every identity here is exact at EVERY width and never CHANGES the value; the
/// PASSTHROUGH identities keep the runtime operand (so its own traps still fire), while the ANNIHILATOR
/// identities (`x*0`, `x&0` → `0`) DISCARD the operand and so are applied ONLY when the discarded
/// operand cannot trap (`is_trap_free`) — else eliding it would drop a defined trap (`(* (/ a b) 0)`
/// must still trap on `b==0`; `numeric-model.md`/§div traps are defined outcomes, not to be optimized
/// away). Applied identities:
///  - `x + 0` = `0 + x` = `x - 0` = `x` (adding/subtracting 0 never overflows; keeps x);
///  - `x * 1` = `1 * x` = `x` (keeps x); `x * 0` = `0 * x` = `0` (ONLY if x is trap-free — discards x);
///  - `x | 0` = `0 | x` = `x ^ 0` = `0 ^ x` = `x` (keeps x); `x & 0` = `0 & x` = `0` (trap-free x only);
///  - `x << 0` = `x >> 0` = `x` (a zero shift COUNT is a no-op — count is the RIGHT operand; keeps x).
///
/// Deliberately NOT applied HERE: `0 - x` (negation traps at MIN), `x & allbits` (all-ones is width-
/// dependent), `0 << x` / `0 >> x` (a non-constant count must still trap if out of range). NOTE: the
/// STRENGTH REDUCTION `x * 2^k → x << k` is not a value-identity (it rewrites the op, not elides it), so
/// it lives at the SELECTION tier (`emit`'s `Core::Arith` Mul arm → `emit_mul_pow2_as_shift`), where the
/// shift's cheaper round-trip overflow check replaces the mul's division-based one — sound because a
/// left shift is EXACT multiplication by a power of two with the SAME defined overflow-trap
/// (`numeric-model.md` §Overflow Is Defined for shifts).
fn arith_identity(
    db: &mut Db,
    op: Prim,
    lhs: StructId,
    lc: &Core,
    rhs: StructId,
    rc: &Core,
) -> Option<Core> {
    // A constant operand's value tested against a small literal (0 or 1), by value (magnitude-agnostic).
    let is =
        |c: &Core, k: i64| matches!(c, Core::ConstInt(v) if v.eq_value(&IntValue::from_i64(k)));
    let zero = || Core::ConstInt(IntValue::from_i64(0));
    match op {
        // `x + 0` / `0 + x` → x.
        Prim::Add if is(rc, 0) => Some(lc.clone()),
        Prim::Add if is(lc, 0) => Some(rc.clone()),
        // `x - 0` → x. (`0 - x` is negation — NOT an identity, would need a trap-checked negate.)
        Prim::Sub if is(rc, 0) => Some(lc.clone()),
        // `x * 1` / `1 * x` → x (keeps x).
        Prim::Mul if is(rc, 1) => Some(lc.clone()),
        Prim::Mul if is(lc, 1) => Some(rc.clone()),
        // `x * 0` / `0 * x` → 0 — DISCARDS x, so only when x cannot trap.
        Prim::Mul if is(rc, 0) && is_trap_free(db, lhs) => Some(zero()),
        Prim::Mul if is(lc, 0) && is_trap_free(db, rhs) => Some(zero()),
        // `x * -1` / `-1 * x` → NEGATION `(- 0 x)` — a strength reduction. A full-width `* -1` keeps the
        // expensive `div_s` round-trip overflow guard (the constant-multiplier fast path EXCLUDES `-1`,
        // since its `MIN/-1` bound is not i64-representable), but negation `0 - x` has the SAME single
        // overflow — `x == MIN` (`-MIN` is unrepresentable) — emitted as one `eq` check (the negation fast
        // path in `emit_machine_overflow_guard`). Value- AND trap-identical: `x * -1 == -x`, both overflow
        // iff `x == MIN`. Rewrite to `(- 0 x)`, synthesizing the `0` (the `Leaf::Int` node-synth pattern);
        // `x` STAYS an operand (the subtrahend), so its OWN traps are preserved — no `is_trap_free` guard
        // needed (unlike `* 0`, which discards `x`). (A NARROW `* -1` already sheds the div via the
        // narrow-product-fits-slot path, so this mainly helps full width — but the rewrite is correct at
        // every width: `0 - x` narrows/range-checks exactly as the narrow `* -1` result does.)
        Prim::Mul if is(rc, -1) || is(lc, -1) => {
            let x = if is(rc, -1) { lhs } else { rhs };
            let z = db.push_atom(crate::ast::Leaf::Int {
                value: IntValue::from_i64(0),
                radix: crate::ast::Radix::Dec,
            });
            trace!(target: "rcdzc::fold", "x * -1 → negation (- 0 x)");
            Some(Core::Arith {
                op: Prim::Sub,
                lhs: z,
                rhs: x,
            })
        }
        // WRAPPING arithmetic has the SAME algebraic identities as checked `+`/`*` — the wrap is total,
        // so it never traps and the fold is value-identical (`a +% 0 = a`, `a *% 1 = a`, `a *% 0 = 0`).
        // The keeping folds preserve the surviving operand's traps; the annihilator `*% 0` DISCARDS the
        // other operand, so it too is guarded on trap-freedom (`(/ x 0) *% 0` must still trap).
        Prim::WrappingAdd if is(rc, 0) => Some(lc.clone()),
        Prim::WrappingAdd if is(lc, 0) => Some(rc.clone()),
        // `a -% 0 = a` — the RIGHT-zero identity only (subtraction is not commutative, so `0 -% a` is the
        // negation of `a`, NOT `a`, and does not simplify here).
        Prim::WrappingSub if is(rc, 0) => Some(lc.clone()),
        Prim::WrappingMul if is(rc, 1) => Some(lc.clone()),
        Prim::WrappingMul if is(lc, 1) => Some(rc.clone()),
        Prim::WrappingMul if is(rc, 0) && is_trap_free(db, lhs) => Some(zero()),
        Prim::WrappingMul if is(lc, 0) && is_trap_free(db, rhs) => Some(zero()),
        // `x | 0` / `0 | x` / `x ^ 0` / `0 ^ x` → x.
        Prim::BitOr | Prim::BitXor if is(rc, 0) => Some(lc.clone()),
        Prim::BitOr | Prim::BitXor if is(lc, 0) => Some(rc.clone()),
        // `x & 0` / `0 & x` → 0 — DISCARDS x, so only when x cannot trap.
        Prim::BitAnd if is(rc, 0) && is_trap_free(db, lhs) => Some(zero()),
        Prim::BitAnd if is(lc, 0) && is_trap_free(db, rhs) => Some(zero()),
        // `x & M` / `M & x` → x when the constant `M` has ALL of `x`'s value bits set — a redundant mask.
        // An UNSIGNED width-N `x` lives in `[0, 2^N)`, so if `M`'s low N bits are all 1s the `&` cannot
        // clear anything (`x & M == x`). Restricted to UNSIGNED: a SIGNED value's slot high bits are sign
        // extension, which a mask WOULD clear (changing negatives), so `& fullmask` is not the identity
        // there. `x` keeps its own traps (the operand is returned). `x` is the value operand, `M` the
        // constant — `(& x M)` returns `lc` (x) when `rc` (M) masks x's whole width; symmetric for `(& M x)`.
        Prim::BitAnd if is_full_mask_for(db, lhs, rc) => Some(lc.clone()),
        Prim::BitAnd if is_full_mask_for(db, rhs, lc) => Some(rc.clone()),
        // OR-THEN-MASK ABSORPTION: `(& (| v C1) C2)` → `C2` when `C2 ⊆ C1` (`C2 & C1 == C2`). The inner OR
        // sets every bit of C1 (⊇ C2), the outer mask keeps only C2's bits — all of which are 1 — so the
        // result is exactly the constant C2, independent of `v`. `(& (| x 15) 15)` → 15. DISCARDS `v`, so
        // gated on `is_trap_free`. `C2` is the constant operand of the outer `&`; the inner `(| v C1)` is
        // the other. Both operand orders of the outer `&` are tried.
        Prim::BitAnd
            if let Core::ConstInt(c2) = rc
                && let Some(c2v) = c2.to_i64()
                && let Some(v) = or_then_mask_absorbs(db, lhs, c2v)
                && is_trap_free(db, v) =>
        {
            Some(rc.clone())
        }
        Prim::BitAnd
            if let Core::ConstInt(c2) = lc
                && let Some(c2v) = c2.to_i64()
                && let Some(v) = or_then_mask_absorbs(db, rhs, c2v)
                && is_trap_free(db, v) =>
        {
            Some(lc.clone())
        }
        // `x | M` / `M | x` → M when the constant `M` covers ALL of `x`'s value bits — the OR-SATURATION
        // dual of the `&`-mask elision. `x | M` sets every bit of M plus x's bits; if M already has all the
        // bits x could set (`is_full_mask_for`: x nonneg in `[0, 2^B)`, M's low B bits all 1), the OR adds
        // nothing NEW and the result is exactly M (`(| x 255)` with `x ∈ [0,255]` → 255). DISCARDS x (the
        // result is the constant M, not x), so — like `& 0`/`* 0` — only when x is TRAP-FREE (a trapping x
        // must still trap). Returns the CONSTANT operand's core (M). Same `is_full_mask_for` predicate as
        // the `&` fold, so it too fires on an emit-refined range via the emit-time sibling.
        Prim::BitOr if is_full_mask_for(db, lhs, rc) && is_trap_free(db, lhs) => Some(rc.clone()),
        Prim::BitOr if is_full_mask_for(db, rhs, lc) && is_trap_free(db, rhs) => Some(lc.clone()),
        // XOR CANCELLATION: `(^ (^ v w) w)` → `v` — the two XORs by the SAME `w` (constant OR runtime)
        // cancel (`w ^ w == 0`, and `v ^ 0 == v`). Handled BEFORE the nested-bitwise collapse so the
        // constant case produces `v` DIRECTLY rather than a residual `(^ v 0)` the collapse would leave
        // (which does not re-simplify). DISCARDS `w`, so gated on `is_trap_free(w)`. `v` stays, traps kept.
        Prim::BitXor
            if let Some((v, w)) = xor_cancels(db, lhs, rhs)
                && is_trap_free(db, w) =>
        {
            trace!(target: "rcdzc::fold", node = v.0, "XOR cancellation (^ (^ v w) w) → v");
            Some(core_of(db, v))
        }
        // IDEMPOTENT-BITWISE COLLAPSE: `(OP (OP v w) w)` → `(OP v w)` for `&`/`|` (idempotent: `w OP w == w`,
        // so re-applying `OP w` changes nothing), where the outer operand is `core_equiv` to `w` in the
        // inner op. Covers a RUNTIME `w` (`(| (| x y) y)` → `(| x y)`); the CONSTANT case already collapses
        // via `nested_bitwise_collapse` (`(| x (w|w))` = `(| x w)`). Unlike XOR-cancel, this KEEPS the inner
        // `(OP v w)` node — BOTH operands survive — so NO `is_trap_free` guard is needed (any trap in `v`/`w`
        // is still evaluated). Returns the inner node's core. `nested_shift_combine` — not `nested_bitwise_
        // collapse` — placement before it is fine (the collapse only fires on a CONSTANT operand, distinct
        // from this same-runtime-operand shape). `idempotent_bitwise_collapse` returns the inner node.
        Prim::BitAnd | Prim::BitOr
            if let Some(inner) = idempotent_bitwise_collapse(db, op, lhs, rhs) =>
        {
            trace!(target: "rcdzc::fold", node = inner.0, ?op, "idempotent bitwise (OP (OP v w) w) → (OP v w)");
            Some(core_of(db, inner))
        }
        // ABSORPTION LAW: `x & (x | y)` → `x` and `x | (x & y)` → `x` — a value combined with the DUAL op of
        // itself-with-anything absorbs to itself. The outer op is `&`/`|` and one operand is an inner op of
        // the DUAL kind (`| ` under `&`, `&` under `|`) that CONTAINS `x` (either side); the OTHER outer
        // operand is `x` (`core_equiv`). Result is `x`. DISCARDS the inner op's OTHER operand `y`, so gated
        // on `is_trap_free(y)` (a trapping `y` must still trap). `x` is returned so its own traps stay. Both
        // outer orders and both inner-operand positions are tried by `absorption_operand`.
        Prim::BitAnd | Prim::BitOr
            if let Some((x, y)) = absorption_operand(db, op, lhs, rhs)
                && is_trap_free(db, y) =>
        {
            trace!(target: "rcdzc::fold", node = x.0, ?op, "absorption law (x OP (x DUAL y)) → x");
            Some(core_of(db, x))
        }
        // NESTED-BITWISE COLLAPSE: `(OP (OP v C1) C2)` → `(OP v (C1 ⊙ C2))` for a TOTAL, ASSOCIATIVE
        // bitwise op — `&`/`|`/`^`. Two constant operations on the same value collapse to ONE by folding
        // the constants (`(& (& x 255) 15)`→`(& x 15)`, `(| (| x 5) 3)`→`(| x 7)`, `(^ (^ x 5) 3)`→`(^ x
        // 6)`); the `&` case's folded constant also enables downstream range folds. `v` keeps its own
        // traps (it stays the operand). Guarded on the shape (`nested_bitwise_collapse` returns `None`
        // when it does not apply) so the same-operand `& a a`/`| a a` fold below still fires. Verified
        // value-identical: each op is associative, so `(v OP C1) OP C2 == v OP (C1 ⊙ C2)`.
        Prim::BitAnd | Prim::BitOr | Prim::BitXor
            if let Some(folded) = nested_bitwise_collapse(db, op, lhs, lc, rhs, rc) =>
        {
            Some(folded)
        }
        // `x << 0` / `x >> 0` → x (a zero shift COUNT is a no-op; count is the right operand).
        Prim::Shl | Prim::Shr if is(rc, 0) => Some(lc.clone()),
        // NESTED SHIFT COLLAPSE: `(SH (SH v A) B)` → `(SH v (A+B))` for the SAME shift direction, A, B
        // constants, A+B < width. A RIGHT shift is TOTAL — shifting right by A then B drops the same low
        // A+B bits as one shift by A+B (both `>>ₛ` sign-fill and `>>ᵤ` zero-fill; the inner and outer `>>`
        // on the same-typed value are the same kind, so composing is exact). A LEFT shift is CHECKED (it
        // is exact `·2^count`, trapping on N-bit overflow) but STILL collapses trap-identically: magnitude
        // is MONOTONIC in the count, so `(v<<A)<<B` overflows on exactly the inputs `v<<(A+B)` does (inner
        // overflow ⟹ combined overflow, and combined overflow ⟹ the double's outer step overflows) — same
        // value `v·2^(A+B)` when neither traps, same trap set otherwise. Bounded by A+B < width for BOTH:
        // a combined count ≥ width is masked mod width by the machine op (wrong), and for `<<` it must also
        // TRAP as an out-of-range count — so only the in-range sum is faithful. `v` keeps its own traps (it
        // stays the operand). Guarded via `nested_shift_combine`.
        Prim::Shr | Prim::Shl if let Some(folded) = nested_shift_combine(db, op, lhs, rc) => {
            Some(folded)
        }
        // `(>>ᵤ x k)` → 0 when the LOGICAL right shift drops ALL of `x`'s significant bits — its provable
        // bit-bound `B <= k`. E.g. `(x & 15) >>ᵤ 4`: `x & 15` fits 4 bits, `>>ᵤ 4` shifts them all out → 0.
        // DISCARDS `x`, so gated on `is_trap_free` (a trapping operand's trap must survive). `k` must be a
        // valid IN-RANGE constant count (`< width`) — an out-of-range shift TRAPS rather than yielding 0,
        // so a too-large `k` is left for the runtime count-guard. `unsigned_value_bits` returns the bound
        // only for an unsigned logical-shift chain, so this never misfires on a signed `>>ₛ` (which
        // sign-extends, not zero-fills).
        Prim::Shr
            if is_trap_free(db, lhs)
                && let Core::ConstInt(k) = rc
                && let Some(k) = k.to_i64()
                && k >= 1
                && let Some(bits) = unsigned_value_bits(db, lhs)
                && (k as u32) < shift_width(db, lhs)
                && bits <= k as u32 =>
        {
            trace!(target: "rcdzc::fold", node = lhs.0, k, bits, "logical shift drops all significant bits → 0");
            Some(zero())
        }
        // `x / 1` → x (division by one is the identity; keeps x, so its own traps stay).
        Prim::Div if is(rc, 1) => Some(lc.clone()),
        // `x % 1` → 0 (every integer is divisible by 1) — DISCARDS x, so only when x cannot trap.
        Prim::Rem if is(rc, 1) && is_trap_free(db, lhs) => Some(zero()),
        // DIVIDEND-SMALLER-THAN-DIVISOR: when `x` is provably in `[0, C-1]` for a POSITIVE constant divisor
        // `C`, the truncating `x / C` is 0 and `x % C` is `x` — the divisor is too big to divide `x` even
        // once. `(/ (& x 7) 100)` → 0, `(% (& x 7) 100)` → `x & 7` (a masked/refined value modding by a
        // larger constant). Requires `x` NONNEGATIVE with a known upper bound `< C` (`value_range` lo ≥ 0,
        // hi < C) so truncation-toward-zero equals the mathematical result; a negative `x` (`-1 % 100 =
        // -1`, `-1 / 100 = 0`) is excluded for simplicity (the nonneg case is the masked/unsigned idiom).
        // The `/` DISCARDS `x` → gated on `is_trap_free`; the `%` KEEPS `x` (returns `lc`) so its traps
        // survive. `C ≥ 2` (the `/1`,`%1` identities above handle `C=1`; a constant `÷0` is a poison in
        // `lower` before here). Verified: for `0 ≤ x < C`, `x/C == 0` and `x%C == x`.
        Prim::Div
            if let Core::ConstInt(c) = rc
                && let Some(c) = c.to_i64()
                && c >= 2
                && dividend_below_divisor(db, lhs, c)
                && is_trap_free(db, lhs) =>
        {
            trace!(target: "rcdzc::fold", node = lhs.0, c, "dividend provably < divisor → x / C = 0");
            Some(zero())
        }
        Prim::Rem
            if let Core::ConstInt(c) = rc
                && let Some(c) = c.to_i64()
                && c >= 2
                && dividend_below_divisor(db, lhs, c) =>
        {
            trace!(target: "rcdzc::fold", node = lhs.0, c, "dividend provably < divisor → x % C = x");
            Some(lc.clone())
        }
        // NESTED-MODULO COLLAPSE: `(% (% v M) N)` → `(% v N)` when the outer divisor `N` DIVIDES the inner
        // `M` (`M % N == 0`), both positive constants. Since `M` is a multiple of `N`, reducing mod `M`
        // first then mod `N` gives the same residue as reducing mod `N` directly — for truncated (toward-
        // zero) division at every sign of `v` (`(x%100)%10 == x%10`, incl. negatives: `-25%100=-25`,
        // `-25%10=-5`, and `-25%10=-5` directly). One `rem` instead of two. `v` STAYS the operand of the
        // outer `% N`, so its own traps (and the outer `% N`'s ÷0 — impossible here, N≥2) are preserved; no
        // `is_trap_free` needed. Both divisors must be constants ≥ 2 and `N | M`.
        Prim::Rem
            if let Core::ConstInt(n) = rc
                && let Some(n) = n.to_i64()
                && n >= 2
                && let Core::Arith {
                    op: Prim::Rem,
                    lhs: v,
                    rhs: inner_div,
                } = core_of(db, lhs)
                && let Core::ConstInt(mm) = core_of(db, inner_div)
                && let Some(m) = mm.to_i64()
                && m >= 2
                && m % n == 0 =>
        {
            trace!(target: "rcdzc::fold", inner_m = m, outer_n = n, "nested modulo (% (% v M) N) → (% v N) (N | M)");
            Some(Core::Arith {
                op: Prim::Rem,
                lhs: v,
                rhs,
            })
        }

        // COMPLEMENT LAWS: `x & ~x` → 0 and `x | ~x` → -1 (all-ones), where `~x` is `(^ x -1)` (there is no
        // dedicated bit-NOT prim). A value AND its bitwise complement share NO set bit, so `&` is 0 and `|`
        // is every bit set. Both DISCARD `x` (the result does not depend on it), so gated on
        // `is_trap_free(x)` — a trapping `x` must still trap. `complement_pair` matches `v` against
        // `(^ v -1)` on either operand order.
        //
        // The `&` result 0 is valid at EVERY width/sign. But the `|` all-ones result is `-1` only for a
        // SIGNED type; an UNSIGNED width-N all-ones is `2^N − 1`, and a literal `-1` is OUT OF RANGE there
        // (`(: -1 UInt8)` is a CDZ0302 reject) — `arith_identity` has no width to synthesize `2^N−1`. So the
        // `|` fold is restricted to a SIGNED operand type, where `-1` IS the all-ones and representable;
        // an unsigned `x | ~x` keeps the runtime `or` (correct, just not folded).
        Prim::BitAnd if complement_pair(db, lhs, rhs).is_some_and(|v| is_trap_free(db, v)) => {
            trace!(target: "rcdzc::fold", "complement law x & ~x → 0");
            Some(zero())
        }
        Prim::BitOr
            if matches!(crate::infer::type_of(db, lhs), crate::ty::Ty::Int(it) if it.ground_signed())
                && complement_pair(db, lhs, rhs).is_some_and(|v| is_trap_free(db, v)) =>
        {
            trace!(target: "rcdzc::fold", "complement law x | ~x → -1 (all ones, signed)");
            Some(Core::ConstInt(IntValue::from_i64(-1)))
        }
        // SAME-OPERAND identities: the two operands are the SAME value (`core_equiv`), so the result is
        // determined regardless of that value. `core_equiv` matches only pure scalar cores, but the
        // operand may still be a checked op that TRAPS (`(- (/ a b) (/ a b))` — the `/` traps on b==0),
        // so a DISCARDING identity (`- a a → 0`, `^ a a → 0`) fires only when the operand is trap-free;
        // eliding a possibly-trapping operand would drop a defined trap. The KEEPING identities
        // (`& a a → a`, `| a a → a`) return the operand's own core, so its traps are preserved — always
        // safe. (`/ a a → 1` is NOT applied: `a == 0` traps ÷0, a defined outcome, so it is not an
        // identity.)
        Prim::Sub | Prim::BitXor if core_equiv(db, lhs, rhs) && is_trap_free(db, lhs) => {
            Some(zero())
        }
        Prim::BitAnd | Prim::BitOr if core_equiv(db, lhs, rhs) => Some(lc.clone()),
        _ => None,
    }
}

/// For an outer bitwise op with operands `(lhs, rhs)`, whether one operand is the bitwise COMPLEMENT of
/// the other — i.e. one is `v` and the other is `(^ v -1)` (`~v`). Returns the un-complemented value `v`
/// (so the caller can trap-check it, since the complement laws `x & ~x = 0` / `x | ~x = -1` DISCARD `x`).
/// Both operand orders are tried, and the `-1` may be on either side of the inner XOR. `None` otherwise.
fn complement_pair(db: &mut Db, lhs: StructId, rhs: StructId) -> Option<StructId> {
    // Whether `maybe_not` is `(^ v -1)` for a `v` that is `core_equiv` to `other`.
    let is_not_of = |db: &mut Db, maybe_not: StructId, other: StructId| -> bool {
        let Core::Arith {
            op: Prim::BitXor,
            lhs: il,
            rhs: ir,
        } = core_of(db, maybe_not)
        else {
            return false;
        };
        let is_neg1 =
            |c: &Core| matches!(c, Core::ConstInt(v) if v.eq_value(&IntValue::from_i64(-1)));
        // `(^ v -1)` — the `-1` on the right (`v` = il) or left (`v` = ir), and `v` matches `other`.
        (is_neg1(&core_of(db, ir)) && core_equiv(db, il, other))
            || (is_neg1(&core_of(db, il)) && core_equiv(db, ir, other))
    };
    if is_not_of(db, rhs, lhs) {
        Some(lhs) // `(op v (^ v -1))`
    } else if is_not_of(db, lhs, rhs) {
        Some(rhs) // `(op (^ v -1) v)`
    } else {
        None
    }
}

/// Whether `lhs`/`rhs` are BOOLEAN complements — one is `v` and the other is `(not v)` (`Core::Not` whose
/// operand is `core_equiv` to the first). The `and`/`or` analogue of the bitwise `complement_pair`; drives
/// the boolean complement laws `(and a (not a))` → false, `(or a (not a))` → true. Both operand orders are
/// tried. (`Core::Not` is `lower`'s canonical boolean negation — a `(not (not a))` already cancelled in the
/// `Resolved::Not` fold, so a `Not` here wraps a non-`Not` operand.)
fn bool_complement_pair(db: &mut Db, lhs: StructId, rhs: StructId) -> bool {
    let is_not_of = |db: &mut Db, maybe_not: StructId, other: StructId| -> bool {
        matches!(core_of(db, maybe_not), Core::Not { operand } if core_equiv(db, operand, other))
    };
    is_not_of(db, rhs, lhs) || is_not_of(db, lhs, rhs)
}

/// Fold a SHORT-CIRCUIT CONNECTIVE `(and/or lhs rhs)` (the `is_and` flag selects) into its simplest core.
/// Shared by the `Resolved::And` arm AND the `(if c a false)`→`(and c a)` / `(if c true b)`→`(or c b)`
/// if-encoded-connective rewrites — an if-shaped connective routes through the SAME boolean-algebra fold
/// family (constant short-circuit, idempotence, absorption, complement, and the comparison-pair folds).
///
/// A constant LEFT operand short-circuits WITHOUT evaluating `rhs` (a trapping/ill-formed `rhs` is
/// shielded, exactly as an `if`'s unselected branch): `(and false _)`→false, `(and true rhs)`→rhs;
/// `(or true _)`→true, `(or false rhs)`→rhs. Otherwise `lhs` is the always-evaluated short-circuit
/// condition; the arms below simplify against a constant/structural `rhs`, and the fallthrough emits a
/// `Core::And` the backend lowers to `if lhs then/else <rhs|const>`.
fn fold_short_circuit(db: &mut Db, lhs: StructId, rhs: StructId, is_and: bool) -> Core {
    match core_of(db, lhs) {
        Core::ConstBool(b) => {
            // `and`: left decides when false (short-circuit to false), else the result is rhs.
            // `or`:  left decides when true  (short-circuit to true),  else the result is rhs.
            if b == is_and {
                core_of(db, rhs) // and-true → rhs ; or-false → rhs
            } else {
                Core::ConstBool(!is_and) // and-false → false ; or-true → true
            }
        }
        Core::Poison(r) => Core::Poison(r),
        // A constant RIGHT operand (the left is a non-constant runtime bool, ALWAYS evaluated — it is
        // the short-circuit condition). `(and p true)` / `(or p false)` → `p` (the neutral element,
        // keeps `p` so its effects/traps stay). `(and p false)` → `false` / `(or p true)` → `true`
        // (the ABSORBING element) — this DISCARDS `p`, so it is applied only when `p` is trap-free
        // (else `p`'s trap must still fire, so keep the `Core::And`). Mirrors the constant-left fold
        // above; completes the boolean-identity set. (Both-constant folded via the left arm already.)
        lc => match core_of(db, rhs) {
            Core::ConstBool(rb) if rb == is_and => lc, // and-true / or-false → p (neutral, keeps p)
            Core::ConstBool(_) if is_trap_free(db, lhs) => Core::ConstBool(!is_and), // absorbing
            // IDEMPOTENCE: `(and a a)` → `a` and `(or a a)` → `a` — a boolean combined with itself is
            // itself. The two operands are the SAME value (`core_equiv`), so the result is `a`. `lhs` is
            // the short-circuit condition, ALWAYS evaluated (and evaluated ONCE by returning its core),
            // so `a`'s own effects/traps are preserved regardless of the fold — no `is_trap_free` guard
            // needed (`lhs` runs exactly as it would as the condition; `rhs`, a re-evaluation of the
            // same pure value, is dropped). Mirrors the bitwise `(& a a)`/`(| a a)` same-operand fold.
            _ if core_equiv(db, lhs, rhs) => lc,
            // NESTED IDEMPOTENCE / ABSORPTION: `(and (and a b) a)` → `(and a b)` and `(or (or a b) a)` →
            // `(or a b)` — one operand is a nested SAME-connective `(and/or p q)` that already CONTAINS
            // the other operand (`p` or `q` is `core_equiv` to it), so re-conjoining/disjoining it is
            // redundant. Returns the nested node (all operands stay evaluated → trap-safe, like the
            // bitwise idempotent collapse c117). Only the SAME connective (`is_and` matches). Both outer
            // orders are tried by `bool_nested_idempotent`.
            _ if let Some(keep) = bool_nested_idempotent(db, lhs, rhs, is_and) => core_of(db, keep),
            // ABSORPTION LAW: `(and a (or a b))` → `a` and `(or a (and a b))` → `a` — a boolean combined
            // with the DUAL connective of itself-with-anything absorbs to itself (the short-circuit
            // analogue of the bitwise `x & (x|y)`→x / `x | (x&y)`→x fold, c118). One operand is an inner
            // `and`/`or` of the DUAL connective CONTAINING `x`; the other is `x`. Result is `x`. DISCARDS
            // the inner op's OTHER operand `y`, so gated on `is_trap_free(y)` — `y` is only conditionally
            // evaluated in the short-circuit original, so trap-freedom suffices to drop it. `x` is pure
            // (`core_equiv`) so returning it evaluates once with no trap. Both orders via
            // `bool_absorption_operand`.
            _ if let Some((x, y)) = bool_absorption_operand(db, lhs, rhs, is_and)
                && is_trap_free(db, y) =>
            {
                core_of(db, x)
            }
            // COMPLEMENT LAW: `(and a (not a))` → `false` and `(or a (not a))` → `true` — a boolean and
            // its negation are exhaustive+exclusive, so `and` is always false and `or` always true. The
            // boolean analogue of the bitwise `x & ~x`/`x | ~x` fold (c119). DISCARDS both operands (the
            // result is a constant), so gated on `is_trap_free(lhs)` — a trapping `a` must still trap
            // (`core_equiv` matches only pure cores, so `a` is pure anyway, but keep the guard explicit).
            // Both operand orders (`a`&`!a` / `!a`&`a`) are handled by `bool_complement_pair`.
            _ if bool_complement_pair(db, lhs, rhs) && is_trap_free(db, lhs) => {
                Core::ConstBool(!is_and) // and → false ; or → true
            }
            // COMPLEMENTARY-COMPARISON LAW: `(or (< a b) (>= a b))` → true, `(and (< a b) (>= a b))` →
            // false — two comparisons on the SAME operand PAIR whose operators are exact COMPLEMENTS
            // (`< `↔`>=`, `<=`↔`>`) partition the total order, so their `or` is exhaustive (always true)
            // and their `and` is exclusive (always false). A redundant range guard (`(or (< x c) (>= x
            // c))`). DISCARDS both operands, so gated on `is_trap_free` for each (a comparison is
            // trap-free iff its operands are; a `(< (/ a b) 5)` with a trapping `/` keeps the runtime
            // form). `complementary_comparisons` checks same-pair + complement-op.
            _ if complementary_comparisons(db, lhs, rhs)
                && is_trap_free(db, lhs)
                && is_trap_free(db, rhs) =>
            {
                Core::ConstBool(!is_and) // or → true ; and → false
            }
            // SUBSUMPTION: two comparisons on the SAME runtime operand `v` against constants with the
            // SAME operator (both `<`, both `<=`, both `>`, or both `>=`) — one implies the other, so
            // the redundant one drops. `and` keeps the STRONGER (tighter bound), `or` the WEAKER
            // (looser): `(and (< v 5) (< v 10))` → `(< v 5)`, `(or (< v 5) (< v 10))` → `(< v 10)`. The
            // kept comparison still evaluates `v` (its trap, if any, is preserved) — no operand is
            // dropped, only the redundant second bound. `subsuming_comparison` returns the occurrence to
            // keep (`lhs` or `rhs`).
            _ if let Some(keep) = subsuming_comparison(db, lhs, rhs, is_and) => core_of(db, keep),
            // COINCIDENT-POINT COLLAPSE: `(and (>= v c) (<= v c))` → `(= v c)` — two INCLUSIVE
            // opposite bounds pinning `v` to a single point ARE equality (`v>=c && v<=c ⟺ v==c`), so
            // three ops (`ge`+`le`+`and`) become one `eq`. Only under `and`; reuses the existing (proven
            // in-type) constant node, so no synthesis / no range guard. DISCARDS the second comparison,
            // so gated on `is_trap_free` for both (like the sibling disjoint/covering fold); the kept
            // `(= v c)` still evaluates `v`. Distinct from disjoint/covering (which folds `L>U` empty /
            // `L<=U+1` covering — the coincident `L==U` point is exactly what THIS fold handles).
            _ if is_and
                && let Some((v, c)) = coincident_point_eq(db, lhs, rhs)
                && is_trap_free(db, lhs)
                && is_trap_free(db, rhs) =>
            {
                Core::Compare {
                    op: Prim::Eq,
                    lhs: v,
                    rhs: c,
                }
            }
            // DISJOINT/COVERING INTERVAL: two comparisons on the SAME operand `v` vs constants forming
            // OPPOSITE-direction half-lines (one an upper bound `v ≤ U`, the other a lower bound `v ≥
            // L`). `and` (intersection `L ≤ v ≤ U`) is EMPTY iff `L > U` → `false`; `or` (union) COVERS
            // everything iff the half-lines touch/overlap (`L ≤ U+1`) → `true`. `(and (< x 5) (> x 10))`
            // → false, `(or (< x 5) (> x 3))` → true. Only the constant verdicts (a non-empty `and` /
            // gapped `or` is not a constant — kept). DISCARDS both operands, so gated on `is_trap_free`.
            _ if let Some(v) = disjoint_or_covering(db, lhs, rhs, is_and)
                && is_trap_free(db, lhs)
                && is_trap_free(db, rhs) =>
            {
                Core::ConstBool(v)
            }
            // EQUALITY-VS-RANGE: one operand is `(= x c)`, the other an ordering comparison `(cmp x k)`
            // on the SAME `x`. Whether `c` satisfies `(cmp c k)` (a compile-time test) decides:
            //   `and`: `sat` → `(= x c)` (the range is redundant given equality); `!sat` → `false`
            //          (equality contradicts the range). `(and (= x 5) (> x 0))` → `(= x 5)`,
            //          `(and (= x 5) (> x 100))` → false.
            //   `or`:  `sat` → `(cmp x k)` (equality is subsumed by the range it satisfies); `!sat` →
            //          keep both (not a constant — `x==c` adds one point outside the range).
            // Each DISCARDS one operand — gated on that operand's `is_trap_free`. `eq_vs_range` returns
            // `(eq_node, range_node, sat)`.
            _ if let Some((eq_node, range_node, sat)) = eq_vs_range(db, lhs, rhs) => {
                if is_and {
                    if sat && is_trap_free(db, range_node) {
                        core_of(db, eq_node) // range redundant → keep the equality
                    } else if !sat && is_trap_free(db, eq_node) && is_trap_free(db, range_node) {
                        Core::ConstBool(false) // contradiction
                    } else {
                        Core::And { lhs, rhs, is_and }
                    }
                } else if sat && is_trap_free(db, eq_node) {
                    core_of(db, range_node) // `or`: equality subsumed → keep the range
                } else {
                    Core::And { lhs, rhs, is_and }
                }
            }
            // REASSOCIATE TO EXPOSE A COMPARISON PAIR across a same-connective nested tree. The pairwise
            // comparison folds above only see the TWO DIRECT operands, so `(and (and (> x 0) (< x 100)) (> x
            // 5))` misses that `(> x 5)` subsumes the buried `(> x 0)`. When one operand is a same-connective
            // `(op P Q)` and the other is a comparison `C`, try folding `C` against `P` (and against `Q`) via
            // `fold_short_circuit`: if that pair COLLAPSES (to a constant or a single kept comparison — i.e.
            // NOT a plain two-operand `Core::And`), rebuild the tree with the collapsed result and the
            // remaining leaf. `(and (and (> x 0) (< x 100)) (> x 5))` → `(and (> x 5) (< x 100))`; a nested
            // COMPLEMENT `(and (and (< x y) …) (>= x y))` → false. SOUND only when every involved leaf (`C`,
            // `P`, `Q`) is TRAP-FREE: `and`/`or` is associative+commutative over pure booleans, so regrouping
            // and reordering is unobservable (no trap/effect order to preserve). `reassociate_comparison_pair`
            // returns the rebuilt `Core` or `None`.
            _ if let Some(folded) = reassociate_comparison_pair(db, lhs, rhs, is_and) => folded,
            _ => Core::And { lhs, rhs, is_and },
        },
    }
}

/// Reassociate a short-circuit `(op lhs rhs)` (connective `is_and`) to expose a COMPARISON PAIR that the
/// direct pairwise folds miss because it is split across a same-connective nested subtree. When one operand
/// is a nested `(op P Q)` (SAME connective) and the OTHER operand `C` is a comparison, this folds `C`
/// against `P` and against `Q` (via `fold_short_circuit`); if either pair COLLAPSES — the recursive fold
/// returns something OTHER than a plain two-operand `Core::And` of those same two nodes (a constant, or a
/// single subsuming comparison) — the whole tree is rebuilt as `(op collapsed remaining_leaf)`, dropping the
/// redundant comparison. `(and (and (> x 0) (< x 100)) (> x 5))` → `(and (> x 5) (< x 100))` (subsumption);
/// `(and (and (< x y) p) (>= x y))` → `false` (complement). Returns `None` when nothing collapses.
///
/// SOUNDNESS: fires ONLY when `C`, `P`, and `Q` are all TRAP-FREE. A short-circuit `and`/`or` over pure
/// (trap-free, effect-free) boolean operands is fully associative AND commutative — there is no evaluation
/// order or trap to preserve — so regrouping `(op (op P Q) C)` as `(op (op C P) Q)` and folding the exposed
/// `(op C P)` pair is behavior-identical. (A non-trap-free leaf could change WHICH branch's trap fires or
/// its order, so it is excluded — the tree stays as-is.) Both outer operand orders and both nested-operand
/// positions are tried.
fn reassociate_comparison_pair(
    db: &mut Db,
    lhs: StructId,
    rhs: StructId,
    is_and: bool,
) -> Option<Core> {
    // Try: `nested` = a same-connective `(op P Q)`, `c` = the other operand (a comparison). Fold `c`
    // against each nested leaf; on a genuine collapse, rebuild `(op collapsed other_leaf)`.
    let try_side = |db: &mut Db, nested: StructId, c: StructId| -> Option<Core> {
        // `c` must be a comparison, and trap-free (a discarding/regrouping fold requires purity).
        if !matches!(core_of(db, c), Core::Compare { .. }) || !is_trap_free(db, c) {
            return None;
        }
        let Core::And {
            lhs: p,
            rhs: q,
            is_and: nested_is_and,
        } = core_of(db, nested)
        else {
            return None;
        };
        if nested_is_and != is_and {
            return None; // must be the SAME connective to reassociate
        }
        // Every leaf must be trap-free so the reassociation is unobservable.
        if !is_trap_free(db, p) || !is_trap_free(db, q) {
            return None;
        }
        // Fold `c` against P, keeping Q; then against Q, keeping P. A genuine collapse = the recursive fold
        // did NOT return a plain `Core::And` re-pairing the same two nodes (that would be no progress).
        let collapsed = |db: &mut Db, pair_a: StructId, pair_b: StructId| -> Option<Core> {
            let folded = fold_short_circuit(db, pair_a, pair_b, is_and);
            match folded {
                // No progress: the pair stayed a two-operand `and`/`or`. (Any other shape — ConstBool, a
                // single Compare, a Not, an Eq — is a real collapse.)
                Core::And { .. } => None,
                other => Some(other),
            }
        };
        if let Some(folded) = collapsed(db, c, p) {
            // `(op (op P Q) C)` → `(op folded(C,P) Q)`.
            let fid = synth_core(db, folded, crate::ty::Ty::Bool);
            return Some(fold_short_circuit(db, fid, q, is_and));
        }
        if let Some(folded) = collapsed(db, c, q) {
            // → `(op folded(C,Q) P)`.
            let fid = synth_core(db, folded, crate::ty::Ty::Bool);
            return Some(fold_short_circuit(db, fid, p, is_and));
        }
        None
    };
    try_side(db, lhs, rhs).or_else(|| try_side(db, rhs, lhs))
}

/// NESTED IDEMPOTENCE for a short-circuit `and`/`or`: when one outer operand is a nested `Core::And` of the
/// SAME connective (`is_and`) that already CONTAINS the other outer operand (one of its sides is
/// `core_equiv` to it), the outer re-application is redundant — `(and (and a b) a)` == `(and a b)`. Returns
/// the NESTED node to keep (all its operands stay evaluated → trap-safe, no operand dropped). Both outer
/// operand orders and both nested-operand positions are tried. `None` when the shape does not match.
fn bool_nested_idempotent(
    db: &mut Db,
    lhs: StructId,
    rhs: StructId,
    is_and: bool,
) -> Option<StructId> {
    // `nested` is `(op p q)` with the SAME connective; `outer` must be `core_equiv` to `p` or `q`.
    let check = |db: &mut Db, nested: StructId, outer: StructId| -> Option<StructId> {
        let Core::And {
            lhs: p,
            rhs: q,
            is_and: nested_is_and,
        } = core_of(db, nested)
        else {
            return None;
        };
        if nested_is_and != is_and {
            return None;
        }
        (core_equiv(db, p, outer) || core_equiv(db, q, outer)).then_some(nested)
    };
    check(db, lhs, rhs).or_else(|| check(db, rhs, lhs))
}

/// The SHORT-CIRCUIT BOOLEAN ABSORPTION LAW: `(and a (or a b))` → `a` and `(or a (and a b))` → `a` (either
/// outer order, `a` on either side of the inner op). A boolean combined with the DUAL connective of
/// itself-with-anything absorbs to itself — the boolean analogue of the bitwise `x & (x|y)`→x / `x | (x&y)`
/// →x fold (c118, `absorption_operand`). The outer connective is `is_and`; one operand must be an inner
/// `Core::And` of the DUAL connective (`or` under `and`, `and` under `or`) that CONTAINS `x` (either side);
/// the OTHER outer operand is `x` (`core_equiv`). Returns `(x, y)` — the whole expression absorbs to `x`,
/// discarding the inner op's OTHER operand `y`. `x` is pure (`core_equiv` matches only pure cores) so
/// returning it evaluates it once with no trap; `y` may be arbitrary, so the caller gates `is_trap_free(y)`
/// (in the short-circuit original `y` is only conditionally evaluated, so trap-freedom is SUFFICIENT to
/// drop it soundly). Both outer orders and both inner-operand positions are tried.
fn bool_absorption_operand(
    db: &mut Db,
    lhs: StructId,
    rhs: StructId,
    is_and: bool,
) -> Option<(StructId, StructId)> {
    // `inner` must be a `Core::And` of the DUAL connective; `outer_x` must be `core_equiv` to one operand.
    let check = |db: &mut Db, inner: StructId, outer_x: StructId| -> Option<(StructId, StructId)> {
        let Core::And {
            lhs: ip,
            rhs: iq,
            is_and: inner_is_and,
        } = core_of(db, inner)
        else {
            return None;
        };
        if inner_is_and == is_and {
            return None; // must be the DUAL connective (`or` under `and`, `and` under `or`)
        }
        if core_equiv(db, ip, outer_x) {
            Some((outer_x, iq)) // x matched ip → y is iq
        } else if core_equiv(db, iq, outer_x) {
            Some((outer_x, ip)) // x matched iq → y is ip
        } else {
            None
        }
    };
    check(db, lhs, rhs).or_else(|| check(db, rhs, lhs))
}

/// Whether `lhs`/`rhs` are two comparisons on the SAME operand pair whose operators are exact COMPLEMENTS
/// over the total order — `< `/`>=` or `<=`/`>` — so together they partition every value: their `or` is
/// always TRUE (exhaustive) and their `and` always FALSE (disjoint). `(or (< a b) (>= a b))` → true,
/// `(and (< a b) (>= a b))` → false. Requires BOTH to be `Core::Compare` with `core_equiv` operand pairs
/// (same order — `< a b` complements `>= a b`, NOT `>= b a`) and complementary ops. `=`/`Compare` are not
/// ordering complements and never match. Drives the complementary-comparison fold (caller trap-guards).
fn complementary_comparisons(db: &mut Db, lhs: StructId, rhs: StructId) -> bool {
    let Core::Compare {
        op: lop,
        lhs: la,
        rhs: lb,
    } = core_of(db, lhs)
    else {
        return false;
    };
    let Core::Compare {
        op: rop,
        lhs: ra,
        rhs: rb,
    } = core_of(db, rhs)
    else {
        return false;
    };
    // Exact ordering complements: `<` ↔ `>=`, `<=` ↔ `>` (either assignment to lhs/rhs).
    let complement = matches!(
        (lop, rop),
        (Prim::Lt, Prim::Ge) | (Prim::Ge, Prim::Lt) | (Prim::Le, Prim::Gt) | (Prim::Gt, Prim::Le)
    );
    // Same operand pair in the SAME order (the operators already encode the direction).
    complement && core_equiv(db, la, ra) && core_equiv(db, lb, rb)
}

/// SUBSUMPTION between two comparisons on the SAME runtime operand `v` against CONSTANTS that form
/// SAME-DIRECTION half-lines (both upper bounds `v ≤ B`, or both lower `v ≥ B`) — one implies the other,
/// so `(and …)`/`(or …)` keeps just one. Returns the occurrence to KEEP (`lhs` or `rhs`), or `None` when
/// the pair is not two same-direction half-lines on the same `v`. `is_and` selects which survives: `and`
/// keeps the STRONGER (tighter) bound, `or` the WEAKER (looser).
///
/// Uses `comparison_halfline` to normalize each side to an INCLUSIVE bound (`v ≤ B` / `v ≥ B`), so MIXED
/// operators are handled uniformly — `(< v 5)` and `(<= v 4)` both normalize to `v ≤ 4` (and keeps either),
/// `(or (<= v 10) (< v 5))` → `v ≤ 10` (the looser). For two UPPER bounds the tighter is the SMALLER `B`;
/// for two LOWER bounds the tighter is the LARGER `B`. `comparison_halfline` already handles either operand
/// side (a mirrored `(< c v)` normalizes to a lower bound on `v`) and only the four ordering ops (`Eq`/
/// `Compare` are not half-lines, so a `(= x 5)`/`(= x 6)` pair returns `None` here — never mis-subsumed).
/// The kept comparison still evaluates `v`, so no trap drops. OPPOSITE-direction pairs are `None` here (the
/// disjoint/covering + coincident-point folds handle those).
fn subsuming_comparison(
    db: &mut Db,
    lhs: StructId,
    rhs: StructId,
    is_and: bool,
) -> Option<StructId> {
    let (lv, l_upper, lb) = comparison_halfline(db, lhs)?;
    let (rv, r_upper, rb) = comparison_halfline(db, rhs)?;
    // Same operand, same direction (both upper or both lower) — an opposite-direction pair is a range, not
    // a subsumption (handled by `disjoint_or_covering`/`coincident_point_eq`).
    if l_upper != r_upper || !core_equiv(db, lv, rv) {
        return None;
    }
    // For UPPER bounds `v ≤ B`, the tighter (stronger) is the SMALLER B; for LOWER bounds `v ≥ B`, the
    // LARGER B. `and` keeps the stronger, `or` the weaker.
    let lhs_stronger = if l_upper { lb <= rb } else { lb >= rb };
    let keep_lhs = if is_and { lhs_stronger } else { !lhs_stronger };
    Some(if keep_lhs { lhs } else { rhs })
}

/// Normalize a `Core::Compare` on a runtime operand `v` against a constant into an INCLUSIVE half-line
/// bound on `v`, as `(v, is_upper, bound)` — `is_upper` means `v <= bound`, else `v >= bound`. Handles all
/// four ops on either operand side (`(< v c)` → `v <= c-1`; `(> v c)` → `v >= c+1`; `(< c v)` = `v > c` →
/// `v >= c+1`; etc). Bound arithmetic is `i128` so `c±1` never overflows at the i64 extremes. `None` when
/// the node is not a comparison of a runtime value against a constant. Used by `disjoint_or_covering`.
fn comparison_halfline(db: &mut Db, id: StructId) -> Option<(StructId, bool, i128)> {
    let Core::Compare { op, lhs, rhs } = core_of(db, id) else {
        return None;
    };
    let as_int = |db: &mut Db, id: StructId| match core_of(db, id) {
        Core::ConstInt(v) => v.to_i64().map(|v| v as i128),
        _ => None,
    };
    // `(op v c)` (v on the left) or `(op c v)` (v on the right, which flips the operator's sense).
    let (v, c, v_left) = match (as_int(db, rhs), as_int(db, lhs)) {
        (Some(c), _) => (lhs, c, true),
        (_, Some(c)) => (rhs, c, false),
        _ => return None,
    };
    // Effective operator with `v` on the left (`(op c v)` mirrors: `<`↔`>`, `<=`↔`>=`).
    let eff = if v_left {
        op
    } else {
        match op {
            Prim::Lt => Prim::Gt,
            Prim::Gt => Prim::Lt,
            Prim::Le => Prim::Ge,
            Prim::Ge => Prim::Le,
            other => other,
        }
    };
    // To an inclusive bound: `v < c` ⇒ `v <= c-1`; `v <= c` ⇒ `v <= c`; `v > c` ⇒ `v >= c+1`; `v >= c` ⇒
    // `v >= c`. (`=`/`Compare` are not half-lines.)
    match eff {
        Prim::Lt => Some((v, true, c - 1)),
        Prim::Le => Some((v, true, c)),
        Prim::Gt => Some((v, false, c + 1)),
        Prim::Ge => Some((v, false, c)),
        _ => None,
    }
}

/// For two comparisons forming OPPOSITE-direction half-lines on the SAME operand `v` — one `v <= U`, the
/// other `v >= L` — decide whether their `and`/`or` is a CONSTANT. `and` (intersection `L <= v <= U`) is
/// EMPTY iff `L > U` → `Some(false)`; `or` (union) COVERS every value iff the half-lines touch or overlap
/// (`L <= U + 1`) → `Some(true)`. `None` when the pair is not opposite half-lines on the same `v`, or the
/// intersection is non-empty (`and`) / the union has a gap (`or`) — those stay runtime. `(and (< x 5) (> x
/// 10))` → false; `(or (< x 5) (> x 3))` → true. All bound math is `i128` (no overflow at i64 extremes).
fn disjoint_or_covering(db: &mut Db, lhs: StructId, rhs: StructId, is_and: bool) -> Option<bool> {
    let (lv, l_upper, lb) = comparison_halfline(db, lhs)?;
    let (rv, r_upper, rb) = comparison_halfline(db, rhs)?;
    if l_upper == r_upper || !core_equiv(db, lv, rv) {
        return None; // need OPPOSITE directions on the SAME operand
    }
    // Order them: `u` = the upper bound `v <= U`, `l` = the lower bound `v >= L`.
    let (upper, lower) = if l_upper { (lb, rb) } else { (rb, lb) };
    if is_and {
        // Intersection `lower <= v <= upper` is empty iff `lower > upper`.
        (lower > upper).then_some(false)
    } else {
        // Union `v <= upper || v >= lower` covers all iff the pieces touch/overlap: `lower <= upper + 1`.
        (lower <= upper + 1).then_some(true)
    }
}

/// COINCIDENT-POINT COLLAPSE for `and`: `(and (>= v c) (<= v c))` (either operand order, `v` on either
/// side of each comparison) → `(= v c)`. Two INCLUSIVE opposite-direction bounds pinning `v` to a single
/// point ARE equality — `v >= c && v <= c ⟺ v == c` in any total order (sound for signed AND unsigned
/// integers alike; it is a pure order-theoretic fact, no sign assumption). Returns `(v, c_node)` to build
/// `Core::Compare { op: Eq, lhs: v, rhs: c_node }` — three ops (`ge` + `le` + `and`) collapse to one `eq`.
/// Restricted to the two INCLUSIVE ops (`>=`/`<=`) against the SAME i64 constant VALUE on both sides, and
/// REUSES an existing constant node (proven representable in `v`'s type — it typechecked against `v`), so
/// no constant is synthesized and no type-range guard is needed. The strictly-inclusive requirement also
/// keeps this distinct from the exclusive width-2 point `(and (> v (c-1)) (< v (c+1)))`, which would need a
/// synthesized `c` + a representability guard — deliberately left un-folded (conservative, no regression).
/// `None` unless the shape matches. DISCARDS the second comparison, so the caller gates on `is_trap_free`
/// for both operands (matches the sibling disjoint/covering fold); the kept `(= v c)` evaluates `v` once.
fn coincident_point_eq(db: &mut Db, lhs: StructId, rhs: StructId) -> Option<(StructId, StructId)> {
    // From a `Core::Compare` on a runtime `v` against an i64 constant, return `(v, c_node, c_value, eff)`
    // where `eff` is the operator NORMALIZED to `v` on the left (`(op c v)` mirrors `<`↔`>`, `<=`↔`>=`).
    let bound_of = |db: &mut Db, id: StructId| -> Option<(StructId, StructId, i64, Prim)> {
        let Core::Compare { op, lhs: a, rhs: b } = core_of(db, id) else {
            return None;
        };
        let as_int = |db: &mut Db, id: StructId| match core_of(db, id) {
            Core::ConstInt(v) => v.to_i64(),
            _ => None,
        };
        // `(op v c)` (v on the left) or `(op c v)` (v on the right, which mirrors the operator).
        match (as_int(db, b), as_int(db, a)) {
            (Some(c), _) => Some((a, b, c, op)),
            (_, Some(c)) => Some((
                b,
                a,
                c,
                match op {
                    Prim::Lt => Prim::Gt,
                    Prim::Gt => Prim::Lt,
                    Prim::Le => Prim::Ge,
                    Prim::Ge => Prim::Le,
                    other => other,
                },
            )),
            _ => None,
        }
    };
    let (lv, lc_node, lc, leff) = bound_of(db, lhs)?;
    let (rv, _rc_node, rc, reff) = bound_of(db, rhs)?;
    // Same runtime operand, same constant VALUE, and the two effective ops are exactly `>=` and `<=`
    // (opposite INCLUSIVE bounds). Either assignment (`>= , <=` or `<= , >=`).
    if lc != rc || !core_equiv(db, lv, rv) {
        return None;
    }
    let inclusive_opposite = matches!((leff, reff), (Prim::Ge, Prim::Le) | (Prim::Le, Prim::Ge));
    if !inclusive_opposite {
        return None;
    }
    trace!(target: "rcdzc::fold", "coincident-point collapse (and (>= v c) (<= v c)) → (= v c)");
    // Reuse `lv` as the operand and lhs's constant node as `c` — both proven trap-free by the caller's gate.
    Some((lv, lc_node))
}

/// For two comparisons where one is an EQUALITY `(= x c)` and the other an ORDERING comparison `(cmp x k)`
/// on the SAME `x` (both constants), return `(eq_node, range_node, sat)` — `sat` = whether `c` satisfies
/// the range predicate `(cmp c k)`, computed at compile time. The caller decides the fold: for `and`, `sat`
/// keeps the equality (range redundant) / `!sat` is `false` (contradiction); for `or`, `sat` keeps the
/// range (equality subsumed). `None` unless exactly one side is a scalar `Eq` and the other a scalar
/// ordering comparison (`< > <= >=`), both on the SAME `x` (`core_equiv`) against i64 constants.
fn eq_vs_range(db: &mut Db, lhs: StructId, rhs: StructId) -> Option<(StructId, StructId, bool)> {
    let as_const_i64 = |db: &mut Db, id: StructId| match core_of(db, id) {
        Core::ConstInt(v) => v.to_i64(),
        _ => None,
    };
    // Extract `(x, c)` from a `(= x c)` / `(= c x)` node (equality is symmetric).
    let eq_of = |db: &mut Db, id: StructId| -> Option<(StructId, i64)> {
        let Core::Compare {
            op: Prim::Eq,
            lhs: a,
            rhs: b,
        } = core_of(db, id)
        else {
            return None;
        };
        match (as_const_i64(db, b), as_const_i64(db, a)) {
            (Some(c), _) => Some((a, c)),
            (_, Some(c)) => Some((b, c)),
            _ => None,
        }
    };
    // Extract `(x, effective-op-with-x-on-left, k)` from an ordering comparison `(cmp x k)` / `(cmp k x)`.
    let range_of = |db: &mut Db, id: StructId| -> Option<(StructId, Prim, i64)> {
        let Core::Compare { op, lhs: a, rhs: b } = core_of(db, id) else {
            return None;
        };
        if !matches!(op, Prim::Lt | Prim::Gt | Prim::Le | Prim::Ge) {
            return None;
        }
        match (as_const_i64(db, b), as_const_i64(db, a)) {
            (Some(k), _) => Some((a, op, k)), // `(op x k)`
            (_, Some(k)) => Some((
                b,
                match op {
                    // `(op k x)` mirrors to x on the left.
                    Prim::Lt => Prim::Gt,
                    Prim::Gt => Prim::Lt,
                    Prim::Le => Prim::Ge,
                    Prim::Ge => Prim::Le,
                    other => other,
                },
                k,
            )),
            _ => None,
        }
    };
    // Try both assignments (eq on the left or right).
    let (eq_node, range_node, ex, c, rx, rop, k) =
        if let (Some((ex, c)), Some((rx, rop, k))) = (eq_of(db, lhs), range_of(db, rhs)) {
            (lhs, rhs, ex, c, rx, rop, k)
        } else if let (Some((ex, c)), Some((rx, rop, k))) = (eq_of(db, rhs), range_of(db, lhs)) {
            (rhs, lhs, ex, c, rx, rop, k)
        } else {
            return None;
        };
    if !core_equiv(db, ex, rx) {
        return None; // same `x`
    }
    // Does the equality's value `c` satisfy the range predicate `(rop c k)`?
    let sat = compare_ord(rop, c.cmp(&k));
    Some((eq_node, range_node, sat))
}

/// The NESTED-BITWISE COLLAPSE for an outer TOTAL, ASSOCIATIVE bitwise op (`&`/`|`/`^`) whose operands
/// are `(lhs, rhs)` (cores `lc`/`rc`): when one operand is `(OP v C1)` — the SAME op — and the OTHER is a
/// constant `C2`, returns `(OP v (C1 ⊙ C2))` where `⊙` is that op's constant fold — one op instead of
/// two. `(& (& v C1) C2)` → `(& v (C1&C2))`, `(| (| v C1) C2)` → `(| v (C1|C2))`, `(^ (^ v C1) C2)` →
/// `(^ v (C1^C2))`. `None` when neither shape matches (so the caller's later folds still fire). All three
/// ops are TOTAL (never trap) and ASSOCIATIVE, so no trap is dropped and the value is identical; `v` stays
/// the operand (its own traps preserved). The folded constant is a fresh `Leaf::Int` atom, lowered lazily
/// to `Core::ConstInt` and grounded to the op width at selection. (NOT for `+`/`-`/`*`/`<<` — those are
/// CHECKED, so `(v OP C1) OP C2` traps differently from `v OP (C1⊙C2)`.)
fn nested_bitwise_collapse(
    db: &mut Db,
    op: Prim,
    lhs: StructId,
    lc: &Core,
    rhs: StructId,
    rc: &Core,
) -> Option<Core> {
    if !matches!(op, Prim::BitAnd | Prim::BitOr | Prim::BitXor) {
        return None;
    }
    let apply = |a: i64, b: i64| match op {
        Prim::BitAnd => a & b,
        Prim::BitOr => a | b,
        _ => a ^ b, // BitXor
    };
    // The `(v, C1)` of an inner `(OP v C1)` node with the SAME op, C1 a constant on either side.
    let nested_op_const = |db: &mut Db, inner: StructId| -> Option<(StructId, i64)> {
        let Core::Arith {
            op: inner_op,
            lhs: il,
            rhs: ir,
        } = core_of(db, inner)
        else {
            return None;
        };
        if inner_op != op {
            return None;
        }
        match (core_of(db, il), core_of(db, ir)) {
            (Core::ConstInt(c), _) => c.to_i64().map(|c| (ir, c)),
            (_, Core::ConstInt(c)) => c.to_i64().map(|c| (il, c)),
            _ => None,
        }
    };
    let combine = |db: &mut Db, inner: StructId, outer_c: i64| -> Option<Core> {
        let (v, inner_c) = nested_op_const(db, inner)?;
        let folded = apply(inner_c, outer_c);
        let fc = db.push_atom(crate::ast::Leaf::Int {
            value: IntValue::from_i64(folded),
            radix: crate::ast::Radix::Dec,
        });
        trace!(target: "rcdzc::fold", ?op, inner_c, outer_c, folded, "nested-bitwise collapse (OP (OP v C1) C2) → (OP v (C1⊙C2))");
        Some(Core::Arith {
            op,
            lhs: v,
            rhs: fc,
        })
    };
    // inner on the LEFT, constant C2 on the RIGHT.
    if let Core::ConstInt(c2) = rc
        && let Some(c2) = c2.to_i64()
        && let Some(folded) = combine(db, lhs, c2)
    {
        return Some(folded);
    }
    // constant C2 on the LEFT, inner on the RIGHT.
    if let Core::ConstInt(c2) = lc
        && let Some(c2) = c2.to_i64()
        && let Some(folded) = combine(db, rhs, c2)
    {
        return Some(folded);
    }
    None
}

/// XOR CANCELLATION for an outer `(^ lhs rhs)`: when one operand is `(^ v w)` and the OTHER is
/// `core_equiv` to `w`, the two XORs by `w` cancel — `(v ^ w) ^ w == v ^ (w ^ w) == v ^ 0 == v` (XOR is
/// associative/commutative and self-inverse). Returns `v`. Covers a CONSTANT `w` (`(^ (^ x 5) 5)` → x —
/// which `nested_bitwise_collapse` would leave as a residual `(^ x 0)`) AND a RUNTIME `w` (`(^ (^ x y) y)`
/// → x, the involution `nested_bitwise_collapse` cannot see). Both operand orders of the outer `^`, and
/// `w` on either side of the inner `^`, are tried. The result is `v` (its own traps preserved); `w` is
/// DISCARDED, so the caller gates on `is_trap_free(w)` — a trapping `w` (`(^ (^ v (/ a b)) (/ a b))` at
/// b==0) must still trap. Returns `(v, w)` so the caller can trap-check `w`.
fn xor_cancels(db: &mut Db, lhs: StructId, rhs: StructId) -> Option<(StructId, StructId)> {
    // Try: `lhs` is the inner `(^ v w)`, `rhs` is the matching `w`.
    let check = |db: &mut Db, inner: StructId, outer_w: StructId| -> Option<(StructId, StructId)> {
        let Core::Arith {
            op: Prim::BitXor,
            lhs: il,
            rhs: ir,
        } = core_of(db, inner)
        else {
            return None;
        };
        // The outer operand `outer_w` must equal ONE side of the inner XOR; the OTHER side is `v`.
        if core_equiv(db, ir, outer_w) {
            Some((il, ir)) // (v, w)
        } else if core_equiv(db, il, outer_w) {
            Some((ir, il)) // (v, w) — inner XOR is commutative
        } else {
            None
        }
    };
    check(db, lhs, rhs).or_else(|| check(db, rhs, lhs))
}

/// IDEMPOTENT-BITWISE COLLAPSE for an outer `(op lhs rhs)` where `op` is `&` or `|`: when one operand is
/// an inner `(op v w)` (the SAME op) and the OTHER outer operand is `core_equiv` to `w`, return the inner
/// node — `(op (op v w) w) == (op v w)` because `&`/`|` are idempotent (`w op w == w`) and associative.
/// Covers a RUNTIME `w` the constant-folding `nested_bitwise_collapse` cannot (`(| (| x y) y)` → `(| x
/// y)`). Both outer orders and `w` on either side of the inner op are tried. The inner `(op v w)` node is
/// RETAINED, so both `v` and `w` are still evaluated — no trap is dropped (no `is_trap_free` needed).
fn idempotent_bitwise_collapse(
    db: &mut Db,
    op: Prim,
    lhs: StructId,
    rhs: StructId,
) -> Option<StructId> {
    // `inner` must be `(op v w)` with the SAME op; `outer_w` must match one of its operands.
    let check = |db: &mut Db, inner: StructId, outer_w: StructId| -> Option<StructId> {
        let Core::Arith {
            op: inner_op,
            lhs: il,
            rhs: ir,
        } = core_of(db, inner)
        else {
            return None;
        };
        if inner_op != op {
            return None;
        }
        // The outer operand equals ONE side of the inner op → re-applying `op` by it is a no-op.
        if core_equiv(db, il, outer_w) || core_equiv(db, ir, outer_w) {
            Some(inner)
        } else {
            None
        }
    };
    check(db, lhs, rhs).or_else(|| check(db, rhs, lhs))
}

/// ABSORPTION LAW for an outer `(op lhs rhs)` where `op` is `&` or `|`: when one operand is an inner op of
/// the DUAL kind (`|` under `&`, `&` under `|`) that contains `x`, and the OTHER outer operand IS `x`
/// (`core_equiv`), the whole expression absorbs to `x` — `x & (x | y) == x`, `x | (x & y) == x`. Returns
/// `(x, y)` where `y` is the inner op's OTHER operand (the one absorbed away), so the caller can trap-check
/// `y` (it is DISCARDED). Both outer orders and both inner-operand positions are tried. `None` when the
/// shape does not match.
fn absorption_operand(
    db: &mut Db,
    op: Prim,
    lhs: StructId,
    rhs: StructId,
) -> Option<(StructId, StructId)> {
    let dual = match op {
        Prim::BitAnd => Prim::BitOr,
        Prim::BitOr => Prim::BitAnd,
        _ => return None,
    };
    // `inner` must be `(dual p q)`; `outer_x` must equal one of `p`/`q` (that side is `x`, the other `y`).
    let check = |db: &mut Db, inner: StructId, outer_x: StructId| -> Option<(StructId, StructId)> {
        let Core::Arith {
            op: inner_op,
            lhs: ip,
            rhs: iq,
        } = core_of(db, inner)
        else {
            return None;
        };
        if inner_op != dual {
            return None;
        }
        if core_equiv(db, ip, outer_x) {
            Some((outer_x, iq)) // (x, y) — x matched ip, y is iq
        } else if core_equiv(db, iq, outer_x) {
            Some((outer_x, ip)) // (x, y) — x matched iq, y is ip
        } else {
            None
        }
    };
    check(db, lhs, rhs).or_else(|| check(db, rhs, lhs))
}

/// The NESTED SHIFT COLLAPSE for `(SH lhs rhs)` where `SH` is `Shr` OR `Shl`: when `lhs` is itself
/// `(SH v A)` — the SAME shift op — with a constant inner count A, and the outer count `rc` is a constant
/// B with `A + B < width`, returns `(SH v (A+B))` — one shift instead of two. `None` otherwise (so later
/// folds fire). For `>>`: the shift is total; inner and outer are the same kind (`>>ₛ`/`>>ᵤ`) on the
/// same-typed value, so composing drops the same low `A+B` bits as one shift by `A+B`. For `<<`: the
/// shift is CHECKED (exact `·2^count`, traps on N-bit overflow) but still collapses TRAP-IDENTICALLY —
/// magnitude is monotonic in the count, so `(v<<A)<<B` and `v<<(A+B)` overflow on exactly the same inputs
/// (inner overflow ⟹ combined; combined ⟹ the double's outer step) and agree on value otherwise. The
/// `A + B < width` bound is essential for BOTH: a combined count `≥ width` would be masked mod width by
/// the machine shift (`>>`) / must trap as an out-of-range count (`<<`), disagreeing with the double
/// shift. `v` keeps its traps (it stays the operand). The combined count `A+B` is a fresh `Leaf::Int`.
fn nested_shift_combine(db: &mut Db, op: Prim, lhs: StructId, rc: &Core) -> Option<Core> {
    // Only the two shift ops, and the inner must be the SAME op (a `<<` inside a `>>` composes bits
    // differently and does not collapse).
    if !matches!(op, Prim::Shr | Prim::Shl) {
        return None;
    }
    // Outer count B must be a constant ≥ 1 (0 is handled by the `SH 0` identity).
    let Core::ConstInt(b) = rc else { return None };
    let b = b.to_i64().filter(|&b| b >= 1)?;
    // `lhs` must be an inner shift by the SAME op with a constant count A ≥ 1.
    let Core::Arith {
        op: inner_op,
        lhs: v,
        rhs: inner_count,
    } = core_of(db, lhs)
    else {
        return None;
    };
    if inner_op != op {
        return None;
    }
    let Core::ConstInt(a) = core_of(db, inner_count) else {
        return None;
    };
    let a = a.to_i64().filter(|&a| a >= 1)?;
    // Sound ONLY when the combined count stays in range for the SHIFTED VALUE's width (both shifts share
    // it — binary-op unification). A `width` of 0 (deferred) fails the guard, so no fold.
    let width = shift_width(db, v) as i64;
    if width == 0 || a + b >= width {
        return None;
    }
    let fc = db.push_atom(crate::ast::Leaf::Int {
        value: IntValue::from_i64(a + b),
        radix: crate::ast::Radix::Dec,
    });
    trace!(target: "rcdzc::fold", ?op, a, b, sum = a + b, "nested shift collapse (SH (SH v A) B) → (SH v (A+B))");
    Some(Core::Arith {
        op,
        lhs: v,
        rhs: fc,
    })
}

/// OR-THEN-MASK ABSORPTION: for an outer `(& inner C2)` whose `inner` is `(| v C1)` (C1 a constant on
/// either side), return `C2` when `C2`'s set bits are a SUBSET of `C1`'s (`C2 & C1 == C2`) — because
/// `(v | C1) & C2` forces every bit of `C2` to 1 (they are all in `C1`, which the OR sets) and clears the
/// rest, so the result is exactly `C2`, regardless of `v`. `(& (| x 15) 15)` → 15, `(& (| x 255) 15)` →
/// 15. Returns the CONSTANT `C2` occurrence (the outer mask, `c2_occ`) to reuse as the folded value.
/// `None` when the shape does not match or `C2 ⊄ C1`. The fold DISCARDS `v`, so the caller gates it on
/// `is_trap_free(v)` — the returned `Some` reports `v` so the caller can check it.
fn or_then_mask_absorbs(db: &mut Db, inner: StructId, c2: i64) -> Option<StructId> {
    let Core::Arith {
        op: Prim::BitOr,
        lhs: il,
        rhs: ir,
    } = core_of(db, inner)
    else {
        return None;
    };
    // The inner OR's constant C1 (on either side); the other operand is `v`.
    let (v, c1) = match (core_of(db, il), core_of(db, ir)) {
        (Core::ConstInt(c), _) => (ir, c.to_i64()?),
        (_, Core::ConstInt(c)) => (il, c.to_i64()?),
        _ => return None,
    };
    // `C2 ⊆ C1` — every bit the outer mask keeps is one the inner OR already set to 1.
    if (c2 & c1) == c2 { Some(v) } else { None }
}

/// Whether masking the value at `val` with the constant `mask_core` is a NO-OP — i.e. `val & M == val`.
/// True iff `val`'s solved type is a resolved UNSIGNED integer of width `N` (`Sign::Fixed(false)` +
/// `Width::Fixed(N)`, `N < 64`) and the mask's low `N` bits are ALL set (`M & (2^N − 1) == 2^N − 1`). An
/// unsigned width-N value lives in `[0, 2^N)`, so a mask covering its whole range clears nothing. NOT
/// applied to signed types (the slot's high bits are sign extension a mask would wrongly clear) nor to
/// a 64-bit width (whose full mask `2^64−1` is not i64-representable here — and `& allbits` at 64 is a
/// separate case the `x & x` fold does not cover; skipped for simplicity, low value).
fn is_full_mask_for(db: &mut Db, val: StructId, mask_core: &Core) -> bool {
    let Core::ConstInt(m) = mask_core else {
        return false;
    };
    let Some(m) = m.to_i64() else {
        return false;
    };
    let Some(bits) = unsigned_value_bits(db, val) else {
        return false; // not a provably-nonnegative value with a known ≤63-bit range.
    };
    // `2^bits − 1` — all bits the value can possibly set. `bits` is `1..=63`; at `bits == 63` the shift
    // `1i64 << 63` is `i64::MIN`, so `− 1` would OVERFLOW in a checked build (a latent panic) — that case
    // is exactly `i64::MAX` (all 63 low bits, the whole nonneg i64 range), so special-case it.
    let low = if bits >= 63 {
        i64::MAX
    } else {
        (1i64 << bits) - 1
    };
    (m & low) == low
}

/// For a `BitAnd` at emit time, whether the constant mask on ONE side covers the WHOLE provable range of
/// the value on the other side — so `v & M == v` and the `&` is redundant. Returns the VALUE operand to
/// emit alone (`Some(v)`), or `None` when neither side is such a redundant mask. This is the EMIT-TIME
/// sibling of the `is_full_mask_for` lower fold: identical soundness (a nonneg `v ∈ [0, 2^B)` whose bits
/// `M` all covers), but it consults `value_range` HERE — where the flow-refinement stack is populated — so
/// it fires on a refined value the lower fold could not see (`(if (and (>= x 0) (< x 256)) (& x 255) …)`:
/// under the branch `x ∈ [0,255]`, `x & 255 == x`). Both operand orders are tried. The `&` is TOTAL, so
/// eliding it drops no trap; returning the value operand preserves its own evaluation (and any trap in it).
pub(crate) fn redundant_and_mask_value(
    db: &mut Db,
    lhs: StructId,
    rhs: StructId,
) -> Option<StructId> {
    let rc = core_of(db, rhs);
    if is_full_mask_for(db, lhs, &rc) {
        return Some(lhs); // `(& v M)` with M covering v's range → v
    }
    let lc = core_of(db, lhs);
    if is_full_mask_for(db, rhs, &lc) {
        return Some(rhs); // `(& M v)` → v
    }
    None
}

/// For a `BitOr` at emit time, whether the constant `M` on ONE side covers the WHOLE provable range of the
/// value `v` on the other side — so `v | M == M` (OR-SATURATION) and the `|` is redundant. Returns the
/// CONSTANT operand (`Some(M_occ)`) to emit alone, or `None`. The emit-time sibling of the `BitOr`
/// OR-saturation lower fold, firing on a flow-refined `v` the lower fold cannot see (`(if (and (>= x 0)
/// (< x 256)) (| x 255) …)` → `x | 255 == 255` under `x ∈ [0,255]`). DISCARDS `v` (the result is the
/// constant M), so the caller must first confirm `v` is TRAP-FREE — a trapping `v` must still trap. Both
/// operand orders tried; returns whichever operand is the covering constant.
pub(crate) fn redundant_or_mask_const(
    db: &mut Db,
    lhs: StructId,
    rhs: StructId,
) -> Option<StructId> {
    // `(| v M)` — v on the left, constant M on the right covering v's range → M (rhs).
    let rc = core_of(db, rhs);
    if is_full_mask_for(db, lhs, &rc) && is_trap_free(db, lhs) {
        return Some(rhs);
    }
    // `(| M v)` — constant M on the left → M (lhs).
    let lc = core_of(db, lhs);
    if is_full_mask_for(db, rhs, &lc) && is_trap_free(db, rhs) {
        return Some(lhs);
    }
    None
}

/// Whether the dividend `val` is provably in `[0, divisor − 1]` for a positive `divisor` — so a truncating
/// `val / divisor` is `0` and `val % divisor` is `val` (the divisor is too large to divide `val` even
/// once). True iff `value_range(val)` is a NONNEGATIVE closed interval `[lo, hi]` with `lo >= 0` and
/// `hi < divisor`. Restricted to a nonnegative dividend: for `0 <= val < divisor`, both the mathematical
/// and the truncate-toward-zero results are exact (`val / divisor = 0`, `val % divisor = val`); a negative
/// dividend is excluded (its `value_range` lo is `< 0`, failing the check). `None`/unbounded range → false.
fn dividend_below_divisor(db: &mut Db, val: StructId, divisor: i64) -> bool {
    matches!(value_range(db, val), Some((lo, Some(hi))) if lo >= 0 && hi < divisor)
}

/// An upper bound (in `1..=63`) on the number of LOW bits a runtime value can set — i.e. the value is
/// provably in `[0, 2^B)`. Derived from `value_range`: a value whose range is `[0, hi]` (nonnegative,
/// `hi` a nonneg i64) fits `bits(hi)` bits. `None` when the value is not provably nonnegative or has no
/// i64 upper bound. Drives the mask-elision (`& fullmask`) and shift-out (`>>ᵤ` all bits) folds.
fn unsigned_value_bits(db: &mut Db, val: StructId) -> Option<u32> {
    match value_range(db, val) {
        // A nonnegative closed range `[0, hi]` → the significant-bit count of `hi` (≥ 1). `hi` is a
        // nonnegative i64, so `bits(hi) ∈ 1..=63`.
        Some((0, Some(hi))) if hi >= 0 => Some((64 - (hi as u64).leading_zeros()).max(1)),
        _ => None,
    }
}

/// The language WIDTH `N` of `val`'s resolved integer type — the range a shift COUNT is guarded to
/// `[0, N)`. Used by the shift-out-to-zero fold to confirm the constant count is IN-RANGE (an
/// out-of-range shift TRAPS, so it must NOT be folded to 0). `None` if the type is not a resolved
/// integer (a deferred width would guess the guard bound).
fn shift_width(db: &mut Db, val: StructId) -> u32 {
    match crate::infer::type_of(db, val) {
        crate::ty::Ty::Int(it) => match it.width {
            crate::ty::Width::Fixed(n) => n,
            _ => 0, // deferred/var — treat as 0 so the `k < width` guard fails (no fold).
        },
        _ => 0,
    }
}

/// Whether the node at `id` lowers to a core that CANNOT TRAP at run time — so discarding it (an
/// annihilator identity like `x * 0 → 0`) loses no defined trap. CONSERVATIVE: only a value with no
/// checked operation anywhere inside it. Trap-free = a leaf (constant/param/local/unit), a wrap
/// (total), or a bitwise op / conversion / projection over trap-free operands. NOT trap-free = `+`/
/// `-`/`*`/`<<`/`>>` (overflow/count guards), `/`/`%` (÷0, MIN/-1), a call (its body may trap), an
/// `if`/`match` (a branch may trap), a sum/tuple/record construct (may allocate/box — treated as
/// possibly-effecting here). Reads the operand's already-lowered core recursively.
pub(crate) fn is_trap_free(db: &mut Db, id: StructId) -> bool {
    match core_of(db, id) {
        Core::ConstInt(_)
        | Core::ConstBool(_)
        | Core::ConstStr(_)
        | Core::ConstBytes(_)
        | Core::ConstChar(_)
        | Core::ConstFloat(_)
        | Core::ConstFloatNan
        | Core::Unit
        // A PARAMETER is caller-forced (its argument was already evaluated before the body runs), so a param
        // reference can never itself be an un-forced trap — always trap-free.
        | Core::Param { .. } => true,
        // A LOCAL REFERENCE reads an already-bound slot, so the READ is pure — but `is_trap_free` is used by
        // DISCARDING folds (`x OP x → const`, `x * 0 → 0`, `(if c a a) → a`, the absorbing/complement laws …)
        // to decide "safe to DROP this operand without eliding a trap". A `let` binding is LAZY (forced on
        // use), so DISCARDING the reference can drop the LAST forcing of a binding whose INIT traps → eliding
        // an OBSERVABLE trap (cdz-smith L2 differential: `(let ((v0 (r 2))) (< v0 v0))` folded to false,
        // dropping v0's trapping force; the Lean oracle correctly traps). So a reference is trap-free-TO-DISCARD
        // iff its BOUND VALUE is: follow the ref chain (`peel_ref_annot`) to the underlying init and check THAT.
        // (Consistent with the trap-observability precedent #4417 — a real trap is never silently elided.) An
        // unresolvable ref keeps the prior `true` (regression-safe — this only tightens the resolvable cases,
        // which are exactly the unsound discards).
        Core::LocalRef { .. } => {
            let init = peel_ref_annot(db, id);
            init == id || is_trap_free(db, init)
        }
        // PURE VALUE CONSTRUCTORS never trap in themselves — building a record/tuple/list/sum node is total;
        // only a trapping SUB-expression inside makes the whole thing trap. So a construction is trap-free
        // iff every field/element/payload it holds is. (This is what lets a discarding fold over a compound
        // fire — e.g. the `List.len` constant-arity fold drops the element VALUES, sound only when every
        // element construction is trap-free; a `(list _ (Rational.of 3 d))` with a runtime denominator is
        // NOT trap-free, so the fold declines and the runtime op preserves the element's trap.)
        Core::Record { fields } => fields.values().copied().all(|v| is_trap_free(db, v)),
        Core::Tuple { elems } | Core::ListNew { elems } => {
            elems.iter().copied().all(|e| is_trap_free(db, e))
        }
        Core::SumNew { payloads, .. } => payloads.iter().all(|&p| is_trap_free(db, p)),
        // Bitwise ops are total; a comparison never traps — trap-free if their operands are. The WRAPPING
        // arithmetic ops (`wrapping-add`/`wrapping-sub`/`wrapping-mul`) are ALSO total: they emit the raw
        // machine `add`/`sub`/`mul` with NO overflow guard (wasm's op wraps modulo the slot — that is their
        // whole point vs checked `+`/`-`/`*`), so they never trap and are trap-free when their operands
        // are. (Checked `Add`/`Sub`/`Mul` — with an overflow guard — stay in the possibly-trapping `_` arm below.)
        Core::Arith {
            op:
                Prim::BitAnd
                | Prim::BitOr
                | Prim::BitXor
                | Prim::WrappingAdd
                | Prim::WrappingSub
                | Prim::WrappingMul,
            lhs,
            rhs,
        }
        | Core::Compare { lhs, rhs, .. } => is_trap_free(db, lhs) && is_trap_free(db, rhs),
        // Boolean negation `not` is a single `i32.eqz` — total (never traps) — so trap-free if its operand
        // is. (Lets `(not a)` participate in a discarding fold, e.g. the boolean complement law
        // `(and (not a) a)` → false, whose `is_trap_free(lhs)` guard sees the `(not a)` lhs.)
        Core::Not { operand } => is_trap_free(db, operand),
        // `wrap` is total (never traps) — trap-free if its operand is.
        Core::Convert {
            op: Prim::Wrap,
            operand,
        } => is_trap_free(db, operand),
        // A COLLECTION COUNT (`List.len`/`Bytes.len`/`Map.size`/`Set.len`) is a TOTAL O(1) borrowing read —
        // it never traps — so it is trap-free when its collection operand is (a param/kept-local handle is;
        // a count of a trapping construction stays tied to that trap). This lets a length feed a discarding
        // fold (`(>= (List.len xs) 0)` → true drops the length) with its `[0, 2^32-1]` range from
        // `value_range`. The operand is the container handle for each.
        Core::ListLen { operand } | Core::BytesLen { operand } | Core::StrScalarLen { operand } => {
            is_trap_free(db, operand)
        }
        Core::MapSize { map } => is_trap_free(db, map),
        Core::SetLen { set } => is_trap_free(db, set),
        // A TUPLE/RECORD PROJECTION is a total borrowing read: its `index` is WITHIN the operand's static
        // arity (`type_errors` rejects an out-of-arity index at compile time — never a runtime OOB trap;
        // an arity-≥1 compound is never the immediate that `arr-get` would OOB on), so `arr-get(compound,
        // const-index)` cannot trap. Trap-free when the compound operand is. This lets an invariant field
        // read (`(. p 0)` over a pass-through tuple `p`) hoist out of a loop and a dead projection be
        // dropped by a discarding fold. (A `SumPayload` `Elem` step is EXCLUDED — reading a variant's
        // payload before its discriminant is checked CAN mismatch, so it stays possibly-trapping in `_`.)
        Core::Proj { operand, .. } => is_trap_free(db, operand),
        // A RIGHT SHIFT by a CONSTANT in-range count (`0 <= k < width`) never traps: `>>` cannot overflow
        // (its magnitude only shrinks), and a valid constant count trips no count-guard. So it is trap-free
        // when its value operand is. (A `<<` is EXCLUDED — it is exact `·2^k` and can overflow the type, so
        // it is genuinely trapping even with a valid count. A RUNTIME count is also excluded — an
        // out-of-range count traps.) This lets a `(>> x k)` feed a discarding fold: `(< (>>ᵤ x 60) 20)` on
        // a UInt64 (range `[0,15]`) folds to `true` without keeping a bogus "shift might trap" compare.
        Core::Arith {
            op: Prim::Shr,
            lhs,
            rhs,
        } if matches!(core_of(db, rhs), Core::ConstInt(k)
                if k.to_i64().is_some_and(|k| k >= 0 && (k as u32) < shift_width(db, lhs))) =>
        {
            is_trap_free(db, lhs)
        }
        // A `/` or `%` by a CONSTANT divisor `C ∉ {0, -1}` never traps: `C != 0` rules out ÷0, and `C != -1`
        // rules out the sole signed-division overflow `MIN / -1`. So it is trap-free when its dividend is.
        // (`C == 0` is a constant-trap poison in `lower` before here; `C == -1` keeps the guard. A RUNTIME
        // divisor is excluded — it could be 0 or -1.) Lets `(< (% (& x 255) 10) 10)` fold to `true`.
        Core::Arith {
            op: Prim::Div | Prim::Rem,
            lhs,
            rhs,
        } if matches!(core_of(db, rhs), Core::ConstInt(c)
                if matches!(c.to_i64(), Some(v) if v != 0 && v != -1)) =>
        {
            is_trap_free(db, lhs)
        }
        // Everything else — checked arithmetic (+/-/*), a LEFT shift, a runtime-count/-divisor shift or
        // div/rem, calls, control flow, heap constructs, poison — is conservatively treated as possibly-
        // trapping.
        _ => false,
    }
}

/// Whether the core at `id` CONTAINS a runtime call (`Core::Call`, `CallClosure`, or `HostCall`) anywhere
/// in the positions the mutual-/self-recursion loop transform threads a TAIL call through — the node
/// itself, an `if`'s branches, a `let`'s body, a `match`'s arms. Used to VETO the `(if c a false)`→`(and c
/// a)` rewrite when the branch that would become the connective's guarded `rhs` holds a tail call: the loop
/// transform (`body_has_member_tail_call`) only follows `if`/`let`/`match` tail positions, NOT `and`/`or`,
/// so burying a tail-recursive call inside a connective would defeat tail-loop conversion (a far bigger win
/// than a branchless boolean). Conservative — descends only tail positions, matching the transform's reach;
/// a call in a non-tail operand is not a tail edge and would not be lost, but treating the whole branch as
/// call-bearing here is safe (it only forgoes the rewrite). NOT the same as `!is_trap_free`: a checked-arith
/// boolean branch (`(> (+ x 1) 5)`) is call-free, so the rewrite still fires and stays sound (its trap is
/// shielded in the connective's guarded rhs exactly as in the `if`'s branch).
fn tail_positions_have_call(db: &mut Db, id: StructId) -> bool {
    match core_of(db, id) {
        Core::Call { .. } | Core::CallClosure { .. } | Core::HostCall { .. } => true,
        Core::If { then_, else_, .. } => {
            tail_positions_have_call(db, then_) || tail_positions_have_call(db, else_)
        }
        Core::Let { body, .. } => tail_positions_have_call(db, body),
        Core::Match { arms, .. } => arms.iter().any(|a| tail_positions_have_call(db, a.body)),
        _ => false,
    }
}

/// Whether the nodes at `a` and `b` lower to the STRUCTURALLY IDENTICAL core — the basis for folding an
/// `if` whose two branches are the same (`(if c x x)` → `x`). CONSERVATIVE: matches only PURE
/// deterministic scalar cores (const / param / local-ref leaves; arithmetic / comparison / conversion /
/// projection over recursively-equal operands), so any other core (a call, a nested `if`, a heap
/// construct) compares unequal and the `if` is left intact. Every matched kind is a value that reads the
/// same whichever branch produces it, so collapsing the two branches to one is behavior-preserving.
/// (This is the `lower`-column twin of `select::core_eq`, kept here because `lower` owns the core.)
fn core_equiv(db: &mut Db, a: StructId, b: StructId) -> bool {
    if a == b {
        return true;
    }
    match (core_of(db, a), core_of(db, b)) {
        (Core::ConstInt(x), Core::ConstInt(y)) => x.eq_value(&y),
        (Core::ConstBool(x), Core::ConstBool(y)) => x == y,
        (Core::Unit, Core::Unit) => true,
        (Core::Param { binder: x }, Core::Param { binder: y }) => x == y,
        (Core::LocalRef { binder: x }, Core::LocalRef { binder: y }) => x == y,
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
        ) => {
            // Base structural match: same operator, operands equal position-wise.
            let positional = ox == oy && core_equiv(db, lx, ly) && core_equiv(db, rx, ry);
            // COMMUTATIVITY of EQUALITY: `(= a b)` and `(= b a)` denote the identical boolean (equality is
            // symmetric), so accept the SWAPPED operand match too. Only `Eq` — `<`/`>`/`<=`/`>=` flip
            // direction when swapped, and this arm is shared with `Core::Arith` whose ops are never `Eq`, so
            // `ox == Eq` fires ONLY for a comparison. Guarded on both operands trap-free so the swap changes
            // no observable evaluation ORDER (a trapping operand's position could decide which trap fires
            // first; a pure operand's cannot). Lets `(and (= a b) (= b a))` fold to one `(= a b)` via the
            // idempotence path, which keys on `core_equiv`.
            positional
                || (ox == oy
                    && matches!(ox, Prim::Eq)
                    && is_trap_free(db, lx)
                    && is_trap_free(db, rx)
                    && core_equiv(db, lx, ry)
                    && core_equiv(db, rx, ly))
        }
        (
            Core::Convert {
                op: ox,
                operand: px,
            },
            Core::Convert {
                op: oy,
                operand: py,
            },
        ) => ox == oy && core_equiv(db, px, py),
        (
            Core::Proj {
                operand: px,
                index: ix,
            },
            Core::Proj {
                operand: py,
                index: iy,
            },
        ) => ix == iy && core_equiv(db, px, py),
        _ => false,
    }
}

/// Hoist a COMMON CONSTRUCTOR out of both `if` arms: when both branches build the SAME constructor —
/// the same `SumNew` discriminant + payload arity, a same-arity `Tuple`, a same-length `List`, or a
/// `Record` with the SAME KEY SET — the heap build is DUPLICATED across the two branches, differing only
/// in the payload/element/field occurrences. Build the constructor ONCE and push each DIFFERING position
/// down into its own
/// `(if c pᵢ qᵢ)`; a position that is `core_equiv` across the arms is shared directly.
/// `(if c (Some a) (Some b))` → `(Some (if c a b))`, `(if c (tuple a k) (tuple b k))` →
/// `(tuple (if c a b) k)`, and `(if c (record (x a) (y k)) (record (x b) (y k)))` →
/// `(record (x (if c a b)) (y k))` — ONE alloc + one set of field stores emitted instead of two
/// duplicated build sequences (a module-size win; the runtime alloc count is already one either way
/// since an `if` takes exactly one arm).
///
/// SOUND: exactly one branch's payloads ever MATERIALIZE either way — a differing position stays under
/// an `if`, so the untaken arm's payload is never evaluated or consumed and the Perceus consume-once
/// discipline is unchanged (a heap payload is dup'd/dropped by the inner `if` exactly as the original
/// arm's build did). A SHARED (identical) position is a PURE scalar — `core_equiv` matches only
/// const/param/local/arith/compare/convert/proj — so evaluating it once unconditionally reproduces the
/// always-taken arm's single evaluation (including any arith trap, which the taken arm also incurred).
/// `cond` is evaluated ONCE PER DIFFERING position: exactly one differing position is one evaluation,
/// matching the original `if`, so that COUNT case is sound; zero or ≥2 differing positions change the
/// count, so require a TRAP-FREE `cond` (it re-reads identically with no effect or trap to drop or
/// duplicate). But a trapping `cond` needs an ORDER check too: the original `if` evaluates `cond`
/// FIRST — before any payload — whereas a SHARED payload BEFORE the differing position is built OUTSIDE
/// the per-position `if` and so evaluates BEFORE `cond`. Since `core_equiv` admits trapping arith (a
/// `/` by a runtime divisor), a preceding shared payload that can trap would PREEMPT `cond`'s trap and
/// the hoist would observe the WRONG trap; so a trapping `cond` additionally requires every shared
/// payload preceding the differing position to be trap-free. Returns `None` (keep the `if`) when the
/// arms are not the same constructor, disagree in shape/key-set, or either cond guard fails. A poison
/// arm has a non-constructor core, so it never matches and the `if`'s existing poison handling stands.
fn hoist_common_ctor(
    db: &mut Db,
    cond: StructId,
    then_: StructId,
    else_: StructId,
) -> Option<Core> {
    enum Shape {
        Sum(u32),
        Tuple,
        // A record's fields in a fixed KEY ORDER (the `BTreeMap`'s sorted keys), paired with the aligned
        // then/else value occurrences below; rebuilt into a `BTreeMap` after the per-position hoist.
        Record(Vec<crate::resolved::Symbol>),
        // A same-length list — positional like a tuple, rebuilt as a `ListNew` of the hoisted elements.
        List,
    }
    // Align each arm's positions into `(then_value, else_value)` pairs and record the reconstruction
    // shape. For keyed records the two arms must carry the IDENTICAL key set (a differing key set is a
    // different value, not a hoistable common constructor); the paired values are read in sorted-key
    // order so the rebuilt map re-pairs them by that same order.
    let (shape, pairs): (Shape, Vec<(StructId, StructId)>) =
        match (core_of(db, then_), core_of(db, else_)) {
            (
                Core::SumNew {
                    disc: dt,
                    payloads: pt,
                },
                Core::SumNew {
                    disc: de,
                    payloads: pe,
                },
            ) if dt == de && pt.len() == pe.len() => (
                Shape::Sum(dt),
                pt.iter().copied().zip(pe.iter().copied()).collect(),
            ),
            (Core::Tuple { elems: et }, Core::Tuple { elems: ee }) if et.len() == ee.len() => (
                Shape::Tuple,
                et.iter().copied().zip(ee.iter().copied()).collect(),
            ),
            // A LIST is positional like a tuple, but a list's LENGTH is part of its value (two lists of
            // different lengths are distinct values, not a common constructor) — so the same-length guard
            // both aligns the elements AND is a genuine value check. A list is HOMOGENEOUS (one element
            // type), so a per-element `(if c eᵢ fᵢ)` is well-typed. The backend builds it `vec-empty` +
            // per-element `vec-push`; hoisting shares that whole chain and selects only the differing
            // element, exactly as the tuple arm shares the `arr-alloc` + stores.
            (Core::ListNew { elems: et }, Core::ListNew { elems: ee }) if et.len() == ee.len() => (
                Shape::List,
                et.iter().copied().zip(ee.iter().copied()).collect(),
            ),
            (Core::Record { fields: ft }, Core::Record { fields: fe })
                if ft.len() == fe.len() && ft.keys().zip(fe.keys()).all(|(a, b)| a == b) =>
            {
                let keys: Vec<crate::resolved::Symbol> = ft.keys().cloned().collect();
                let pairs: Vec<(StructId, StructId)> =
                    keys.iter().map(|k| (ft[k], fe[k])).collect();
                (Shape::Record(keys), pairs)
            }
            _ => return None,
        };
    // Nothing to build (a nullary sum variant / empty tuple / empty record — no positions) offers no win
    // and would drop `cond` entirely; leave it to the enum-disc / identical-branch folds.
    if pairs.is_empty() {
        return None;
    }
    let mut diff = 0usize;
    let mut first_diff: Option<usize> = None;
    for (i, &(a, b)) in pairs.iter().enumerate() {
        if !core_equiv(db, a, b) {
            diff += 1;
            first_diff.get_or_insert(i);
        }
    }
    // A trapping `cond` needs both a count check AND an ORDER check before it may hoist.
    if !is_trap_free(db, cond) {
        // COUNT: `cond` is evaluated once per differing position; only ONE differing position matches
        // the original's single evaluation. Any other count would duplicate/drop a cond trap or effect.
        if diff != 1 {
            return None;
        }
        // ORDER: the original `if` evaluates `cond` FIRST — before ANY payload. But in the hoisted form
        // a SHARED (non-differing) payload BEFORE the differing position is built OUTSIDE the per-position
        // `if`, so it evaluates BEFORE `cond`. `core_equiv` admits trapping arith (a `/` by a runtime
        // divisor), so a preceding shared payload that traps would PREEMPT `cond`'s own trap — the hoist
        // would observe the WRONG trap. Only hoist a trapping cond when every shared payload preceding
        // the differing position is itself trap-free (then the first observable trap is still `cond`'s).
        let diff_idx = first_diff.expect("diff == 1 guarantees a differing position");
        for &(a, _) in &pairs[..diff_idx] {
            if !is_trap_free(db, a) {
                return None;
            }
        }
    }
    let mut vals: Vec<StructId> = Vec::with_capacity(pairs.len());
    for &(a, b) in &pairs {
        vals.push(if core_equiv(db, a, b) {
            a
        } else {
            synth_if_hoisted(db, cond, a, b)
        });
    }
    Some(match shape {
        Shape::Sum(disc) => Core::SumNew {
            disc,
            payloads: vals.into(),
        },
        Shape::Tuple => Core::Tuple { elems: vals.into() },
        Shape::List => Core::ListNew { elems: vals.into() },
        Shape::Record(keys) => Core::Record {
            fields: std::rc::Rc::new(keys.into_iter().zip(vals).collect()),
        },
    })
}

/// Hoist a COMMON OPERATOR out of both `if` arms — the arithmetic/conversion sibling of
/// `hoist_common_ctor`. When both branches apply the SAME operator to operands that mostly agree, the
/// operator (and its overflow guard, for checked arith) is DUPLICATED across the two branches:
///   `(if c (+ a 1) (+ b 1))`  → `(+ (if c a b) 1)`          — one checked add + one guard, not two
///   `(if c (* a k) (* b k))`  → `(* (if c a b) k)`
///   `(if c (< a k) (< b k))`  → `(< (if c a b) k)`          — one compare, not two (Compare is total)
///   `(if c (wrap a) (wrap b))`→ `(wrap (if c a b))`         — a unary `Convert`
/// Each DIFFERING operand position is pushed into its own `(if c pᵢ qᵢ)`; a `core_equiv` position is
/// shared directly, so the operator applies to exactly the operand tuple the taken arm would have used.
///
/// SOUND for ANY operator, INCLUDING a trapping checked op: the hoisted form applies the operator ONCE to
/// the SELECTED operands — the identical operand values the taken arm passed it — so it computes the same
/// result and traps under exactly the same condition (this is operand SELECTION under the op, NOT
/// reassociation: no operand ever crosses the operator or combines with a different partner). A shared
/// operand is `core_equiv` (a pure scalar — const/param/local/arith/compare/convert/proj) evaluated once,
/// reproducing the taken arm's single evaluation. `cond` is evaluated once per DIFFERING operand: exactly
/// one differing operand matches the original's single `cond` eval (unconditionally sound); 0 or ≥2
/// require a TRAP-FREE `cond` (the same guard `hoist_common_ctor` uses). Returns `None` unless both arms
/// are the same `Arith` operator, the same `Compare` operator (both over 2 operands), or the same
/// `Convert` op (1 operand) — a poison arm has neither core, so it never matches.
fn hoist_common_arith(
    db: &mut Db,
    cond: StructId,
    then_: StructId,
    else_: StructId,
) -> Option<Core> {
    enum Head {
        Arith(Prim),
        Compare(Prim),
        FloatCompare(Prim, u32),
        Convert(Prim),
    }
    let (head, pairs): (Head, Vec<(StructId, StructId)>) =
        match (core_of(db, then_), core_of(db, else_)) {
            (
                Core::Arith {
                    op: ot,
                    lhs: lt,
                    rhs: rt,
                },
                Core::Arith {
                    op: oe,
                    lhs: le,
                    rhs: re,
                },
            ) if ot == oe => (Head::Arith(ot), vec![(lt, le), (rt, re)]),
            // A COMPARISON is total (never traps, no guard), so the hoist is value-safe: `(if c (< a k)
            // (< b k))` → `(< (if c a b) k)` computes the same boolean (the comparison of the SELECTED
            // operand against `k`) with ONE compare instead of two. Operands are paired POSITIONALLY (`==`
            // on the `Prim` requires the same operator, so no `<`-vs-`>` mixup); the `Eq`-commutativity
            // `core_equiv` allows is irrelevant here since each position selects its own actual operand.
            // Value-safe is NOT trap-ORDER-safe, though: the compare's OPERANDS can still be trapping arith
            // (e.g. `(/ 100 d)`), and a SHARED trapping operand hoisted ahead of a trapping `cond` would
            // preempt `cond`'s trap. The shared-operand order guard below (`!is_trap_free(db, cond)` →
            // preceding-operand scan) applies to this Head arm exactly as it does to `Arith`.
            (
                Core::Compare {
                    op: ot,
                    lhs: lt,
                    rhs: rt,
                },
                Core::Compare {
                    op: oe,
                    lhs: le,
                    rhs: re,
                },
            ) if ot == oe => (Head::Compare(ot), vec![(lt, le), (rt, re)]),
            // A canonical-byte FLOAT equality (`Core::FloatCompare`) is total exactly like an integer
            // `Compare` — it canonicalizes each operand's NaN and compares the resulting bit patterns
            // (`i32.eq`/`i64.eq`), never trapping on its own — so `(if c (= a k) (= b k))` over Float
            // operands hoists to `(= (if c a b) k)`, ONE canon-and-compare instead of two, on exactly the
            // same value-safety footing as `Compare`. Both arms must share the operator AND the WIDTH (an
            // f32 compare canonicalizes to i32 bits and an f64 to i64 bits — mixing them would emit two
            // different machine ops off one hoisted operand). The shared-operand trap-ORDER guard below
            // applies identically (a `FloatCompare` operand can still be a trapping `/`).
            (
                Core::FloatCompare {
                    op: ot,
                    lhs: lt,
                    rhs: rt,
                    width: wt,
                },
                Core::FloatCompare {
                    op: oe,
                    lhs: le,
                    rhs: re,
                    width: we,
                },
            ) if ot == oe && wt == we => (Head::FloatCompare(ot, wt), vec![(lt, le), (rt, re)]),
            (
                Core::Convert {
                    op: ot,
                    operand: pt,
                },
                Core::Convert {
                    op: oe,
                    operand: pe,
                },
            ) if ot == oe => (Head::Convert(ot), vec![(pt, pe)]),
            _ => return None,
        };
    let mut diff = 0usize;
    let mut first_diff: Option<usize> = None;
    for (i, &(a, b)) in pairs.iter().enumerate() {
        if !core_equiv(db, a, b) {
            diff += 1;
            first_diff.get_or_insert(i);
        }
    }
    // All operands identical → the two arms are already `core_equiv`; the identical-branches fold handles
    // that (and would have fired first). Nothing to hoist here.
    if diff == 0 {
        return None;
    }
    // A trapping `cond` needs both a COUNT check AND an ORDER check before it may hoist — the same guard
    // `hoist_common_ctor` uses (a shared operand is built OUTSIDE the per-position `if`, so it evaluates
    // BEFORE `cond`).
    if !is_trap_free(db, cond) {
        // COUNT: `cond` is evaluated once per differing operand; only ONE differing operand matches the
        // original's single evaluation. Any other count would duplicate/drop a cond trap or effect.
        if diff != 1 {
            return None;
        }
        // ORDER: the original `if` evaluates `cond` FIRST — before ANY operand. In the hoisted form a
        // SHARED operand PRECEDING the differing one is built outside the per-position `if`, so it runs
        // before `cond`. `core_equiv` admits trapping arith (a `/` by a runtime divisor is `core_equiv`
        // to itself), so a preceding shared operand that traps would PREEMPT `cond`'s own trap — observing
        // the WRONG trap kind. Only hoist a trapping cond when every shared operand preceding the differing
        // one is itself trap-free (then the first observable trap is still `cond`'s). For binary `Arith`
        // the only preceding position is lhs when the diff is at rhs; a unary `Convert` has a single
        // operand so a diff==1 Convert has no preceding shared operand and is unaffected.
        let diff_idx = first_diff.expect("diff == 1 guarantees a differing position");
        for &(a, _) in &pairs[..diff_idx] {
            if !is_trap_free(db, a) {
                return None;
            }
        }
    }
    let mut operands: Vec<StructId> = Vec::with_capacity(pairs.len());
    for &(a, b) in &pairs {
        operands.push(if core_equiv(db, a, b) {
            a
        } else {
            synth_if_hoisted(db, cond, a, b)
        });
    }
    Some(match head {
        Head::Arith(op) => Core::Arith {
            op,
            lhs: operands[0],
            rhs: operands[1],
        },
        Head::Compare(op) => Core::Compare {
            op,
            lhs: operands[0],
            rhs: operands[1],
        },
        Head::FloatCompare(op, width) => Core::FloatCompare {
            op,
            lhs: operands[0],
            rhs: operands[1],
            width,
        },
        Head::Convert(op) => Core::Convert {
            op,
            operand: operands[0],
        },
    })
}

/// The "not-yet-computed on a runtime string" DECLINE for a string operation whose `arg` did not fold
/// to a constant — BUT only when `arg` is actually a `String`. When `arg` is NOT a string (`(Symbol.of
/// 5)` — a type error `infer` already reports as CDZ0203), the "runtime string" wording is a lie that
/// shadows the real type error; emit a NEUTRAL decline instead so the coded CDZ0203 is the story the
/// reader sees. (A genuine runtime string keeps the precise `msg`, the honest "constant strings only"
/// increment note.)
fn runtime_string_op_decline(db: &mut Db, arg: crate::ast::StructId, msg: &str) -> Core {
    if matches!(crate::infer::type_of(db, arg), crate::ty::Ty::String) {
        Core::Poison(Reject::decline(msg.to_string()))
    } else {
        // Not a string — the type mismatch (CDZ0203) is the authoritative report; do not claim a
        // "runtime string" it is not. This decline is generic (a lowering can't proceed on an
        // ill-typed operand) and defers to the coded type error.
        Core::Poison(Reject::decline(
            "this operation's operand is not a string (see the type error above)",
        ))
    }
}

/// Fold a constant SHIFT/BITWISE op (`BitAnd`/`BitOr`/`BitXor`/`Shl`/`Shr`) over the SOLVED WIDTH's u128
/// two's-complement bit pattern, rather than `fold_arith`'s i64 path. This is correct at ANY fixed width
/// (including UInt64, whose values above `i64::MAX` the i64 path spuriously rejects) AND for a small-operand
/// shift whose RESULT exceeds i64 but fits the solved width (`(<< (: 1 UInt64) 63)` = 2^63 fits UInt64 —
/// `checked_shl_i64` wrongly overflow-checked against Int64). Returns `Some(Core)` (the folded `ConstInt`
/// or a CDZ0304 trap for an out-of-range shift count / an overflow past the width), or `None` when the op is
/// not a shift/bitwise (caller falls back to `fold_arith`). Operands reaching here are non-negative at their
/// width (a signed value ≥ 2^63 cannot type — CDZ0302 at its literal), so `>>` is a logical shift; bitwise
/// ops are total on the value bits (masked to width). Trap semantics match the i64 helpers, generalized to
/// the solved width: a shift COUNT ≥ width traps, a `<<` bit shifted past the width traps.
fn fold_shift_bitwise_at_width(
    op: Prim,
    a: &IntValue,
    b: &IntValue,
    signed: bool,
    width: u32,
) -> Option<Core> {
    if !matches!(
        op,
        Prim::BitAnd | Prim::BitOr | Prim::BitXor | Prim::Shl | Prim::Shr
    ) {
        return None;
    }
    // UNSIGNED ONLY. A SIGNED type keeps `fold_arith`'s i64 path: (a) a signed `>>` is ARITHMETIC
    // (sign-extending) — `-256 >> 4 = -16` — which this logical-u128 fold would get wrong (it reads the
    // magnitude of a `wrap_to` result, so a negative value reads as 0); and (b) a signed value's high bits
    // are sign extension a bitwise mask would mismodel. The i64 path is already CORRECT for every signed
    // fixed width (its result range-check catches signed overflow); the ONLY thing it got wrong is an
    // UNSIGNED-width result above `i64::MAX` (`(<< (: 1 UInt64) 63)` = 2^63, `& (: u64max UInt64) …`), where a
    // logical width-masked fold is exactly right (unsigned operands are non-negative, `>>` is logical).
    if signed {
        return None;
    }
    // The operands' low-`width` bit patterns as u128 (`wrap_to` = the canonical two's-complement-at-width the
    // backend uses; a non-negative value's magnitude IS its bit pattern — `to_u128`).
    let bits = |v: &IntValue| -> u128 { v.wrap_to(signed, width).to_u128().unwrap_or(0) };
    let (xb, yb) = (bits(a), bits(b));
    let mask = if width >= 128 {
        u128::MAX
    } else {
        (1u128 << width) - 1
    };
    // `Ok(value)` folds; `Err(reason)` is a provable trap → CDZ0304 carrying the SPECIFIC cause (an
    // out-of-range shift count vs a shifted-result overflow), matching `fold_arith`/`const_trap_cause`'s
    // actionable style rather than one generic "count or overflow" message. Bitwise ops never trap.
    let folded: Result<u128, String> = match op {
        Prim::BitAnd => Ok((xb & yb) & mask),
        Prim::BitOr => Ok((xb | yb) & mask),
        Prim::BitXor => Ok((xb ^ yb) & mask),
        // `<<`: count must be in `0..width`; fold only when no bit is shifted PAST the width (else the checked
        // default traps — matching the i64 helper's `None`-on-overflow, but against the SOLVED width not i64).
        Prim::Shl => {
            let count = yb;
            if count >= width as u128 {
                Err(format!(
                    "shift count {count} is out of range for the {width}-bit type \
                     (a shift count must be 0..={})",
                    width - 1
                ))
            } else {
                match xb.checked_shl(count as u32) {
                    Some(s) if s == (s & mask) => Ok(s),
                    _ => Err(format!(
                        "the shifted result overflows the {width}-bit type \
                         (a `<<` by {count} moves a set bit past the width) — shift a smaller value \
                         or use a wider type"
                    )),
                }
            }
        }
        // `>>`: logical shift (operand non-negative at its width); count in `0..width`.
        Prim::Shr => {
            let count = yb;
            if count >= width as u128 {
                Err(format!(
                    "shift count {count} is out of range for the {width}-bit type \
                     (a shift count must be 0..={})",
                    width - 1
                ))
            } else {
                Ok((xb >> count) & mask)
            }
        }
        _ => unreachable!("guarded by the matches! above"),
    };
    Some(match folded {
        Ok(r) => {
            trace!(target: "rcdzc::fold", op = intrinsic_name(op), "constant shift/bitwise folded over the solved width");
            Core::ConstInt(IntValue::from_u128(r))
        }
        Err(reason) => {
            trace!(target: "rcdzc::fold", op = intrinsic_name(op), %reason, "constant shift traps → CDZ0304");
            Core::Poison(Reject::coded(
                Code::ConstTrap,
                format!("this constant `{}` traps: {reason}", intrinsic_name(op)),
            ))
        }
    })
}

/// Fold a constant arithmetic operation with a CHECKED evaluation. Both operands are compile-time
/// constants; if the operation's defined outcome on them is a trap (an overflow the checked default
/// forbids, or an operand outside the machine range the fold evaluates over), the result is a poison
/// carrying CDZ0304 — the build fails rather than shipping a runtime trap. On success the result is a
/// `ConstInt`. The evaluation is over `i64` (the Stage default integer); a later width stage
/// generalizes the range the check tests to the operands' solved width.
fn fold_arith(op: Prim, a: IntValue, b: IntValue) -> Core {
    let (x, y) = match (a.to_i64(), b.to_i64()) {
        (Some(x), Some(y)) => (x, y),
        // An operand beyond the machine range the fold evaluates over — a provable width trap.
        _ => {
            return Core::Poison(Reject::coded(
                Code::ConstTrap,
                "constant operand does not fit the integer width",
            ));
        }
    };
    // Each integer op evaluates over `i64` (the Stage default width) with the DEFINED numeric-model
    // semantics; `None` marks a provable trap the checked default forbids (`numeric-model.md` §Overflow
    // Is Defined). A later width stage generalizes the range/count the checks test to the solved width.
    let checked = match op {
        Prim::Add => x.checked_add(y),
        Prim::Sub => x.checked_sub(y),
        Prim::Mul => x.checked_mul(y),
        // Division truncates toward zero; traps on a zero divisor and on `MIN / -1` (Rust's
        // `checked_div` returns `None` for both — exactly the two defined traps).
        Prim::Div => x.checked_div(y),
        // Remainder takes the dividend's sign; traps on a zero divisor. `MIN % -1` is 0 (no overflow),
        // but Rust's `%` panics there — `checked_rem` returns `None`, so special-case it to 0.
        Prim::Rem => {
            if y == -1 {
                Some(0)
            } else {
                x.checked_rem(y)
            }
        }
        // A left shift is exact multiplication by `2^count`: it traps on an out-of-range count
        // (< 0 or ≥ width) AND on overflow past the width — NOT wasm's silent mask-and-wrap.
        Prim::Shl => checked_shl_i64(x, y),
        // Arithmetic (sign-extending) right shift; traps on an out-of-range count, never overflows.
        Prim::Shr => checked_shr_i64(x, y),
        // Bitwise operations are total on the two's-complement value — never trap.
        Prim::BitAnd => Some(x & y),
        Prim::BitOr => Some(x | y),
        Prim::BitXor => Some(x ^ y),
        // A non-integer-binary prim never reaches the fold (`lower_arith` is only called for an
        // `is_arith` prim), so these arms are unreachable in practice; decline rather than panic.
        Prim::Lt
        | Prim::Gt
        | Prim::Le
        | Prim::Ge
        | Prim::Eq
        | Prim::Compare
        | Prim::Wrap
        | Prim::CheckedOf
        | Prim::IntCtor
        | Prim::UIntCtor
        | Prim::FnCtor
        | Prim::TupleCtor
        | Prim::RecordCtor
        | Prim::BoolTy
        | Prim::UnitTy
        | Prim::SumNew
        | Prim::SumCtor
        | Prim::TupleNew
        | Prim::RecordNew
        | Prim::RecordProject
        | Prim::RecordWithout
        | Prim::RecordMerge
        | Prim::RecordExtend
        | Prim::RecordWith
        | Prim::RecordPop
        | Prim::TupleCat
        | Prim::TupleSplitAt
        | Prim::TuplePop
        | Prim::ListNew
        | Prim::ListLen
        | Prim::ListPush
        | Prim::ListPrepend
        | Prim::ListConcat
        | Prim::ListUpdate
        | Prim::ListAt
        | Prim::AstSpliceLift
        | Prim::AstLift
        | Prim::AstEncode
        | Prim::AstDecode
        | Prim::Blake3Of
        | Prim::ListCtor
        | Prim::BytesOf
        | Prim::BytesLen
        | Prim::BytesTy
        | Prim::StrScalarLen
        | Prim::StrByteLen
        | Prim::StrAt
        | Prim::StrScalarAt
        | Prim::StrConcat
        | Prim::StrSlice
        | Prim::StrToBytes
        | Prim::StrFromBytes
        | Prim::SumExpect
        | Prim::CheckedAdd
        | Prim::CheckedSub
        | Prim::CheckedMul
        | Prim::WrappingAdd
        | Prim::WrappingSub
        | Prim::WrappingMul
        | Prim::StringTy
        | Prim::BytesAt
        | Prim::BytesConcat
        | Prim::BytesSlice
        | Prim::BytesCompact
        // Float arithmetic is folded by `lower_float_arith` (an f64/f32 fold), not this integer fold.
        | Prim::FAdd
        | Prim::FSub
        | Prim::FMul
        | Prim::FDiv
        | Prim::FloatCtor
        | Prim::FloatOfInt
        | Prim::FloatOf
        | Prim::FloatNan
        | Prim::FloatInf
        | Prim::MapCtor
        | Prim::MapNew
        | Prim::MapEmpty
        | Prim::MapInsert
        | Prim::MapLookup
        | Prim::MapRemove
        | Prim::MapSize
        | Prim::MapSwap
        | Prim::MapTake
        | Prim::SetCtor
        | Prim::SetOf
                | Prim::MapToList
| Prim::SetToList
        | Prim::SetContains
        | Prim::SetLen
        | Prim::SetInsert
        | Prim::SetRemove
        | Prim::SetUnion
        | Prim::SetIntersection
        | Prim::SetDifference
        | Prim::CharTy
        | Prim::CharToInt
        | Prim::CharFromInt
        // Value.encode/decode are unary value↔Bytes conversions (they lower in their own apply-dispatch
        // arm, like Char.to-int), NOT integer binary ops — unreachable in the arith fold, decline.
        | Prim::ValueEncode
        | Prim::ValueDecode
        | Prim::SymbolTy
        | Prim::SymbolOf
        | Prim::SymbolToString
        // `BigIntTy` is a ground type-value builder (bare `BigInt` in type position → `Ty::BigInt`),
        // and `BigIntOf` is the unary widening conversion (folds in its own arm above) — neither is an
        // integer BINARY operation, like `StringTy`/`SymbolTy`/`SymbolOf`. `RationalTy` is likewise a
        // ground type-value builder (bare `Rational` → `Ty::Rational`), not an integer binary op.
        | Prim::BigIntTy
        | Prim::BigIntOf
        | Prim::RationalTy
        // `RationalOf`/`RationalOfInt`/`RationalValue`/`RationalNum`/`RationalDen` are rational construction/
        // conversion/accessor ops (they fold in their own arms), not integer binary operations.
        | Prim::RationalOf
        | Prim::RationalOfInt
        | Prim::RationalValue
        | Prim::RationalNum
        | Prim::RationalDen
        | Prim::RationalTruncate
        | Prim::RationalFloor
        | Prim::RationalCeil
        | Prim::RationalRound
        // The unit/quantity prims are compile-time unit builders / erasing quantity ops — never an
        // integer binary operation (a `Qty.of`/`Qty.value` lowers to its value argument, a unit builder
        // is reduced away by `eval`), so they never reach this integer fold.
        | Prim::UnitOne
        | Prim::UnitBase
        | Prim::UnitMul
        | Prim::UnitDiv
        | Prim::UnitPow
        | Prim::UnitPrefix
        | Prim::UnitOf
        | Prim::UnitDefine
        | Prim::UnitIn
        | Prim::QtyOf
        | Prim::QtyValue
        | Prim::QtyPow
        | Prim::QtyUnit
        | Prim::QtyCtor
        | Prim::TypeOf
        | Prim::TypeEq
        // `trap` is the diverging primitive (lowered to `Core::Trap`), never an integer binary operation.
        | Prim::Trap
        // `print`/`read` are the AST-value text printer/reader (`Ast → String` / `String → Ast`), folded
        // in `lower_print`/`lower_read`, never an integer binary operation.
        | Prim::Print
        | Prim::Read
        // `Ast.module` is the self-reflection magic-constant (folds to the enclosing module's `Ast`
        // value at lowering — v-compiler-primitives' fill), never an integer binary operation.
        | Prim::ReflectModule
        | Prim::FEq
        | Prim::FLt
        | Prim::FLe
        | Prim::FGt
        | Prim::FGe => {
            return Core::Poison(Reject::decline("not an integer binary operation"));
        }
    };
    match checked {
        Some(n) => {
            trace!(target: "rcdzc::fold", op = intrinsic_name(op), result = n, "folded constant integer op");
            Core::ConstInt(IntValue::from_i64(n))
        }
        // A provable trap — the checked default traps, and the compiler can prove it, so the build
        // fails (CDZ0304) rather than emitting a component that traps (`numeric-model.md` §A Constant
        // Operation With No Value Is Rejected At Compile Time).
        None => {
            trace!(target: "rcdzc::fold", op = intrinsic_name(op), "constant op traps → CDZ0304 (fails build)");
            Core::Poison(Reject::coded(
                Code::ConstTrap,
                format!(
                    "constant {} traps: {}",
                    intrinsic_name(op),
                    const_trap_cause(op, y),
                ),
            ))
        }
    }
}

/// The SPECIFIC cause of a provable constant-integer trap (CDZ0304) for op `op` with right operand `y`
/// — the compiler proved the trap, so it knows which of the three defined causes fired and names THAT
/// ("divide by zero" / "overflows Int64" / "shift count N out of range 0..64") rather than listing all
/// three. The evaluation is over the Stage-default `i64`, so overflow is against `Int64`; a later width
/// stage would name the solved width. Only called when the fold returned `None` (a genuine trap), so a
/// default arm is a defensive fallback, not a reachable case. (The left operand doesn't disambiguate a
/// cause — every trap is decided by the op and the divisor/shift-count `y`.)
fn const_trap_cause(op: Prim, y: i64) -> String {
    match op {
        // Name the cause AND the repair — a provable constant trap has a definite fix (there is no runtime
        // value that rescues it), so match the actionable bar the runtime-numerator `(/ n 0)` sibling and
        // the checked-conversion message already set, rather than stating only the cause.
        Prim::Div | Prim::Rem if y == 0 => {
            "divide by zero — a division by the constant 0 has no value; remove it or use a nonzero divisor"
                .to_string()
        }
        // The only non-zero-divisor `Div` trap is `Int64.min / -1` (the quotient overflows Int64).
        Prim::Div => {
            "the quotient overflows Int64 (Int64.min / -1 has no Int64 value) — the only overflowing division"
                .to_string()
        }
        Prim::Add | Prim::Sub | Prim::Mul => {
            "the result overflows Int64 — compute in a wider type, or check the operands fit".to_string()
        }
        Prim::Shl | Prim::Shr => {
            if !(0..64).contains(&y) {
                format!(
                    "shift count {y} is out of range 0..64 — a shift count must be 0..=63 (the bit width)"
                )
            } else {
                // An in-range Shl whose exact result overflows the width.
                "the shifted result overflows Int64 — shift a smaller value or use a wider type".to_string()
            }
        }
        _ => "the operation has no defined value on these operands".to_string(),
    }
}

/// A left shift as EXACT multiplication by `2^count`: `None` (a provable trap) if the count is outside
/// `0..64` or the exact result overflows `i64` — a left shift is not exempt from Overflow Is Defined,
/// so it traps like `*` rather than masking the count and wrapping (`numeric-model.md`).
fn checked_shl_i64(x: i64, count: i64) -> Option<i64> {
    if !(0..64).contains(&count) {
        return None;
    }
    // Multiply by 2^count and narrow to `i64`, `None` on overflow — the defined meaning of a left
    // shift. The product is computed in `i128` because the `2^count` factor is itself not always an
    // `i64`: `1i64 << 63` is `i64::MIN` (a NEGATIVE 2^63), so a signed factor miscomputes both
    // `1 << 63` (folds to `i64::MIN` instead of overflowing) and `-1 << 63` (overflows the signed
    // multiply instead of yielding `i64::MIN`). In `i128`, `2^count` (count < 64) and its product
    // with any `i64` both fit exactly, so the single `i64::try_from` fit-check is the whole rule.
    i64::try_from((x as i128) << count).ok()
}

/// An ARITHMETIC (sign-extending) right shift: `None` if the count is outside `0..64` (an out-of-range
/// count traps rather than masking). Never overflows. The signed shift preserves the sign bit, so
/// shifting a negative value right fills with ones (e.g. `-256 >> 7 = -2`).
fn checked_shr_i64(x: i64, count: i64) -> Option<i64> {
    if !(0..64).contains(&count) {
        return None;
    }
    Some(x >> count)
}

/// Lower a COMPARISON application (`< > <= >= =`). Folds two constant SCALARS (integers or booleans) to
/// a `ConstBool` — a total ordering on the scalar's value. A RUNTIME scalar operand (a function
/// parameter) becomes a `Core::Compare` the backend emits as a machine comparison. A COMPOUND operand
/// (a record/heap value) still declines — structural comparison over the value heap is a later stage.
/// The operator's type stays fully generic (`∀a. a → a → Bool`). A poison operand propagates.
/// The Less/Equal/Greater discriminants of the built-in `Ordering` sum (this node's solved result type),
/// read off the declaration by variant NAME (not baked) — the `Ordering` analogue of `option_discs`.
fn ordering_discs(db: &mut Db, id: StructId) -> Option<(u32, u32, u32)> {
    let crate::ty::Ty::Sum { decl, .. } = crate::infer::type_of(db, id) else {
        return None;
    };
    let decl_ref = db.type_decl_by_occ(decl)?;
    let (mut lt, mut eq, mut gt) = (None, None, None);
    for (i, v) in decl_ref.variants.iter().enumerate() {
        match v.name.as_str() {
            "Less" => lt = Some(i as u32),
            "Equal" => eq = Some(i as u32),
            "Greater" => gt = Some(i as u32),
            _ => {}
        }
    }
    Some((lt?, eq?, gt?))
}

/// A branch/arm occurrence placed in a CONDITIONALLY-reached position — an `if` branch, a `match` arm body.
/// If it const-folded to a PROVABLE `ConstTrap` (a compile-visible divide-by-zero / overflow / out-of-bounds),
/// DEMOTE it to a runtime `Core::Trap`: the trap fires only when the branch is actually TAKEN, matching strict
/// branch-lazy evaluation and the cn02 precedent (operator ruling 2026-08-27: "if branch reachability depends on
/// a runtime value we should absolutely not error out and should trap at runtime"). `(if (> n 0) 7 (/ 1 0))` at
/// n>0 must return 7, not compile-error.
///
/// ONLY a `ConstTrap` is demoted — a NON-trap poison (an ill-FORMED branch: an unbound name, a type fault) stays
/// a poison, a static well-formedness fault the program is rejected for regardless of reachability. An
/// UNCONDITIONAL `ConstTrap` (on the strict spine, NOT a guarded branch) is never routed here, so it still
/// surfaces CDZ0304 via `collect_reached_poisons` / emit — the "statically-unconditional trap" carve-out the
/// ruling keeps. A `(const ...)` demand likewise still surfaces CDZ0304 (its ConstTrap path / `const_eval`).
/// `Core::Trap` is stack-polymorphic (an `unreachable`), so it validates in the branch's result position.
fn demote_conditional_trap(db: &mut Db, branch: StructId) -> StructId {
    if let Core::Poison(r) = core_of(db, branch)
        && r.code == Some(Code::ConstTrap)
    {
        let ty = crate::infer::type_of(db, branch);
        // PRESERVE the trap KIND across the demote (operator ruling 2026-08-27, Lean-oracle finding): a const
        // divide/remainder-by-zero must surface "divide by zero" at runtime — the SAME kind a runtime `(/ n 0)`
        // raises — not the bare "unreachable" a plain `Core::Trap` reports. The ConstTrap message names its
        // cause (`const_trap_cause`: "divide by zero — …" / the const-eval "division by zero"), so classify off
        // it (the same vocabulary `cdz-corpus-grade::trap_kind` canonicalizes to `div-by-zero`). Any other
        // provable-trap kind still demotes to the kind-less `Core::Trap` (its corpus expectation is unchanged).
        if is_div_by_zero_trap(&r.message) {
            trace!(target: "rcdzc::lower", branch = branch.0, "demote a provable const DIVIDE-BY-ZERO in a conditionally-reached branch to a KIND-PRESERVING runtime Core::TrapDivZero (operator ruling)");
            return synth_core(db, Core::TrapDivZero, ty);
        }
        if is_overflow_trap(&r.message) {
            trace!(target: "rcdzc::lower", branch = branch.0, "demote a provable const INTEGER-OVERFLOW in a conditionally-reached branch to a KIND-PRESERVING runtime Core::TrapOverflow (operator ruling)");
            return synth_core(db, Core::TrapOverflow, ty);
        }
        trace!(target: "rcdzc::lower", branch = branch.0, "demote a provable ConstTrap in a conditionally-reached branch to a runtime Core::Trap (cn02 / operator ruling)");
        return synth_core(db, Core::Trap, ty);
    }
    branch
}

/// Whether a `ConstTrap` poison message names a DIVIDE/REMAINDER-BY-ZERO cause — the demote reads this to
/// preserve the specific trap kind (see [`demote_conditional_trap`]). Mirrors the vocabulary
/// `cdz-corpus-grade::trap_kind` canonicalizes to `div-by-zero` (`const_trap_cause`'s "divide by zero"
/// wording and the const-eval `CVal::Trap("division by zero")`), so the demoted trap grades identically to a
/// runtime division-by-zero on both backends.
fn is_div_by_zero_trap(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("divide by zero")
        || m.contains("division by zero")
        || m.contains("remainder by zero")
}

/// Whether a `ConstTrap` poison message names an INTEGER-OVERFLOW cause — the demote reads this to preserve
/// the "overflow" trap kind (see [`demote_conditional_trap`]). Every arithmetic-overflow message
/// `const_trap_cause` emits says "overflow" ("the result overflows Int64", "the quotient overflows Int64",
/// "the shifted result overflows Int64"), which `cdz-corpus-grade::trap_kind` canonicalizes to `overflow`.
/// Checked AFTER div-by-zero (a `Div` by 0 says "divide by zero", not "overflow"), so the two never collide.
/// A shift-COUNT-out-of-range ("shift count N is out of range 0..64") does NOT say "overflow" — it stays a
/// kind-less `Core::Trap` (wasm masks the count, so there is no native overflow trap to surface for it).
fn is_overflow_trap(message: &str) -> bool {
    message.to_ascii_lowercase().contains("overflow")
}

/// The type to use for a comparison's compound-ORDERABILITY check (and the `ValueCmp` descriptor). The type
/// checker UNIFIED both operands, but `type_of` need not apply the final substitution at this query, so it can
/// render an UNRESOLVED element var for a side carrying no direct evidence — a bare empty `(list)` whose element
/// type comes ONLY from its sibling. Checking that side ALONE misclassifies the unified compound: `(< (list)
/// (list 1))` sees `(List ?)`, fails `is_orderable_compound`, and declines as "no total order" though the element
/// is `Int64` by unification (breaker olf4 / wrong-diagnostic #11). Prefer whichever sibling's type is a resolved
/// ORDERABLE compound; else fall back to `a`'s type (for the honest decline / the runtime descriptor). This is the
/// established sibling-check idiom (cf. the one-sided-scalar comparison arm) — unify-then-check without a
/// workaround: since unification made the two operands the same type, a resolved sibling IS this operand's type.
fn comparison_compound_ty(db: &mut Db, a: StructId, b: StructId) -> crate::ty::Ty {
    let ta = crate::infer::type_of(db, a);
    if is_orderable_compound(db, &ta) {
        return ta;
    }
    let tb = crate::infer::type_of(db, b);
    if is_orderable_compound(db, &tb) {
        return tb;
    }
    ta
}

/// Lower `(compare a b)` — the three-way comparison yielding an `Ordering` (core-semantics.md §A Total
/// Order Is Observed Through A Three-Way Comparison). FOLD a constant scalar/string operand pair to the
/// matching `Ordering` variant — `Less`/`Equal`/`Greater` by the operands' `cmp`, built as a NULLARY
/// `Core::SumNew` at the result Ordering's discs (`ordering_discs`), so it rides the ordinary sum
/// fold/escape/match exactly as a variant constructor does. A float pair compares by canonical value
/// (IEEE partial order; a NaN pair — unordered — declines). A compound or runtime operand declines (the
/// heap-walk / runtime three-way compare is a later stage), mirroring `lower_comparison`.
///
/// The order each scalar type offers is TOTAL and a deterministic function of the values: Int by numeric
/// value, Char by scalar value, String lexicographically, and Bool by `false < true` (Rust `bool::cmp`).
/// Every ordered pair folds to exactly one of Less/Equal/Greater, and the fold reads only the operand
/// values — no environment, order, or outside influence enters.
//= spec/capabilities/core-semantics.md#ordering-where-offered-is-total
//# A type that offers an ordering MUST offer a total order over its values.
//= spec/capabilities/core-semantics.md#ordering-where-offered-is-total
//# The ordering a type offers MUST be a deterministic function of the values compared.
//= spec/capabilities/core-semantics.md#ordering-where-offered-is-total
//# The Bool type MUST offer a total order in which false is less than true.
fn lower_compare(db: &mut Db, id: StructId, lhs: StructId, rhs: StructId) -> Core {
    use std::cmp::Ordering::{Equal, Greater, Less};
    let Some((lt, eq, gt)) = ordering_discs(db, id) else {
        return Core::Poison(Reject::decline(
            "compare result is not the built-in Ordering sum",
        ));
    };
    // The ordering of the two constant operands, or `None` to decline (non-constant / non-scalar / a NaN
    // pair / an operand beyond the fold's machine range).
    let ord = match (core_of(db, lhs), core_of(db, rhs)) {
        (Core::ConstInt(a), Core::ConstInt(b)) => match (a.to_i64(), b.to_i64()) {
            (Some(x), Some(y)) => Some(x.cmp(&y)),
            _ => None,
        },
        (Core::ConstBool(a), Core::ConstBool(b)) => Some(a.cmp(&b)),
        // Two strings order LEXICOGRAPHICALLY by their Unicode scalar sequences. `String::cmp` compares the
        // UTF-8 bytes, and UTF-8 byte order coincides with scalar-value order, so this is exactly the
        // scalar-sequence lexicographic order the spec fixes (`"a" < "ab" < "b"`).
        //= spec/capabilities/collections-and-text.md#string-comparison-is-defined-on-scalar-values
        //# An ordering over strings MUST be the lexicographic order of their Unicode scalar value sequences.
        (Core::ConstStr(a), Core::ConstStr(b)) => Some(a.cmp(&b)),
        // Two chars order by scalar value (`compare #\a #\b` → Less).
        (Core::ConstChar(a), Core::ConstChar(b)) => Some((a as u32).cmp(&(b as u32))),
        (Core::ConstFloat(a), Core::ConstFloat(b)) => {
            f64::from_bits(a.to_f64_bits()).partial_cmp(&f64::from_bits(b.to_f64_bits()))
        }
        (Core::Poison(r), _) | (_, Core::Poison(r)) => return Core::Poison(r),
        // A CONSTANT COMPOUND pair (a tuple/record/sum of constant leaves) folds through the shared
        // canonical value order — `const_key_order`, the SAME order `Set.to-list`/`Map.to-list`/equality
        // use, recursing element-/field-/payload-wise through every orderable shape. It mirrors the runtime
        // `value_cmp_shaped` the compound `Core::ValueCmp { op: Compare }` walk below uses, so the constant
        // fold and the runtime walk report the SAME Ordering (§331 — the two surfaces cannot diverge).
        //
        // GUARD: only fold when BOTH operands are FULLY-constant values. This fold DISCARDS the operands
        // (it returns a bare nullary Ordering variant), so a non-constant subterm — e.g. `(Some (/ 1 k))`,
        // whose construction can TRAP or perform an effect — must not be dropped. `const_key_order` can
        // decide the order from an early differing DISCRIMINANT (or an earlier differing field) WITHOUT ever
        // reading a deeper payload, so its own `Some` is NOT proof the operands are effect-free: the explicit
        // `is_const_value` check is what prevents eliding a live subterm. Two fully-constant operands have no
        // construction effect to preserve, so discarding them is sound. A non-constant (or runtime) operand
        // falls through to the runtime `ValueCmp` walk UNCHANGED; a const compound with a float/set/map leaf
        // yields `None` from `const_key_order` and then reaches the `is_orderable_compound`/float declines
        // below exactly as before. The scalar arms above take precedence (match order), preserving their
        // i64-range / NaN-partial semantics.
        //
        // ORDERABILITY: fold ONLY a compound the RUNTIME `Core::ValueCmp` walk also orders —
        // `is_orderable_compound` (Int/Bool/String/Bytes + nested; NOT Char/Float). `const_key_order` is more
        // permissive (it orders a Char leaf, for baking a Set/Map to-list — an enumeration order that never
        // calls a runtime op), but a Char/Float-leaf compound `Ordering.of` DECLINES at runtime (03-equality),
        // so folding it here would diverge from the runtime path (§331). Gating on `is_orderable_compound`
        // makes such a compound yield `None` and fall through to the runtime `ValueCmp`/float declines below.
        _ if is_const_value(db, lhs) && is_const_value(db, rhs) && {
            let ty = comparison_compound_ty(db, lhs, rhs);
            is_orderable_compound(db, &ty)
        } =>
        {
            const_key_order(db, lhs, rhs)
        }
        _ => None,
    };
    match ord {
        Some(o) => {
            let disc = match o {
                Less => lt,
                Equal => eq,
                Greater => gt,
            };
            trace!(target: "rcdzc::fold", node = id.0, ?o, "compare folds to an Ordering variant");
            Core::SumNew {
                disc,
                payloads: Vec::new().into(),
            }
        }
        None => {
            // The constant fold did not decide it. A RUNTIME SCALAR pair (two Int/Bool operands neither
            // of which is a compile-time constant) is a TOTAL order the machine can compare directly — the
            // §331 requirement that `compare` AGREE with the boolean ordering operators. Desugar to the
            // nested-`if` over the existing scalar boolean `Core::Compare` ops (the same machine `lt`/`gt`
            // the `<`/`>` operators emit), yielding the Ordering variant — NO new runtime op:
            //   `compare a b`  →  `if (a < b) Less else if (a > b) Greater else Equal`
            // Both operands are read TWICE (the `<` and the `>`), so each is MATERIALIZED ONCE via a
            // `Core::Let { (op, op) }` self-keyed binding read through a shared `LocalRef` — an operand can
            // carry a trap or a perform (`(compare (/ a b) c)`), and re-emitting its computation would
            // double-evaluate it (the same materialize-once discipline the runtime bin-match scrutinee uses).
            //= spec/capabilities/core-semantics.md#a-total-order-is-observed-through-a-three-way-comparison
            //# The boolean ordering operators MUST agree with the three-way comparison, so that a type has one total order surfaced two ways that cannot disagree.
            // Float is NOT covered — a runtime float `compare` (a NaN operand is unordered) and a general
            // compound operand (the descriptor-guided `value-cmp` heap walk, a later slice pending the
            // `value-cmp` emit) still decline here. Every OTHER machine-orderable runtime type — Int/Bool
            // (`Core::Compare`), String/Symbol (`Core::StrCmp`), BigInt (`Core::BigIntCmp`), Rational
            // (`Core::RationalCmp`) — already realizes its total order as a boolean ordering op the `<`/`>`
            // operators emit, so it takes the same nested-if desugaring over that op.
            let operands = [lhs, rhs];
            let kind = if is_scalar(db, lhs) && is_scalar(db, rhs) {
                Some(CmpKind::Scalar)
            } else if node_ty_is_enum_disc(db, lhs) {
                // ENUM-DISCRIMINANT three-way compare (breaker #43): an ALL-NULLARY sum is a bare i32
                // discriminant (no heap box), ordered by that discriminant (core-semantics §Compound
                // Ordering: "a sum orders first by the discriminant"). Route to `CmpKind::Scalar` so the
                // three-way desugars to `if (a<b) Less else if (a>b) Greater else Equal` over `Core::Compare`
                // i32 TAG compares — the SAME path the `<`/`>` enum-disc arm now takes (§331: compare AGREES
                // with the boolean ops). MUST precede the `is_orderable_compound → ValueCmp` arm below: an
                // all-nullary `Ty::Sum` is orderable, but routing it to the runtime `value-cmp` heap walk
                // MISCOMPILES — `value-cmp`'s `op_sum_disc` expects a boxed sum node, gets a bare immediate
                // i32, reads disc 0 for both → Equal for distinct variants (the #43 bug: compare=Equal while
                // `=`=false). The type checker unified both operands, so the single-side check covers both.
                // A payload-carrying sum's nullary variants box (shared heap rep) → the `ValueCmp` arm.
                //= spec/capabilities/core-semantics.md#a-total-order-is-observed-through-a-three-way-comparison
                //# The boolean ordering operators MUST agree with the three-way comparison, so that a type has one total order surfaced two ways that cannot disagree.
                Some(CmpKind::Scalar)
            } else if operand_is_string_or_symbol(db, lhs) && operand_is_string_or_symbol(db, rhs) {
                Some(CmpKind::Str)
            } else if bigint_operand(db, &operands) {
                // A `BigInt`/fixed mix is rejected CDZ0301 before lowering, so a BigInt operand proves both
                // are BigInt — the same invariant `lower_comparison`'s `bigint_operand` dispatch relies on.
                Some(CmpKind::BigInt)
            } else if rational_operand(db, &operands) {
                Some(CmpKind::Rational)
            } else {
                None
            };
            match kind {
                Some(kind) => lower_runtime_compare(db, id, lhs, rhs, (lt, eq, gt), kind),
                // Neither a machine-orderable runtime type nor a constant fold. Two distinct reasons, kept
                // as SEPARATE messages (accuracy matters — every scalar/String/Symbol/BigInt/Rational
                // `compare`, constant or runtime, now computes, so the old blanket "constant scalars only"
                // is wrong):
                //  • a FLOAT operand does not OFFER a three-way `compare` — a floating-point type is the IEEE
                //    PARTIAL order (a NaN is unordered), NOT the total order `compare` reports, so this is a
                //    permanent CARVE-OUT, not a not-yet (§319 / numeric-model §the floating-point relational
                //    operators are a distinct facility). The fix is to use the boolean relational operators
                //    (`<`/`<=`/`>`/`>=`), which DO work on floats.
                //  • a COMPOUND operand (tuple/record/list/sum) whose leaves are all orderable routes to the
                //    runtime `value-cmp` three-way walk (`Core::ValueCmp { op: Prim::Compare }`) — the SAME
                //    descriptor-guided heap walk the boolean compound `<`/`<=`/`>`/`>=` already emit, so the
                //    two surfaces agree (§331) exactly as the scalar/String/BigInt/Rational cases do. The
                //    boolean ops map the walk's `-1/0/1` to a Bool; the three-way `compare` instead builds the
                //    `Ordering` sum (Less/Equal/Greater at discs 0/1/2 — all-nullary enum, a bare `res+1`), the
                //    emit's `op=Compare` arm (v-runtime `select.rs` `ValueCmp`). Until that emit arm lands, a
                //    compound `compare` DECLINES cleanly AT EMIT ("ValueCmp carries a non-ordering prim") rather
                //    than here — same reject verdict (no miscompile: `ValueCmp` reads `op` at emit, and the
                //    non-ordering guard rejects `Compare`), so routing it now is safe and unblocks the emit work.
                //    A float/bytes/set/map leaf makes the compound un-orderable (`is_orderable_compound` false)
                //    → the float/heap-walk declines below (§319 float partial; no blessed Bytes/Set/Map order).
                //= spec/capabilities/core-semantics.md#compound-ordering-is-lexicographic
                //# The total order a compound offers MUST agree with its structural equality — two values compare equal under the three-way comparison exactly when they are equal — and MUST be surfaced through the same three-way comparison and boolean ordering operators as any other total order.
                None if {
                    let lhs_ty = crate::infer::type_of(db, lhs);
                    is_orderable_compound(db, &lhs_ty)
                } =>
                {
                    trace!(target: "rcdzc::lower", node = id.0, "compare of a runtime orderable compound → ValueCmp{{op:Compare}} (three-way value-cmp heap walk)");
                    Core::ValueCmp {
                        op: Prim::Compare,
                        lhs,
                        rhs,
                        ty: crate::infer::type_of(db, lhs),
                    }
                }
                //= spec/capabilities/core-semantics.md#ordering-where-offered-is-total
                //# A floating-point type MUST NOT be treated as offering an ordering in the sense of this section, because its relational operators are the IEEE partial order defined for the floating-point type rather than a total order — so the requirement that an offered ordering be total does not apply to the floating-point relational operators.
                None if float_operand(db, &operands) => Core::Poison(Reject::decline(
                    "`compare` needs a total order, but a floating-point type offers only the IEEE partial order (a not-a-number is unordered), so it has no three-way comparison — use the relational operators `<`, `<=`, `>`, `>=` instead",
                )),
                None => Core::Poison(Reject::decline(
                    "`compare` of this value has no total order the compiler can walk yet (a float/bytes/set/map leaf, or an un-orderable shape) — compare its orderable components individually",
                )),
            }
        }
    }
}

/// The machine ORDERING op a runtime `compare` desugars its two boolean sub-comparisons to — one per
/// machine-orderable type family: a scalar `Core::Compare` (Int/Bool), a String/Symbol `Core::StrCmp` (the
/// content-lexicographic byte walk), a BigInt `Core::BigIntCmp`, or a Rational `Core::RationalCmp`. Each is
/// the SAME op the boolean `<`/`>` operators emit for that type, so the three-way `compare` built over it
/// agrees with those operators (§331) on both backends.
#[derive(Clone, Copy)]
enum CmpKind {
    Scalar,
    Str,
    BigInt,
    Rational,
}

/// Build the runtime three-way `(compare a b)` over two RUNTIME operands of a machine-orderable type as the
/// nested-`if` `if (a < b) Less else if (a > b) Greater else Equal`, yielding the built-in `Ordering` sum (its
/// Less/Equal/Greater discs `lt`/`eq`/`gt`). Reuses the EXISTING boolean ordering op for the operands' kind
/// (`Core::Compare` for a scalar, `Core::StrCmp` for a String/Symbol, `Core::BigIntCmp` for a BigInt,
/// `Core::RationalCmp` for a Rational) — no new runtime op, and the same op the boolean `<`/`>` operators
/// emit, so the two surfaces cannot diverge (§331). Each operand is read twice (the `<` and the `>`), so BOTH
/// are materialized ONCE into a self-keyed `Core::Let` binding read through a shared `Core::LocalRef` — an
/// operand may trap or perform, so re-emitting its computation would double-evaluate it (the runtime-bin-match
/// materialize-once precedent).
fn lower_runtime_compare(
    db: &mut Db,
    id: StructId,
    lhs: StructId,
    rhs: StructId,
    discs: (u32, u32, u32),
    kind: CmpKind,
) -> Core {
    let (lt, eq, gt) = discs;
    let ordering_ty = crate::infer::type_of(db, id);
    let lhs_ty = crate::infer::type_of(db, lhs);
    let rhs_ty = crate::infer::type_of(db, rhs);
    // Materialize each operand once — a shared `LocalRef` the two comparisons both read (keyed by the
    // operand's own occurrence, so the wrapping `Core::Let { (op, op) }` names the computation exactly once).
    db.kept_bindings.insert(lhs);
    db.kept_bindings.insert(rhs);
    let lhs_ref = synth_core(db, Core::LocalRef { binder: lhs }, lhs_ty);
    let rhs_ref = synth_core(db, Core::LocalRef { binder: rhs }, rhs_ty);
    // `a < b` and `a > b` — the existing boolean machine comparison for the operands' kind (a scalar
    // `Core::Compare` the emit widths from the solved scalar type; a `Core::StrCmp` content-lex byte walk;
    // a `Core::BigIntCmp`/`Core::RationalCmp` runtime three-way then compare-with-zero), the same path the
    // `<`/`>` operators lower to for that type.
    let mk_cmp = |db: &mut Db, op: Prim, l: StructId, r: StructId| {
        let core = match kind {
            CmpKind::Scalar => Core::Compare { op, lhs: l, rhs: r },
            CmpKind::Str => Core::StrCmp { op, lhs: l, rhs: r },
            CmpKind::BigInt => Core::BigIntCmp { op, lhs: l, rhs: r },
            CmpKind::Rational => Core::RationalCmp { op, lhs: l, rhs: r },
        };
        synth_core(db, core, crate::ty::Ty::Bool)
    };
    let lt_cmp = mk_cmp(db, Prim::Lt, lhs_ref, rhs_ref);
    let gt_cmp = mk_cmp(db, Prim::Gt, lhs_ref, rhs_ref);
    // The three nullary Ordering variants (`Less`/`Greater`/`Equal`), built at this node's solved Ordering
    // type — the same `Core::SumNew { disc, payloads: [] }` a constant `compare` fold produces.
    let less = synth_core(
        db,
        Core::SumNew {
            disc: lt,
            payloads: Vec::new().into(),
        },
        ordering_ty.clone(),
    );
    let greater = synth_core(
        db,
        Core::SumNew {
            disc: gt,
            payloads: Vec::new().into(),
        },
        ordering_ty.clone(),
    );
    let equal = synth_core(
        db,
        Core::SumNew {
            disc: eq,
            payloads: Vec::new().into(),
        },
        ordering_ty.clone(),
    );
    // `if (a > b) Greater else Equal`, then `if (a < b) Less else <that>`.
    let inner = synth_if(db, gt_cmp, greater, equal);
    let outer = synth_if(db, lt_cmp, less, inner);
    trace!(target: "rcdzc::lower", node = id.0, "compare of runtime operands → nested-if over the boolean ordering op (three-way, materialize-once)");
    Core::Let {
        bindings: vec![(lhs, lhs), (rhs, rhs)].into(),
        body: outer,
    }
}

/// Read a constant float operand's f64-bit payload DEMOTED to the operand's own float width, so a
/// `Float32`-typed constant compares at binary32 precision — mirroring `lower_float_arith`'s width==32
/// `f64::from_bits(bits) as f32 as f64` round, the `ConstFloat` emit (`… as f32`), and the runtime
/// `FloatCompare`/`FEq` grounding. Without this, a `Float32` const fold compares the UN-demoted f64
/// payloads: `(= (: 0.30000001192092896 Float32) (: 0.3 Float32))` folds FALSE though both operands
/// demote to the SAME binary32 (`0x3E99999A`) and the runtime path folds TRUE (adv-61). A `Float64`
/// operand (or an unpinned default-width one) returns the bits unchanged.
fn const_float_bits_at_operand_width(db: &mut Db, operand: StructId, bits: u64) -> u64 {
    let width = match crate::infer::type_of(db, operand) {
        crate::ty::Ty::Float(ft) => ft.ground_width(),
        _ => crate::ty::DEFAULT_FLOAT_WIDTH,
    };
    if width == 32 {
        (f64::from_bits(bits) as f32 as f64).to_bits()
    } else {
        bits
    }
}

fn lower_comparison(db: &mut Db, op: Prim, args: &[StructId]) -> Core {
    if args.len() != 2 {
        return Core::Poison(binop_arity_reject(op, args));
    }
    // A comparison over BIGINT operands routes through the runtime `bigint-cmp` (B3c) — a `BigInt` has no
    // fixed machine slot, so `is_scalar` is false and the plain scalar-compare path below never fires. A
    // CONSTANT pair still folds (both reach as `Core::ConstInt` carrying the exact `IntValue`, compared by
    // `lower_bigint_cmp` at 128-bit precision where it fits, else the runtime op); a runtime operand emits
    // `Core::BigIntCmp` (`bigint-cmp` + a fixed compare-with-zero). A `BigInt`/fixed mix was rejected
    // CDZ0301 in `check_application`, so if one operand is BigInt the other is too. Checked before the
    // constant folds below (a BigInt `ConstInt`'s value can exceed i64, so the `to_i64` fold would decline).
    if bigint_operand(db, args) {
        return lower_bigint_cmp(db, op, args[0], args[1]);
    }
    // A comparison over RATIONAL operands — a constant pair folds by comparing the two normalized
    // rationals exactly (cross-multiply: `a/b <=> c/d` ⇔ `a*d <=> c*b`, denominators strictly positive so
    // the direction is preserved), via `IntValue` bignum; a runtime operand emits `Core::RationalCmp` (the
    // runtime `rational-cmp` op, R3b). Checked before the scalar folds (a Rational is not `is_scalar`).
    if rational_operand(db, args) {
        return lower_rational_cmp(db, op, args[0], args[1]);
    }
    // A comparison over FLOAT operands. Two DISTINCT relations, both realized:
    //  • EQUALITY (`=`) under the CANONICAL BYTE FORM — `nan = nan` true, `-0.0 ≠ 0.0` — a
    //    NaN-canonicalizing BIT compare (NOT IEEE `f64.eq`); the `FEq` float prim.
    //  • ORDERING (`< <= > >=`) under IEEE PARTIAL order (operator ruling 2026-07-16) — a NaN operand
    //    yields false, `-0.0 ==ord +0.0` — the RAW IEEE `f64.lt/le/gt/ge`; the `FLt/FLe/FGt/FGe` prims.
    // The two DISAGREE on NaN + signed zero (bit-equality vs numeric-ordering), by design. A constant pair
    // folds in the `ConstFloat`/`ConstFloatNan` arms below (equality by raw bits; ordering by IEEE
    // partial_cmp, NaN → false — NOT a decline). A RUNTIME operand emits `Core::FloatCompare` carrying the
    // float prim; the backend picks the canonical-bit compare for `FEq` and the raw IEEE op for ordering.
    // Float is NOT `is_scalar` (Int/Bool only), so without this a runtime float compare fell to the
    // compound-heap-walk decline.
    //= spec/capabilities/core-semantics.md#floating-point-equality-follows-the-canonical-byte-form
    //# A floating-point value MUST be equal to another floating-point value exactly when their canonical byte forms are identical, so that a negative zero is distinct from a positive zero and all not-a-number values are equal to one another.
    // Float `< <= > >=` route to the dedicated IEEE-partial-order prims (FLt/FLe/FGt/FGe), NOT the
    // total-order three-way `Compare` path — float is a distinct partial-order facility, not a total-Ord type.
    //= spec/capabilities/numeric-model.md#a-floating-point-relational-operator-follows-the-ieee-partial-order
    //# The floating-point relational operators are the IEEE partial order and are a distinct facility from the total order an orderable type offers under §"Ordering Where Offered Is Total"; a floating-point type MUST NOT be treated as offering that total order, so that the partial order's not-a-number and signed-zero behavior does not contradict the total-order requirement.
    //= spec/capabilities/core-semantics.md#ordering-where-offered-is-total
    //# A floating-point type MUST NOT be treated as offering an ordering in the sense of this section, because its relational operators are the IEEE partial order defined for the floating-point type rather than a total order — so the requirement that an offered ordering be total does not apply to the floating-point relational operators.
    let float_prim = match op {
        Prim::Eq => Some(Prim::FEq),
        Prim::Lt => Some(Prim::FLt),
        Prim::Le => Some(Prim::FLe),
        Prim::Gt => Some(Prim::FGt),
        Prim::Ge => Some(Prim::FGe),
        _ => None,
    };
    if let Some(fprim) = float_prim
        && float_operand(db, args)
    {
        let lhs = core_of(db, args[0]);
        let rhs = core_of(db, args[1]);
        // A constant pair (including NaN / +∞) still folds via the constant arms — defer to them (a
        // non-canonical const has no runtime float value form, so it must fold, not emit a FloatCompare).
        let both_const = matches!(
            (&lhs, &rhs),
            (
                Core::ConstFloat(_) | Core::ConstFloatNan | Core::ConstFloatInf,
                Core::ConstFloat(_) | Core::ConstFloatNan | Core::ConstFloatInf
            )
        );
        if !both_const {
            if let Core::Poison(r) = lhs {
                return Core::Poison(r);
            }
            if let Core::Poison(r) = rhs {
                return Core::Poison(r);
            }
            let width = match crate::infer::type_of(db, args[0]) {
                crate::ty::Ty::Float(ft) => ft.ground_width(),
                _ => crate::ty::DEFAULT_FLOAT_WIDTH,
            };
            trace!(target: "rcdzc::lower", width, ?fprim, "float comparison stays runtime (FloatCompare)");
            return Core::FloatCompare {
                op: fprim,
                lhs: args[0],
                rhs: args[1],
                width,
            };
        }
    }
    let lhs = core_of(db, args[0]);
    let rhs = core_of(db, args[1]);
    match (lhs, rhs) {
        (Core::ConstInt(a), Core::ConstInt(b)) => match (a.to_i64(), b.to_i64()) {
            (Some(x), Some(y)) => {
                let r = compare_ord(op, x.cmp(&y));
                trace!(target: "rcdzc::fold", op = intrinsic_name(op), result = r, "folded constant integer comparison");
                Core::ConstBool(r)
            }
            // An operand exceeds `i64` range — an UNSIGNED value at/above `2^63` (`UInt64.max = 2^64-1`
            // does not fit `i64`). Compare by the TRUE numeric value at 128-bit precision: `to_i128`
            // reads the exact value (any ≤128-bit operand, unsigned or signed), and comparing the true
            // values is correct for BOTH signednesses — an unsigned value is non-negative, a signed one
            // carries its sign, so a naive i64-bit-pattern compare (where `UInt64.max` looks like `-1`)
            // is avoided. This folds `(< (: 0 UInt64) (. UInt64 max))` → true (numeric-model.md §Unsigned
            // Comparison Orders By Magnitude). A value wider than 128 bits (a BigInt) still declines.
            _ => match (a.to_i128(), b.to_i128()) {
                (Some(x), Some(y)) => {
                    let r = compare_ord(op, x.cmp(&y));
                    trace!(target: "rcdzc::fold", op = intrinsic_name(op), result = r, "folded constant integer comparison (i128, wide unsigned)");
                    Core::ConstBool(r)
                }
                _ => Core::Poison(Reject::decline(
                    "comparison of an integer beyond the machine width is not yet folded",
                )),
            },
        },
        (Core::ConstBool(a), Core::ConstBool(b)) => {
            let r = compare_ord(op, a.cmp(&b));
            trace!(target: "rcdzc::fold", op = intrinsic_name(op), result = r, "folded constant boolean comparison");
            Core::ConstBool(r)
        }
        // Two CONSTANT strings compare by their text (lexicographic by Unicode scalar values — the byte
        // order of NFC UTF-8, which the reader already normalized to). `(= "a" "a")` → true; ordering
        // comparisons (`<`) order by text. A constant fold, no heap: the string equality the compiler
        // needs for tag/name dispatch.
        (Core::ConstStr(a), Core::ConstStr(b)) => {
            let r = compare_ord(op, a.cmp(&b));
            trace!(target: "rcdzc::fold", op = intrinsic_name(op), result = r, "folded constant string comparison");
            Core::ConstBool(r)
        }
        // Two CONSTANT chars compare by SCALAR VALUE (`collections-and-text.md` §A Char Is A Single
        // Unicode Scalar Value: "a char's ordering MUST be the numeric order of its scalar value"), so `=`
        // is scalar equality and `<`/`>` order by code point — `(= #\a #\a)` → true, `(< #\a #\b)` → true
        // (97 < 98). A constant fold, no runtime (a char has no machine slot this increment).
        (Core::ConstChar(a), Core::ConstChar(b)) => {
            let r = compare_ord(op, (a as u32).cmp(&(b as u32)));
            trace!(target: "rcdzc::fold", op = intrinsic_name(op), result = r, "folded constant char comparison");
            Core::ConstBool(r)
        }
        // NaN under the CANONICAL BYTE FORM (EQUALITY) vs IEEE PARTIAL order (ORDERING) — the two DISAGREE:
        //  • `=`: every NaN shares one canonical byte form, so `(= nan nan)` is TRUE and `nan` is UNEQUAL to
        //    any finite float (distinct byte forms).
        //  • `< <= > >=`: a NaN operand is UNORDERED, so the relation yields FALSE (operator ruling —
        //    IEEE partialOrd; NaN is neither less, greater, nor equal to anything). It EVALUATES to false,
        //    NOT a decline (a total-order reading would decline, but the operator wants the relational op to
        //    work). Handled BEFORE the finite `ConstFloat` pair. (Signed zero: `(= -0.0 0.0)` is false — the
        //    `ConstFloat` pair carries the sign — while `(<= -0.0 0.0)` is true, equal-under-ordering.)
        //= spec/capabilities/core-semantics.md#floating-point-equality-follows-the-canonical-byte-form
        //# A floating-point value MUST be equal to another floating-point value exactly when their canonical byte forms are identical, so that a negative zero is distinct from a positive zero and all not-a-number values are equal to one another.
        (Core::ConstFloatNan, Core::ConstFloatNan) => {
            if matches!(op, Prim::Eq) {
                trace!(target: "rcdzc::fold", "folded nan = nan → true (canonical byte form)");
                Core::ConstBool(true)
            } else {
                // NaN is unordered → every ordering relation is false.
                trace!(target: "rcdzc::fold", op = intrinsic_name(op), "folded nan <ordering> nan → false (IEEE unordered)");
                Core::ConstBool(false)
            }
        }
        (Core::ConstFloatNan, Core::ConstFloat(_)) | (Core::ConstFloat(_), Core::ConstFloatNan) => {
            if matches!(op, Prim::Eq) {
                trace!(target: "rcdzc::fold", "folded nan = finite → false (distinct byte forms)");
                Core::ConstBool(false)
            } else {
                // A NaN operand makes any ordering relation false (unordered), same as the nan/nan case.
                trace!(target: "rcdzc::fold", op = intrinsic_name(op), "folded nan <ordering> finite → false (IEEE unordered)");
                Core::ConstBool(false)
            }
        }
        // +∞ is a fully-ORDERED float constant (UNLIKE NaN): `+∞ = +∞`, `+∞ > every finite`, and `+∞`
        // is UNORDERED w.r.t. NaN. Its byte-form equality and its IEEE ordering AGREE (a single canonical
        // +∞ bit form that is numerically equal only to itself), so `compare_ord` over the numeric ordering
        // gives the right answer for `=` too — no byte-vs-order divergence here (that only bites NaN and
        // signed zero). Handled BEFORE the finite `ConstFloat` pair, mirroring the NaN arms.
        (Core::ConstFloatInf, Core::ConstFloatInf) => {
            // `+∞` vs `+∞`: equal under both byte form and ordering (`=`/`<=`/`>=` true, `<`/`>` false).
            let r = compare_ord(op, std::cmp::Ordering::Equal);
            trace!(target: "rcdzc::fold", op = intrinsic_name(op), result = r, "folded +inf <cmp> +inf");
            Core::ConstBool(r)
        }
        (Core::ConstFloatInf, Core::ConstFloat(_)) => {
            // `+∞ > x` for every finite `x`: `>`/`>=` true, `=`/`<`/`<=` false.
            let r = compare_ord(op, std::cmp::Ordering::Greater);
            trace!(target: "rcdzc::fold", op = intrinsic_name(op), result = r, "folded +inf <cmp> finite");
            Core::ConstBool(r)
        }
        (Core::ConstFloat(_), Core::ConstFloatInf) => {
            // `x < +∞` for every finite `x`: `<`/`<=` true, `=`/`>`/`>=` false.
            let r = compare_ord(op, std::cmp::Ordering::Less);
            trace!(target: "rcdzc::fold", op = intrinsic_name(op), result = r, "folded finite <cmp> +inf");
            Core::ConstBool(r)
        }
        (Core::ConstFloatInf, Core::ConstFloatNan) | (Core::ConstFloatNan, Core::ConstFloatInf) => {
            // A NaN operand is unordered and byte-unequal to +∞ → every relation is false.
            trace!(target: "rcdzc::fold", op = intrinsic_name(op), "folded +inf <cmp> nan → false (NaN unordered)");
            Core::ConstBool(false)
        }
        // Two CONSTANT floats compare by their canonical value AT THE OPERAND'S OWN FLOAT WIDTH
        // (contracts/deterministic-value-form.md #Numeric Values Serialize Deterministically — floats
        // equal under structural equality share a canonical form, distinct floats have distinct forms) —
        // a `Float32` pair is demoted to binary32 first (`const_float_bits_at_operand_width`), a `Float64`
        // pair stays f64. EQUALITY (`=`) is by RAW BITS at that width, so `-0.0 ≠ 0.0` (distinct bit
        // patterns → the canonical form distinguishes them) and a NaN is unequal to itself. `1e19` and
        // `1e20` round to different doubles → unequal. Ordering (`<`/`>`)
        // uses the IEEE PARTIAL order (`f64::partial_cmp`): an ordered pair folds to the relation's result,
        // an unordered pair (a NaN — but a bare `nan` constant is `ConstFloatNan`, handled above, so this
        // arm's operands are finite) folds to false. Only the fold — no float runtime for a Bool result.
        (Core::ConstFloat(a), Core::ConstFloat(b)) => {
            // Demote each payload to the operand's own float width BEFORE comparing — a `Float32` pair
            // compares at binary32 (matching the runtime `FEq`/ordering + the `ConstFloat` emit), not at
            // the un-demoted f64 payload the reader parsed (adv-61).
            let ba = const_float_bits_at_operand_width(db, args[0], a.to_f64_bits());
            let bb = const_float_bits_at_operand_width(db, args[1], b.to_f64_bits());
            if matches!(op, Prim::Eq) {
                let r = ba == bb;
                trace!(target: "rcdzc::fold", op = intrinsic_name(op), result = r, "folded constant float equality (by canonical bits)");
                Core::ConstBool(r)
            } else {
                // IEEE PARTIAL order (operator ruling): an ordered pair gives the relation. Both operands
                // here are FINITE — a bare `nan` is `ConstFloatNan` (handled above) and constant float
                // arithmetic DECLINES on a non-finite result (`fold_float_arith`), so a NaN never reaches
                // this `(ConstFloat, ConstFloat)` arm; the `partial_cmp → None → false` branch is defensive
                // completeness / parity with the runtime path, not a reachable const case. `partial_cmp`
                // also treats `-0.0`/`+0.0` as EQUAL, so `(<= -0.0 0.0)` is true — the ordering's
                // equal-case, which DISAGREES with the byte-form `=` above (there `-0.0 ≠ 0.0`).
                let r = match f64::from_bits(ba).partial_cmp(&f64::from_bits(bb)) {
                    Some(ord) => compare_ord(op, ord),
                    None => false,
                };
                trace!(target: "rcdzc::fold", op = intrinsic_name(op), result = r, "folded constant float ordering (IEEE partial)");
                Core::ConstBool(r)
            }
        }
        // Two UNIT values — there is exactly ONE unit value, so two units always compare EQUAL. Fold at
        // compile time to the ordering-`Equal` result for the operator (`= unit ()` → true, `< unit ()`
        // → false, `<= unit ()` → true). No heap walk and no runtime op: unit carries no data to
        // compare (it has no machine slot — `valtype_of(Ty::Unit)` is `None`), so `(= unit ())` is not a
        // "compound needs a heap walk" case but a trivial constant. (`unit` and `()` are the same value —
        // core-semantics.md #Unit And The Empty Tuple Are The Same Value.)
        (Core::Unit, Core::Unit) => {
            let r = compare_ord(op, std::cmp::Ordering::Equal);
            trace!(target: "rcdzc::fold", op = intrinsic_name(op), result = r, "folded unit comparison (two units are equal)");
            Core::ConstBool(r)
        }
        (Core::Poison(r), _) | (_, Core::Poison(r)) => Core::Poison(r),
        // A non-constant operand: a runtime comparison IF both operands are scalars (integers or
        // booleans, which have a machine representation the backend can compare); a compound operand
        // still declines (heap-walk equality is a later stage).
        _ => {
            // CONSTANT COMPOUND EQUALITY folds STRUCTURALLY: two values are equal when they have the same
            // type and their contents are equal component-wise. Only for `=` (a total ordering `<`/`>`
            // over compounds is a later stage); only when BOTH operands are compile-time-visible constant
            // compounds (a `SumNew`/`Tuple`/`Record`/`ListNew`, recursively) — a runtime operand still
            // needs the heap walk (`value-eq`/`champ_eq`, deferred to the backend).
            //= spec/capabilities/core-semantics.md#equality-is-structural
            //# Two values MUST be equal when they have the same type and their contents are equal component-wise.
            // This component-wise fold agrees with the canonical byte form: two constant compounds are equal
            // exactly when their canonical forms coincide (a scalar leaf compares by its canonical value, a
            // nested compound recurses), so structural equality and byte-form identity never disagree.
            //= spec/capabilities/core-semantics.md#equality-is-structural
            //# Value equality MUST agree with the canonical byte form, so that two values are equal exactly when their canonical byte forms are identical.
            // `(= (Some 1) (Some 1))` → true, `(= (Some 1) (Some 2))` → false, `(= None None)` → true,
            // `(= (tuple 1 2) (tuple 1 2))` → true. A nested compound compares recursively (a payload/
            // element that is itself a compound). Returns `None` when either side is not a constant
            // compound → falls through to the scalar-runtime / decline below.
            //
            // (A) STRICT heap-collection construction (#5194): `const_compound_eq` can decide the result from
            // an early differing CONSTANT element WITHOUT reading a later one, so a constructed operand whose
            // element can TRAP could fold to `false` (at element 0) and the trapping arg be DROPPED. This is
            // SOUND for a TUPLE/RECORD operand — a reached tuple's later element is UNOBSERVED when the eq
            // short-circuits (§283 laziness; 03-equality "compound equality short-circuits at the first
            // differing element — a later trapping element is not forced" → false, NOT a trap). But a
            // LIST/SET/MAP CONSTRUCTOR is STRICT: ruling (A) forces its element args whenever the ctor is
            // reached (an `=` operand IS reached), independent of any comparison short-circuit (03-equality
            // "list construction strictly evaluates its element arguments … in an = operand" → trap). So
            // DECLINE the fold when an operand IS a heap-COLLECTION ctor with a trappable arg (NOT a
            // tuple/record, which stays lazy) — falling through to the runtime `value-eq` walk that
            // materializes both operands (their list/set/map ctor args evaluate → the trap occurs).
            if matches!(op, Prim::Eq)
                && !is_strict_heap_ctor_with_trappable_arg(db, args[0])
                && !is_strict_heap_ctor_with_trappable_arg(db, args[1])
                && let Some(eq) = const_compound_eq(db, args[0], args[1])
            {
                trace!(target: "rcdzc::fold", result = eq, "folded constant compound equality (structural)");
                return Core::ConstBool(eq);
            }
            // CONSTANT COMPOUND ORDERING (`<`/`<=`/`>`/`>=`) — the §331 companion of the `=` fold just above
            // and of the three-way `Ordering.of` compound fold (`lower_compare`): two fully-constant compounds
            // order through the shared `const_key_order` canonical value order, the SAME order
            // `Set.to-list`/`Map.to-list`/equality use, which mirrors the runtime `value_cmp_shaped` the
            // compound `Core::ValueCmp { op }` walk below uses — so the constant fold and the runtime walk
            // report the SAME relation (the boolean ops and the three-way compare surface one total order two
            // ways that cannot disagree). GUARD: fold only when BOTH operands are fully-constant values. This
            // fold DISCARDS the operands, so a non-constant subterm — e.g. `(Some (/ 1 k))`, whose construction
            // can TRAP or perform — must not be dropped; `const_key_order` can decide the order from an early
            // differing DISCRIMINANT/field WITHOUT reading a deeper payload, so its `Some` is not proof the
            // operands are effect-free (the same effect-safety guard `lower_compare` uses). A float/set/map leaf
            // yields `None` and falls through to the runtime `ValueCmp` walk / float decline below unchanged.
            //
            // ORDERABILITY: fold ONLY a compound the RUNTIME compound walk also orders — `is_orderable_compound`
            // (the blessed leaf vocabulary: Int/Bool/String/Bytes + nested; NOT Char/Float). `const_key_order`
            // is MORE permissive (it orders a Char leaf, for baking a Set/Map to-list at compile time — an
            // enumeration order that never calls a runtime op). But `<`/`Ordering.of` on a Char/Float-leaf
            // compound DECLINES at runtime (03-equality: "a tuple with a Char leaf declines compound ordering"),
            // so folding it here would answer where the runtime refuses — a §331 divergence. Gating on
            // `is_orderable_compound` makes a Char/Float-leaf compound fall through to that decline unchanged.
            if matches!(op, Prim::Lt | Prim::Le | Prim::Gt | Prim::Ge)
                && is_const_value(db, args[0])
                && is_const_value(db, args[1])
                && {
                    let ty = comparison_compound_ty(db, args[0], args[1]);
                    is_orderable_compound(db, &ty)
                }
                && let Some(ord) = const_key_order(db, args[0], args[1])
            {
                let r = compare_ord(op, ord);
                trace!(target: "rcdzc::fold", op = intrinsic_name(op), result = r, "folded constant compound ordering (canonical value order)");
                return Core::ConstBool(r);
            }
            // BOOL-INT EQUALITY: `(= (if c 1 0) K)` / `(= (if c 0 1) K)` with `K` ∈ {0,1} — the `if` is a
            // bool coerced to an int (0/1), so comparing it to 0/1 is just the condition or its negation:
            // `(if c 1 0) == 1` → `c`, `== 0` → `!c`; `(if c 0 1)` is the mirror. Folds away the whole
            // materialize-then-compare (`lt_s ; extend ; const 1 ; eq` → `lt_s`). Only for `Eq`, only a
            // 0/1 constant against a `(if c <1> <0>)`-shaped bool-int (either operand order). Reuses `c`'s
            // occurrence (no synthesis); `!c` is `Core::Not`. Verified value-identical over c ∈ {T,F}.
            if matches!(op, Prim::Eq)
                && let Some(folded) = fold_bool_int_eq(db, args[0], args[1])
            {
                trace!(target: "rcdzc::fold", "(= (if c 1 0) 0/1) folds to c / !c");
                return folded;
            }
            // BOOL-CONST EQUALITY: `(= c true)` → `c`, `(= c false)` → `!c` (either order) — a bool compared
            // to a bool literal is itself / its negation, dropping the `i32.const K ; i32.eq`.
            if matches!(op, Prim::Eq)
                && let Some(folded) = fold_bool_const_eq(db, args[0], args[1])
            {
                trace!(target: "rcdzc::fold", "(= c true/false) folds to c / !c");
                return folded;
            }
            if is_scalar(db, args[0]) && is_scalar(db, args[1]) {
                // SELF-COMPARISON: the two operands are the SAME value (`core_equiv`), so the ordering is
                // fixed regardless of what that value is — `x < x`/`x > x` → false, `x <= x`/`x >= x`/`x =
                // x` → true. Sound ONLY for a TOTAL order and a TRAP-FREE operand: `is_scalar` is Int/Bool
                // (a total order — Float, where `NaN < NaN` etc. is false and `NaN = NaN` is false, is NOT
                // scalar and never reaches here), and the operand must be trap-free since the fold DISCARDS
                // it — `(< (/ a b) (/ a b))` must still trap on b==0 (`core_equiv` matches pure cores, but a
                // matched pure compare/arith can still wrap a trapping `/`). `compare_ord` gives the result
                // for each operator at `Ordering::Equal`.
                if core_equiv(db, args[0], args[1]) && is_trap_free(db, args[0]) {
                    let r = compare_ord(op, std::cmp::Ordering::Equal);
                    trace!(target: "rcdzc::fold", op = intrinsic_name(op), result = r, "self-comparison folds to a constant (x is a total-order scalar)");
                    return Core::ConstBool(r);
                }
                // TYPE-BOUND simplification: a comparison of a runtime integer against a constant AT its
                // own type's min/max is (partly) decidable — `v < min`/`v > max` are unsatisfiable, `v >=
                // min`/`v <= max` are tautologies, and `v <= min`/`v >= max` rewrite to `v == bound` (the
                // backend selects `eqz` when the bound is 0). This subsumes the unsigned-vs-0 case (an
                // unsigned type's `min` is 0) and adds signed narrow (`Int8`'s `[-128,127]`) and full-width
                // bounds. Only fires when the runtime operand's type is fully resolved (Fixed sign+width).
                if let Some(folded) = fold_comparison_at_type_bound(db, op, args[0], args[1]) {
                    return folded;
                }
                trace!(target: "rcdzc::lower", op = intrinsic_name(op), "comparison stays runtime (scalar operands)");
                Core::Compare {
                    op,
                    lhs: args[0],
                    rhs: args[1],
                }
            } else if is_scalar(db, args[0]) || is_scalar(db, args[1]) {
                // ONE-SIDED SCALAR: exactly one operand's type resolves to a scalar Int/Bool while the
                // OTHER reads as an unresolved var (`_`). The type checker UNIFIED the two operands before
                // lowering, so a scalar on one side proves the comparison is scalar — but the unresolved
                // side's `type_of` still renders the raw var (the final substitution is not applied at this
                // query), so `is_scalar && is_scalar` above was false and `=` wrongly fell through to the
                // `value-eq` heap walk below. That mis-lowering surfaced downstream as an ownership decline
                // ("borrowing op operand …") when the scalar side is a bare `ConstInt` (not a heap operand).
                // This is the shape of a comparison over a value projected from a GENERIC-variant payload —
                // `match it (Iter.Mk(s,f) …)` then `(= h 1)` where `h`'s element type is the erased type
                // param `a`: at a concrete instantiation `a := Int64`, `h` IS an Int64 at runtime (the
                // `== 1` unified it), but `type_of(h)` reads the ungrounded var. Route to a scalar
                // `Core::Compare`; the emit's `operand_int_ty` grounds the machine width from the scalar
                // side. Sound because unification guarantees both sides share the type — a scalar side
                // means the other is that same scalar. (v-iterators fused-iterator step; NOT a Perceus/
                // borrow bug — the ownership decline was a symptom of this lowering mis-route.)
                trace!(target: "rcdzc::lower", op = intrinsic_name(op), "one-sided scalar comparison (other operand's var is unified to the scalar) → scalar compare");
                Core::Compare {
                    op,
                    lhs: args[0],
                    rhs: args[1],
                }
            } else if matches!(op, Prim::Eq) && node_ty_is_enum_disc(db, args[0]) {
                // ENUM-DISCRIMINANT equality: both operands are bare discriminant i32s (an all-nullary
                // enum is represented as its discriminant, no heap box), so `=` is a plain `i32.eq` — NOT
                // a `value-eq` heap walk (which would misread a small discriminant as a tagged immediate
                // handle). Route to `Core::Compare`, whose backend emits `i32.eq` for an enum-disc operand
                // (`operand_int_ty` widths it as i32). Only equality — an enum has no order, so `<`/`>`
                // never reach here (they take the `is_scalar` path above, false for a sum, then decline).
                trace!(target: "rcdzc::lower", op = intrinsic_name(op), "enum-discriminant equality → i32.eq compare");
                Core::Compare {
                    op,
                    lhs: args[0],
                    rhs: args[1],
                }
            } else if matches!(op, Prim::Eq) && compound_eq_heap_walkable(db, args[0]) {
                // RUNTIME STRUCTURAL EQUALITY — a `=` on two COMPOUND heap values neither of which folded
                // (a sum/tuple/record built from a parameter or a recursive call). Emit a `value-eq`
                // runtime call (the tagless `champ_eq` walk): equal iff same shape + component-wise equal
                // (core-semantics.md §Equality Is Structural). Restricted to a value whose leaves are all
                // SCALAR (Int/Bool/Unit) — such a value is canonical BY CONSTRUCTION (no embedded RRB
                // vector / CHAMP map / Bytes rope, whose byte form is canonical only after a compaction
                // the compiler would first have to emit), so the walk's result is exact. A compound with a
                // collection/bytes leaf still declines (that canonicalization is a later increment). The
                // type checker already unified the two operands' types before lowering, so a single-side
                // walkability check suffices — both sides share the shape.
                trace!(target: "rcdzc::lower", op = intrinsic_name(op), "runtime structural equality → value-eq heap walk");
                Core::ValueEq {
                    lhs: args[0],
                    rhs: args[1],
                }
            } else if matches!(op, Prim::Eq) && {
                let opnd_ty = crate::infer::type_of(db, args[0]);
                // A BYTES leaf makes the compound take the `ValueEqShaped` path below, NOT this `ValueCmp{op:Eq}`
                // one — even though Bytes is now orderable (so `is_orderable_compound` is true). RATIONALE: a
                // Bytes element may be a runtime rope/SLICE-VIEW, and `value_eq_shaped`'s Bytes arm compares it
                // by `leaf_bytes_eq` (flatten-then-`raw`-compare), the proven-canonicalizing equality the
                // slice-view corpus cases pin; routing a `List<Bytes>` `=` through the ordering walk regressed
                // that (a view element mis-compared). ORDERING (`<` below) still uses `ValueCmp` for Bytes —
                // only EQUALITY prefers the shaped walk, which is exactly what `is_value_eq_shaped_compound`
                // selects. So: take this arm only when orderable AND NOT a value-eq-shaped (Bytes-containing)
                // compound; a Bytes-containing List falls through to the `ValueEqShaped` arm.
                is_orderable_compound(db, &opnd_ty) && !is_value_eq_shaped_compound(db, &opnd_ty)
            } {
                // RUNTIME LIST(-CONTAINING) EQUALITY — a `=` on two compound values that contain a LIST and
                // whose leaves are all ORDERABLE (and none is a Bytes leaf — see the guard above). This is
                // reached ONLY after the `compound_eq_heap_walkable` arm above DECLINED it: a List is an RRB
                // vector that is element-canonical but NOT shape-canonical (a concat-built and a push-built
                // list with the same elements have different internal byte layouts), so `value-eq`'s tagless
                // `champ_eq` byte-walk is UNSOUND for it — `ty_heap_walkable`/`compound_eq_heap_walkable`
                // return false for `Ty::List` by design. Route instead to `Core::ValueCmp { op: Prim::Eq }` —
                // the descriptor-guided element-wise value-cmp walk (which compares a list element-by-element
                // via vec-get, shape-INDEPENDENT), with the emit mapping the three-way result to `res == 0`. So
                // a concat-built `[1,2,3]` `=` a push-built `[1,2,3]` is TRUE (the §331 companion for equality:
                // the same value-cmp walk the boolean `<` uses). A List-FREE orderable compound already took the
                // `ValueEq` arm above (cheaper, and champ_eq IS sound there — no non-canonical RRB spine); only
                // a List-containing (or bare-List) orderable Bytes-FREE type falls through to here. A
                // Bytes-containing List takes `ValueEqShaped` (the guard); a float/Set/Map leaf makes it
                // non-orderable → declines / ValueEqShaped below.
                //= spec/capabilities/collections-and-text.md#a-list-is-an-ordered-homogeneous-sequence
                //# Two lists MUST be equal exactly when they have equal elements in the same order.
                trace!(target: "rcdzc::lower", op = intrinsic_name(op), "runtime list-containing equality → ValueCmp op=Eq (value-cmp == 0, element-wise)");
                Core::ValueCmp {
                    op,
                    lhs: args[0],
                    rhs: args[1],
                    ty: crate::infer::type_of(db, args[0]),
                }
            } else if matches!(op, Prim::Lt | Prim::Le | Prim::Gt | Prim::Ge)
                && operand_is_string_or_symbol(db, args[0])
            {
                // RUNTIME STRING/SYMBOL ORDERING — a `<`/`<=`/`>`/`>=` on two String/Symbol values neither of
                // which folded (built from a parameter / concat / call). A String/Symbol is a UTF-8 byte leaf
                // whose blessed total order is CONTENT-LEXICOGRAPHIC (core-semantics.md §Compound Ordering /
                // 17-symbols §order). Emit `Core::StrCmp` (a byte-lex walk on wasm via bytes-len/bytes-get —
                // HASH-NEUTRAL; native `String` compare on rust). The type checker unified both operands, so a
                // single-side String/Symbol check proves both are. EQUALITY does NOT reach here (the `Eq` arms
                // above route `=` to `ValueEq`); only the four ordering ops. Distinct from the compound-value
                // decline below, which still declines tuple/sum/list ordering (the compound heap walk — a
                // later slice needing the runtime `value-cmp` op).
                trace!(target: "rcdzc::lower", op = intrinsic_name(op), "runtime String/Symbol ordering → StrCmp byte-lexicographic compare");
                Core::StrCmp {
                    op,
                    lhs: args[0],
                    rhs: args[1],
                }
            } else if matches!(op, Prim::Lt | Prim::Le | Prim::Gt | Prim::Ge)
                && node_ty_is_enum_disc(db, args[0])
            {
                // ENUM-DISCRIMINANT ORDERING (breaker #43): an ALL-NULLARY sum (every variant payload-free —
                // a user `(type Tri (Lo)(Mid)(Hi))`, `Sign`, `Ordering` itself) is represented as a BARE i32
                // discriminant with NO heap box (`db.is_enum_disc`). Its blessed order is by discriminant
                // (core-semantics §Compound Ordering Is Lexicographic: "a sum MUST be ordered first by the
                // discriminant"), and since the runtime value IS that discriminant i32, the order is a plain
                // `i32` compare of the two tags — the SAME `Core::Compare` the `=` enum-disc arm uses, just
                // with the ordering `op`. This MUST precede the `is_orderable_compound → ValueCmp` arm below:
                // an all-nullary `Ty::Sum` passes `is_orderable_compound`, but routing it to the runtime
                // `value-cmp` heap walk is a MISCOMPILE — `value-cmp` calls `op_sum_disc` expecting a BOXED
                // sum node, but an enum-disc operand is a bare immediate i32, so it reads disc 0 for both →
                // Equal for distinct variants + `<` false both ways (the #43 soundness bug: compare
                // contradicts `=`, violating §331). A payload-carrying sum's nullary variants still box (they
                // share the sum's heap rep), so they correctly take the `ValueCmp` arm below — only a
                // wholly-nullary sum is a bare disc. `operand_int_ty` widths the compare as i32.
                //= spec/capabilities/core-semantics.md#compound-ordering-is-lexicographic
                //# A sum MUST be ordered first by the discriminant as encoded in its canonical byte form, and then, for two values of the same variant, lexicographically by payload, so that the order agrees with the canonical byte form equality already requires.
                trace!(target: "rcdzc::lower", op = intrinsic_name(op), "enum-discriminant ordering → i32 tag compare (all-nullary sum orders by discriminant)");
                Core::Compare {
                    op,
                    lhs: args[0],
                    rhs: args[1],
                }
            } else if matches!(op, Prim::Lt | Prim::Le | Prim::Gt | Prim::Ge) && {
                let opnd_ty = comparison_compound_ty(db, args[0], args[1]);
                is_orderable_compound(db, &opnd_ty)
            } {
                // RUNTIME COMPOUND ORDERING — a `<`/`<=`/`>`/`>=` on two tuple/record/list/sum values whose
                // leaves are all orderable (core-semantics.md §Compound Ordering Is Lexicographic). Emit
                // `Core::ValueCmp` (the runtime `value-cmp` descriptor-guided three-way walk). The type checker
                // unified both operands, so the single-side orderability check covers both. A float/bytes/set/
                // map leaf makes the compound UN-orderable (`is_orderable_compound` false) → the decline below,
                // matching the spec's carve-outs (§319 float partial; no blessed Bytes/Set/Map order).
                // NOTE an ALL-NULLARY (enum-disc) sum was already handled by the `node_ty_is_enum_disc` arm
                // above (a bare i32 tag compare) — only a BOXED sum (payload-carrying) reaches here.
                //= spec/capabilities/core-semantics.md#compound-ordering-is-lexicographic
                //# A tuple or record MUST be ordered by comparing its components in the same canonical order its equality and canonical byte form use, taking the first component that differs as decisive and comparing equal only when every component compares equal.
                //= spec/capabilities/core-semantics.md#compound-ordering-is-lexicographic
                //# A list MUST be ordered lexicographically: comparing elements from the first, the first differing element is decisive, and a list that is a proper prefix of another MUST compare less than that longer list.
                //= spec/capabilities/core-semantics.md#compound-ordering-is-lexicographic
                //# A sum MUST be ordered first by the discriminant as encoded in its canonical byte form, and then, for two values of the same variant, lexicographically by payload, so that the order agrees with the canonical byte form equality already requires.
                //= spec/capabilities/core-semantics.md#compound-ordering-is-lexicographic
                //# The total order a compound offers MUST agree with its structural equality — two values compare equal under the three-way comparison exactly when they are equal — and MUST be surfaced through the same three-way comparison and boolean ordering operators as any other total order.
                trace!(target: "rcdzc::lower", op = intrinsic_name(op), "runtime compound ordering → ValueCmp (value-cmp heap walk)");
                Core::ValueCmp {
                    op,
                    lhs: args[0],
                    rhs: args[1],
                    ty: comparison_compound_ty(db, args[0], args[1]),
                }
            } else if matches!(op, Prim::Eq) && {
                let opnd_ty = crate::infer::type_of(db, args[0]);
                is_value_eq_shaped_compound(db, &opnd_ty)
            } {
                // RUNTIME LIST(-CONTAINING) EQUALITY WITH A FLOAT/BYTES LEAF — reached ONLY after BOTH the
                // `compound_eq_heap_walkable` `ValueEq` arm (a List is not `champ_eq`-sound — an RRB spine is
                // element- but not shape-canonical) AND the `is_orderable_compound` `ValueCmp{op:Eq}` arm (a
                // float/bytes leaf has no total ORDER) declined. Route to `Core::ValueEqShaped` — the runtime
                // `value-eq-shaped` descriptor-guided element-wise walk: a list compared positionally, a float
                // leaf by canonical byte form (nan==nan, -0.0≠+0.0). So a concat-built `[1.0,2.0]` `=` a
                // push-built `[1.0,2.0]` is TRUE, and `[nan]` `=` `[nan]` is TRUE — the equality companion of
                // the boolean `<` value-cmp path, extended to the float leaf that `<` cannot order.
                //= spec/capabilities/collections-and-text.md#a-list-is-an-ordered-homogeneous-sequence
                //# Two lists MUST be equal exactly when they have equal elements in the same order.
                //= spec/capabilities/core-semantics.md#floating-point-equality-follows-the-canonical-byte-form
                //# A floating-point value MUST be equal to another floating-point value exactly when their canonical byte forms are identical, so that a negative zero is distinct from a positive zero and all not-a-number values are equal to one another.
                trace!(target: "rcdzc::lower", op = intrinsic_name(op), "runtime list-with-float-leaf equality → ValueEqShaped (value-eq-shaped element-wise walk)");
                Core::ValueEqShaped {
                    lhs: args[0],
                    rhs: args[1],
                    ty: crate::infer::type_of(db, args[0]),
                }
            } else if matches!(op, Prim::Lt | Prim::Le | Prim::Gt | Prim::Ge) {
                // An ORDERING op reaching here is a PERMANENT carve-out, not a not-yet: an all-orderable
                // compound already took the `is_orderable_compound → ValueCmp` arm above (and an all-nullary
                // enum the disc-compare arm), so a `<`/`<=`/`>`/`>=` that falls through has an UN-orderable
                // leaf — a float (only the IEEE partial order, §319), or a Set/Map (no blessed order). Naming
                // it "needs a heap walk (not yet built)" MISLEADS (reads as a temporary limit that a later
                // slice lifts); it never will. Mirror the sibling three-way `compare` carve-out (this file's
                // `lower_compare` float/compound arms): name the un-orderable leaf as the reason AND the
                // actionable route — order the orderable components individually. Keeps the shared
                // `COMPOUND_ORDERING_NO_TOTAL_ORDER_DECLINE` substring so the mismatched-type dedup still fires.
                trace!(target: "rcdzc::lower", op = intrinsic_name(op), "decline: ordering of a compound with an un-orderable (float/set/map) leaf — permanent carve-out");
                Core::Poison(Reject::decline(
                    "a compound value with a float, set, or map leaf has no total order, so it cannot be \
                     ordered by `<`/`<=`/`>`/`>=` (a float offers only the IEEE partial order; a set/map \
                     carries no blessed order) — order its orderable components individually",
                ))
            } else {
                // EQUALITY (`=`) reaching here is a genuinely NOT-YET-BUILT canonicalization (a Set/Map leaf
                // whose structural `=` the compiler cannot yet walk) — keep the honest heap-walk decline
                // (the `COMPOUND_COMPARISON_DECLINE` marker, byte-identical: it is corpus-referenced + the
                // mismatched-type dedup key).
                trace!(target: "rcdzc::lower", op = intrinsic_name(op), "decline: equality of a compound value needs a heap walk (not yet built)");
                Core::Poison(Reject::decline(
                    "comparison of a compound value needs a heap walk (not yet built)",
                ))
            }
        }
    }
}

/// If `id` is a boolean coerced to an integer — a `(if cond 1 0)` (the bool itself) or `(if cond 0 1)`
/// (its negation) — return `(cond, negated)`: the underlying boolean condition occurrence and whether the
/// materialized int is the NEGATION of it. `None` otherwise. The shared shape-recognizer for the bool-int
/// folds (a `Core::If` whose two branches are the constants `1` and `0`).
fn materialized_bool(db: &mut Db, id: StructId) -> Option<(StructId, bool)> {
    let Core::If { cond, then_, else_ } = core_of(db, id) else {
        return None;
    };
    let branch_val = |db: &mut Db, b: StructId| -> Option<i64> {
        match core_of(db, b) {
            Core::ConstInt(v) => v.to_i64().filter(|&x| x == 0 || x == 1),
            _ => None,
        }
    };
    match (branch_val(db, then_)?, branch_val(db, else_)?) {
        (1, 0) => Some((cond, false)), // `(if c 1 0)` — c itself
        (0, 1) => Some((cond, true)),  // `(if c 0 1)` — !c
        _ => None,
    }
}

/// Fold `(= <bool-int-if> K)` where `K` ∈ {0,1} and the other operand is a `(if c 1 0)` / `(if c 0 1)`
/// (a boolean coerced to an integer). Returns `c` when the `if`-value equals `K`, `!c` (`Core::Not`) when
/// it is the complement — dropping the materialize-then-compare. Also folds `(= <bool-int> <bool-int>)`
/// (BOTH operands materialized bools) to `(= c d)` / `(= c (not d))`. `None` when neither operand is a 0/1
/// constant, or the other is not a `(if c <one> <zero>)`-shaped bool-int, so the caller keeps the runtime
/// compare. Value-identical: `(if c 1 0)` is `1` iff `c` (so `== 1` is `c`, `== 0` is `!c`); `(if c 0 1)`
/// is the mirror. `c` is trap-free by construction (it was an `if` condition, a Bool) and is REUSED, so
/// no synthesis and no dropped trap. Only `Eq` (equality) — an ordering `<`/`>` against 0/1 is handled by
/// the range fold (`[0,1]` vs a bound).
fn fold_bool_int_eq(db: &mut Db, lhs: StructId, rhs: StructId) -> Option<Core> {
    // (the bool-int `if` node, the 0/1 constant K) in either operand order.
    let as_const01 = |db: &mut Db, id: StructId| -> Option<i64> {
        match core_of(db, id) {
            Core::ConstInt(v) => v.to_i64().filter(|&k| k == 0 || k == 1),
            _ => None,
        }
    };
    // BOTH operands bool-materialized ints — `(= (if c 1 0) (if d 1 0))` is `(= c d)`, dropping two
    // `extend_i32_u`s and the i64 compare for a direct bool `i32.eq`. Each `(if _ 1 0)`/`(if _ 0 1)` maps
    // to its underlying bool (or its negation); the equality of two 0/1 values is the equality of the
    // bools, XOR-adjusted for each negation: `(= biᶜ biᵈ)` = `c == d` when neither/both negated, else `c
    // != d` (i.e. `= c (not d)`). Built as a `Core::Compare{Eq}` on the two bool operands, one wrapped in
    // `Core::Not` when the polarities differ (synth so the negation folds — `(not (< …))` → the complement
    // op). Tried before the const-K path since a bool-int is not a 0/1 constant.
    if let Some((cc, cneg)) = materialized_bool(db, lhs)
        && let Some((dd, dneg)) = materialized_bool(db, rhs)
    {
        // Equal iff the bools agree after accounting for each side's negation. Same net polarity → `(= c
        // d)`; opposite → `(= c (not d))`. Reuse `cc` as the left bool; the right is `dd` or `(not dd)`.
        let right = if cneg == dneg {
            dd
        } else {
            synth_core(db, Core::Not { operand: dd }, crate::ty::Ty::Bool)
        };
        return Some(Core::Compare {
            op: Prim::Eq,
            lhs: cc,
            rhs: right,
        });
    }
    let (if_node, k) = if let Some(k) = as_const01(db, rhs) {
        (lhs, k)
    } else if let Some(k) = as_const01(db, lhs) {
        (rhs, k)
    } else {
        return None;
    };
    // The `if` must be `(if c <then> <else>)` with the branches the integer constants 1 and 0.
    let Core::If { cond, then_, else_ } = core_of(db, if_node) else {
        return None;
    };
    let branch_val = |db: &mut Db, b: StructId| -> Option<i64> {
        match core_of(db, b) {
            Core::ConstInt(v) => v.to_i64().filter(|&x| x == 0 || x == 1),
            _ => None,
        }
    };
    let (t, e) = (branch_val(db, then_)?, branch_val(db, else_)?);
    // `then`/`else` must be the two distinct values {1,0}. The `if`-value equals `t` when `c`, else `e`.
    if !((t == 1 && e == 0) || (t == 0 && e == 1)) {
        return None;
    }
    // `(if c t e) == k`: true exactly when the SELECTED branch equals k. Since one branch is 1 and the
    // other 0, `== k` picks out the condition polarity: it holds under `c` iff `t == k`.
    // t==k → the value equals k precisely when c holds → fold to `c`; else → `!c`.
    if t == k {
        Some(core_of(db, cond)) // (= (if c 1 0) 1) → c ;  (= (if c 0 1) 0) → c
    } else {
        Some(Core::Not { operand: cond }) // (= (if c 1 0) 0) → !c ; (= (if c 0 1) 1) → !c
    }
}

/// Fold a BOOLEAN EQUALITY against a boolean LITERAL: `(= c true)` → `c`, `(= c false)` → `(not c)` (and
/// the mirrored operand order). A boolean compared to a constant boolean IS that boolean (compared to
/// `true`) or its negation (compared to `false`) — dropping the redundant `i32.const K ; i32.eq` the
/// runtime `Core::Compare` would emit. Returns the runtime operand's core (`== true`) or `Core::Not` of it
/// (`== false`); `None` unless exactly one operand is a constant `Bool` and the OTHER a runtime `Bool`
/// (a `ConstBool`/`ConstBool` pair already folded in the caller's earlier arm; a NON-`Bool` operand is not
/// this fold). The runtime operand is REUSED (its evaluation/traps preserved — no synthesis, no dropped
/// trap; the discarded operand is a constant, trivially trap-free). Only `Eq` — a `Bool` has no order, so
/// `<`/`>` never reach here (the caller routes them past `is_scalar` then declines for a non-total order).
fn fold_bool_const_eq(db: &mut Db, lhs: StructId, rhs: StructId) -> Option<Core> {
    // One side a constant bool `k`, the other a RUNTIME bool `v` (not itself a constant — a const/const
    // pair folded earlier). `v` must be Bool-typed so the result is the operand as-is / negated.
    let as_const_bool = |db: &mut Db, id: StructId| match core_of(db, id) {
        Core::ConstBool(b) => Some(b),
        _ => None,
    };
    let (v, k) = if let Some(k) = as_const_bool(db, rhs) {
        (lhs, k)
    } else if let Some(k) = as_const_bool(db, lhs) {
        (rhs, k)
    } else {
        return None;
    };
    // The runtime operand must be a Bool (its machine value is the 0/1 the fold returns directly). A
    // constant `v` would already have folded via the caller's `ConstBool`/`ConstBool` arm.
    if !matches!(crate::infer::type_of(db, v), crate::ty::Ty::Bool) {
        return None;
    }
    if k {
        Some(core_of(db, v)) // (= c true) → c
    } else {
        Some(Core::Not { operand: v }) // (= c false) → !c
    }
}

/// The inclusive bounds a resolved integer type occupies, as `(min, max)` where each is `Some` only if it
/// fits an `i64`. Returns `None` (skip the whole type) if the sign or width is not yet `Fixed` (a
/// deferred/variable operand must NOT be folded — its bounds are a guess, and a deferred sign grounds to
/// SIGNED where the range differs). Signed `N` holds `[-2^(N-1), 2^(N-1)-1]` (both fit i64 for `N <= 64`);
/// unsigned `N` holds `[0, 2^N - 1]` — the min `0` always fits, but the max `2^64 - 1` at `N == 64` does
/// NOT, so that bound is `None` (a comparison against it stays a runtime compare, out of i64 reach here).
pub(crate) fn resolved_int_bounds(it: crate::ty::IntTy) -> Option<(Option<i64>, Option<i64>)> {
    let crate::ty::Sign::Fixed(signed) = it.sign else {
        return None;
    };
    let crate::ty::Width::Fixed(w) = it.width else {
        return None;
    };
    if signed {
        let half = 1i64 << (w - 1); // w <= 64, so 2^(w-1) <= 2^63 fits i64 (as `1<<63` = i64::MIN's magnitude)
        Some((Some(half.wrapping_neg()), Some(half.wrapping_sub(1)))) // w=64: (i64::MIN, i64::MAX)
    } else if w >= 64 {
        Some((Some(0), None)) // unsigned 64: min 0 folds; max 2^64-1 is not i64-representable → None
    } else {
        // unsigned `w < 64`: max is `2^w − 1`, which fits i64. At `w == 63` the shift `1i64 << 63` is
        // `i64::MIN`, so `− 1` would OVERFLOW in a checked build — `2^63 − 1` IS `i64::MAX`, so use it
        // directly (a `UInt63`'s max); every `w ≤ 62` uses the shift.
        let max = if w == 63 { i64::MAX } else { (1i64 << w) - 1 };
        Some((Some(0), Some(max)))
    }
}

/// Simplify an ordering comparison of a runtime scalar against a constant that sits at (or beyond) the
/// operand's OWN type bound, exploiting the type's domain `[min, max]`:
///  - `v < min` → `false`, `v >= min` → `true`, `v <= min` → `v == min`;
///  - `v > max` → `false`, `v <= max` → `true`, `v >= max` → `v == max`.
///
/// This subsumes the unsigned-vs-0 case (`min = 0` for an unsigned type) and adds signed narrow bounds
/// (`Int8`'s `[-128,127]`) and full-width (`Int64`'s `[i64::MIN, i64::MAX]`). `v < max` / `v > min` are
/// NOT decidable (the value can be anywhere in range) and are left as the native compare. The `v == bound`
/// rewrite emits a `Core::Compare Eq`, which the backend selects to `eqz` when the bound is 0.
///
/// Returns `None` unless exactly one operand is a constant EQUAL to the other operand's resolved-type min
/// or max (a `Sign::Fixed` + `Width::Fixed` integer — a deferred/variable type is not folded). `Prim::Eq`
/// is excluded (equality against a bound is not a tautology and its `= 0` is already the `eqz` peephole).
fn fold_comparison_at_type_bound(
    db: &mut Db,
    op: Prim,
    lhs: StructId,
    rhs: StructId,
) -> Option<Core> {
    let const_val = |db: &mut Db, id: StructId| match core_of(db, id) {
        Core::ConstInt(v) => v.to_i64(),
        _ => None,
    };
    // Identify (runtime operand `v`, its `(min, max)` range, the constant `c`, whether `v` is on the
    // LEFT). Exactly one side must be a constant and the OTHER a runtime value with a known range.
    let (v, (min, max), c, v_on_left) =
        if let (Some(c), Some(b)) = (const_val(db, rhs), value_range(db, lhs)) {
            (lhs, b, c, true) // `(op v c)`
        } else if let (Some(c), Some(b)) = (const_val(db, lhs), value_range(db, rhs)) {
            (rhs, b, c, false) // `(op c v)`
        } else {
            return None;
        };
    // EQUALITY against a value with a known range is DECIDABLE when `c` lies OUTSIDE `[min, max]` (no
    // value in the range equals `c` → `false`) or the range pins `v` to the single point `{c}` (`v` can
    // only be `c` → `true`). `=` is symmetric, so `v_on_left` is immaterial. This subsumes the classic
    // `(= (& x 15) 100)` → false (`x & 15 ∈ [0,15]`, `100` above). Discards `v`, so — like the ordering
    // rules below — only fires when `v` is TRAP-FREE (a trapping operand must keep its runtime compare so
    // the trap survives). The single-point `true` case rarely fires at lower time (a `[c,c]` range comes
    // from a `ConstInt`, already folded), but is sound and DOES fire via the refinement sibling
    // (`refined_comparison_const`, a match arm pinning the scrutinee to `{c}`).
    if matches!(op, Prim::Eq) {
        let outside = c < min || max.is_some_and(|m| c > m);
        let pinned = min == c && max == Some(c);
        if (outside || pinned) && is_trap_free(db, v) {
            trace!(target: "rcdzc::fold", node = v.0, c, min, ?max, result = pinned, "range-vs-constant equality is decidable — folds to a constant");
            return Some(Core::ConstBool(pinned));
        }
        return None;
    }
    // Normalize to the `(cmp v c)` sense — the LEFT-const forms are the mirror (`c < v` ≡ `v > c`).
    let cmp = match (op, v_on_left) {
        (Prim::Lt, true) | (Prim::Gt, false) => Prim::Lt,
        (Prim::Gt, true) | (Prim::Lt, false) => Prim::Gt,
        (Prim::Le, true) | (Prim::Ge, false) => Prim::Le,
        (Prim::Ge, true) | (Prim::Le, false) => Prim::Ge,
        _ => return None,
    };
    // The constant occurrence to reuse as the rhs of a rewritten `v == c` (keeps its width grounding).
    let c_occ = if v_on_left { rhs } else { lhs };
    // A tautology/unsatisfiable comparison folds to a CONSTANT — but that DISCARDS the runtime operand
    // `v`, so a TRAPPING operand (`(/ 10 z)` with z==0, an overflowing `(+ x x)`) would lose its trap and
    // the program would run to the folded bool instead of trapping. Only fold when `v` is TRAP-FREE; a
    // possibly-trapping operand keeps the runtime `Core::Compare` (returning `None`), which evaluates `v`
    // and traps exactly as a genuine comparison does. This mirrors the self-comparison fold's
    // `is_trap_free` guard above (which likewise refuses to drop a trapping operand). The `v == bound`
    // rewrite (`eq_bound`) is unaffected — it KEEPS `v` as an operand, so its trap is preserved.
    let operand_trap_free = is_trap_free(db, v);
    let const_bool = |r: bool, why: &str| {
        if !operand_trap_free {
            trace!(target: "rcdzc::fold", node = v.0, why, "range-vs-constant comparison is decidable but the operand may trap — keep the runtime compare to preserve the trap");
            return None;
        }
        trace!(target: "rcdzc::fold", node = v.0, why, "range-vs-constant comparison folds to a constant");
        Some(Core::ConstBool(r))
    };
    let eq_bound = |why: &str| {
        trace!(target: "rcdzc::fold", node = v.0, why, "range-vs-constant comparison folds to `== bound`");
        Some(Core::Compare {
            op: Prim::Eq,
            lhs: v,
            rhs: c_occ,
        })
    };
    // `v ∈ [min, max]` (`max` is `None` when the upper bound is not i64-representable — an unsigned-64
    // value, `[0, 2^64)`). A comparison against `c` is DECIDABLE when the whole range lies on one side;
    // the boundary cases (`c` exactly at `min`/`max`) collapse `<=`/`>=` to an equality test (which the
    // backend selects to `eqz` when the bound is 0). Rules verified exhaustively. A rule that references
    // `max` fires only when `max` is known.
    let above_max = |c: i64| max.is_some_and(|m| c > m);
    let at_or_above_max = |c: i64| max.is_some_and(|m| c >= m);
    let at_max = |c: i64| max == Some(c);
    match cmp {
        Prim::Lt if above_max(c) => const_bool(true, "v < c: c > max"), // every v < c
        Prim::Lt if c <= min => const_bool(false, "v < c: c <= min"),   // no v < c
        Prim::Ge if above_max(c) => const_bool(false, "v >= c: c > max"),
        Prim::Ge if c <= min => const_bool(true, "v >= c: c <= min"),
        Prim::Le if at_or_above_max(c) => const_bool(true, "v <= c: c >= max"),
        Prim::Le if c < min => const_bool(false, "v <= c: c < min"),
        Prim::Le if c == min => eq_bound("v <= min ⇔ v == min"),
        Prim::Gt if at_or_above_max(c) => const_bool(false, "v > c: c >= max"),
        Prim::Gt if c < min => const_bool(true, "v > c: c < min"),
        Prim::Ge if at_max(c) => eq_bound("v >= max ⇔ v == max"),
        // A constant strictly inside the range (and not at a collapsing boundary) — not decidable.
        _ => None,
    }
}

/// Decide an ORDERING comparison `(op a b)` purely from the two operands' inclusive ranges `[alo, ahi]`
/// and `[blo, bhi]` (`hi` is `None` when unbounded above — an unsigned-64 value). Returns `Some(true)`/
/// `Some(false)` only when the ranges are DISJOINT enough that the operator's result is the same for
/// every `(a, b)` in them; `None` when the ranges overlap so the result depends on the runtime values.
/// The range-vs-range companion of the constant-vs-range fold, consumed by `refined_comparison_const` at
/// emit time (where the flow-refinement stack lands two runtime vars in disjoint intervals) — e.g. under
/// `a ∈ [101,…]`, `b ∈ […,49]` ⟹ `a > b` always, `a < b` never. `Eq` is intentionally NOT decided here (an
/// ordering-only helper, mirroring the `Eq`-excludes structure of the constant folds). Reasoning
/// (for `<`): TRUE for all iff even the largest `a` is below the smallest `b` (`ahi < blo`); FALSE for all
/// iff even the smallest `a` is not below the largest `b` (`alo >= bhi`, needs both bounds). The other
/// operators are the standard rewrites of `<`. A missing bound (`None`) simply fails the guard that needs
/// it (leaving the comparison runtime), never fabricating a decision.
fn compare_disjoint_ranges(
    op: Prim,
    (alo, ahi): (i64, Option<i64>),
    (blo, bhi): (i64, Option<i64>),
) -> Option<bool> {
    // `a < b` for ALL pairs iff `max(a) < min(b)`; NEVER iff `min(a) >= max(b)`.
    let lt_always = || ahi.is_some_and(|ah| ah < blo);
    let lt_never = || bhi.is_some_and(|bh| alo >= bh);
    // `a > b` for ALL pairs iff `min(a) > max(b)`; NEVER iff `max(a) <= min(b)`.
    let gt_always = || bhi.is_some_and(|bh| alo > bh);
    let gt_never = || ahi.is_some_and(|ah| ah <= blo);
    match op {
        Prim::Lt if lt_always() => Some(true),
        Prim::Lt if lt_never() => Some(false),
        Prim::Ge if lt_always() => Some(false), // `>=` is `!(<)`
        Prim::Ge if lt_never() => Some(true),
        Prim::Gt if gt_always() => Some(true),
        Prim::Gt if gt_never() => Some(false),
        Prim::Le if gt_always() => Some(false), // `<=` is `!(>)`
        Prim::Le if gt_never() => Some(true),
        _ => None,
    }
}

/// EMIT-TIME comparison fold against the CURRENT flow-sensitive refinement (cycle 82's branch-range
/// stack). When one operand is a compile-time constant `c` and the other a runtime value whose refined
/// `value_range` lies ENTIRELY on one side of `c`, the comparison is decidable to a constant bool — a
/// redundant re-test the enclosing branch already established (`(if (> n 0) (if (> n 0) …) …)`, or an
/// IMPLIED test `(if (>= n 5) (if (> n 0) …) …)`). This is the sibling of [`fold_comparison_at_type_bound`]
/// but is called from the `Core::Compare` EMIT arm (not `lower`), because refinements are populated only
/// during emit — `lower` runs with the stack empty and cannot see branch facts. Returns only the
/// CONSTANT-bool verdicts (not the `== bound` collapse, which needs a synthesized node `lower` owns).
///
/// SOUND: the refined range is a fact the branch GUARANTEES; folding the comparison discards the runtime
/// operand, so — exactly as `fold_comparison_at_type_bound` — it only fires when that operand is
/// `is_trap_free` (a refined variable is a `Param`/`LocalRef`, trap-free), never dropping a trap.
pub(crate) fn refined_comparison_const(
    db: &mut Db,
    op: Prim,
    lhs: StructId,
    rhs: StructId,
) -> Option<bool> {
    // Whether `id` is a variable currently carrying an active flow refinement — the emit-only fact that
    // `lower` could not see. At least one operand must be refined for a fold here to add value beyond the
    // const-fold tier's declared-type reasoning.
    let is_refined = |db: &mut Db, id: StructId| {
        matches!(
            core_of(db, id),
            Core::Param { binder } | Core::LocalRef { binder } if db.refined_range(binder).is_some()
        )
    };
    // RANGE-VS-RANGE (both operands runtime): when NEITHER operand is constant but their refined ranges
    // are disjoint enough to decide the ordering, fold to the constant — a flow-sensitive comparison
    // elimination (`(if (> a 100) (if (< b 50) (< b a) …))` → the inner `(< b a)` is always true, since
    // `a ∈ [101,…]` and `b ∈ […,49]`). This lives ONLY at emit time: at lower time, a disjoint positive
    // range needs either a constant operand (handled by the const-vs-range fold) or a checked arith op
    // (not trap-free, so the discarding fold declines) — the refinement stack is what makes two runtime
    // variables land in disjoint intervals. Only fires when at least one operand is genuinely REFINED and
    // BOTH are trap-free (the fold discards both). `Eq` is not handled here (an ordering-only helper,
    // matching the const-fold structure). Attempted BEFORE the const-operand path so a `(< a b)` where
    // both are refined vars is decided; a const operand has no refinement and falls through below.
    if !matches!(op, Prim::Eq)
        && let (Some(ra), Some(rb)) = (value_range(db, lhs), value_range(db, rhs))
        && let Some(r) = compare_disjoint_ranges(op, ra, rb)
        && (is_refined(db, lhs) || is_refined(db, rhs))
        && is_trap_free(db, lhs)
        && is_trap_free(db, rhs)
    {
        return Some(r);
    }
    let const_val = |db: &mut Db, id: StructId| match core_of(db, id) {
        Core::ConstInt(v) => v.to_i64(),
        _ => None,
    };
    // (runtime operand `v`, its `[min, max]` refined range, the constant `c`, whether `v` is on the left).
    let (v, (min, max), c, v_on_left) =
        if let (Some(c), Some(b)) = (const_val(db, rhs), value_range(db, lhs)) {
            (lhs, b, c, true)
        } else if let (Some(c), Some(b)) = (const_val(db, lhs), value_range(db, rhs)) {
            (rhs, b, c, false)
        } else {
            return None;
        };
    // The fold must ADD VALUE beyond the const-fold tier (a bare declared-type range already folds in
    // `lower` via `fold_comparison_at_type_bound`) AND be SOUND to discard `v` (folding drops it, so `v`
    // must carry no reachable trap). Two operand shapes qualify:
    //   (1) a genuinely REFINED variable (`Param`/`LocalRef` with an active `refined_range`) — trap-free,
    //       and its range reflects the branch fact `lower` could not see; the original case.
    //   (2) a checked-arith node over such a refined operand whose result PROVABLY cannot overflow here
    //       (`(+ x 1)` under `x ∈ [0,9]`): `value_range(v)` already composed the refinement through
    //       `arith_range`, so it can decide the comparison — AND because it is `provably_no_overflow` in
    //       this flow context, discarding it drops NO reachable trap (a checked `+`/`-`/`*` is otherwise
    //       possibly-trapping, so `is_trap_free` rejects it; the overflow proof is what makes it
    //       discardable-in-context). A refined variable operand is what carries the branch fact into the
    //       arith range, so require one — else this only re-derives the declared-type range `lower` handled.
    let refined_var = |db: &mut Db, id: StructId| {
        matches!(
            core_of(db, id),
            Core::Param { binder } | Core::LocalRef { binder } if db.refined_range(binder).is_some()
        )
    };
    // `v` is DISCARDABLE-AND-VALUE-ADDING here iff it is a refined variable, OR a checked-arith node that
    // (a) reads a refined variable (so the fold sees a branch fact) and (b) is provably-no-overflow (so the
    // discard is trap-safe). Both are then safe to drop when the comparison folds.
    let discardable = if refined_var(db, v) {
        true
    } else if let Core::Arith {
        op: aop,
        lhs: al,
        rhs: ar,
    } = core_of(db, v)
        && matches!(aop, Prim::Add | Prim::Sub | Prim::Mul)
        && (refined_var(db, al) || refined_var(db, ar))
        && is_trap_free(db, al)
        && is_trap_free(db, ar)
        && let crate::ty::Ty::Int(vit) = crate::infer::type_of(db, v)
    {
        // Trap-free-IN-CONTEXT: a checked `+`/`-`/`*` is discardable only when the flow facts PROVE it
        // cannot overflow (so no reachable trap is dropped). Reuses the both-backend overflow-elision oracle.
        let grounded = crate::ty::IntTy::fixed(vit.ground_signed(), vit.ground_width());
        provably_no_overflow(db, aop, al, ar, grounded, v)
    } else {
        false
    };
    if !discardable {
        return None;
    }
    // EQUALITY against the refined range: DECIDABLE when `c` lies OUTSIDE `[min, max]` (→ false) or the
    // range PINS `v` to `{c}` (→ true). The pin case is the payoff a match arm's exact-value refinement
    // enables — `refined_frame_for_match_arm` sets the scrutinee to `[c, c]`, so a redundant `(= n 5)`
    // inside the `(5 …)` arm folds to `true`; the outside case folds a re-test the enclosing branch's
    // interval already excludes (`(if (> n 10) (= n 3) …)` → `false`). `=` is symmetric — `v_on_left` is
    // immaterial. Sibling of the ordering rules below.
    if matches!(op, Prim::Eq) {
        let outside = c < min || max.is_some_and(|m| c > m);
        let pinned = min == c && max == Some(c);
        return if outside {
            Some(false)
        } else if pinned {
            Some(true)
        } else {
            None
        };
    }
    // Normalize to `(cmp v c)`.
    let cmp = match (op, v_on_left) {
        (Prim::Lt, true) | (Prim::Gt, false) => Prim::Lt,
        (Prim::Gt, true) | (Prim::Lt, false) => Prim::Gt,
        (Prim::Le, true) | (Prim::Ge, false) => Prim::Le,
        (Prim::Ge, true) | (Prim::Le, false) => Prim::Ge,
        _ => return None,
    };
    // `v ∈ [min, max]`. Decide only when the WHOLE range lies on one side of `c`.
    let above_max = |c: i64| max.is_some_and(|m| c > m); // c strictly above the range → v < c always
    let at_or_above_max = |c: i64| max.is_some_and(|m| c >= m);
    match cmp {
        Prim::Lt if above_max(c) => Some(true), // v < c: c > max → always
        Prim::Lt if c <= min => Some(false),    // v < c: c <= min → never
        Prim::Ge if above_max(c) => Some(false), // v >= c: c > max → never
        Prim::Ge if c <= min => Some(true),     // v >= c: c <= min → always
        Prim::Le if at_or_above_max(c) => Some(true), // v <= c: c >= max → always
        Prim::Le if c < min => Some(false),     // v <= c: c < min → never
        Prim::Gt if at_or_above_max(c) => Some(false), // v > c: c >= max → never
        Prim::Gt if c < min => Some(true),      // v > c: c < min → always
        _ => None,
    }
}

/// The inclusive range a runtime value provably occupies, as `(min, max)` where `min: i64` is always
/// known and `max: Option<i64>` is absent when the upper bound is not i64-representable (an unsigned-64
/// value spans `[0, 2^64)` — min 0, no i64 max). `None` when no bound is known at all. Prefers the DERIVED
/// range from `unsigned_value_bits` (a nonnegative value with a known significant-bit count `B` →
/// `[0, 2^B − 1]`, tighter than its type) and falls back to the value's declared-type bounds. Feeds the
/// range-vs-constant comparison fold.
/// The value range of `id`, MEMOIZED when refinement-free. `value_range_uncached` recurses `LocalRef →
/// initializer`, so over a sequential-dependency chain (`(x_i (+ x_{i-1} p))`) it re-walked every
/// predecessor per node — O(N²) compile-time (a `--warm-only` probe measured near-quadratic growth). The
/// analysis is a PURE function of the node's core EXCEPT when a flow-sensitive refinement frame is active
/// (`db.range_refinements` non-empty), where a refined binder narrows the range for the branch being
/// emitted. So: cache/serve ONLY when the refinement stack is empty (the const-fold callers + the common
/// top-level emit path); when a frame is pushed (inside a guard branch), compute FRESH and never touch the
/// cache — the cache therefore only ever holds refinement-free results and is only served refinement-free.
/// Sound: with no frame the result depends only on the (memoized, stable) node core + type, so a cached
/// entry stays valid across every subsequent refinement-free query. O(N²) → O(N), behavior-identical.
fn value_range(db: &mut Db, id: StructId) -> Option<(i64, Option<i64>)> {
    if db.range_refinements.is_empty()
        && let Some(&cached) = db.value_range_memo.get(&id)
    {
        return cached;
    }
    let r = value_range_uncached(db, id);
    if db.range_refinements.is_empty() {
        db.value_range_memo.insert(id, r);
    }
    r
}

fn value_range_uncached(db: &mut Db, id: StructId) -> Option<(i64, Option<i64>)> {
    // Test-only compile-cost counter (pins the O(N²)→O(N) memo — see `value_range` + the regression guard).
    #[cfg(test)]
    {
        db.value_range_uncached_calls += 1;
    }
    // A CONSTANT's range is exactly itself — `[v, v]` (the tightest possible).
    if let Core::ConstInt(v) = core_of(db, id)
        && let Some(v) = v.to_i64()
    {
        return Some((v, Some(v)));
    }
    // A VARIABLE reference (a parameter or a kept `let`-binding) MAY carry a flow-sensitive REFINEMENT: a
    // range known to hold in the branch currently being emitted (`n : Int64` refined to `[2, MAX]` inside
    // the else-branch of `(< n 2)`). When present, INTERSECT it with the declared-type bounds — the
    // tightest sound range — so a guard-elision check sees the narrowed range and drops a dead overflow
    // guard (`(- n 1)` under `n ≥ 2` cannot underflow). Refinements are EMIT-ONLY (the const-fold callers
    // run with the stack empty, so this is a no-op there); the `value_range` WRAPPER only memoizes when the
    // refinement stack is EMPTY, so this refinement-active path is never cached and a transient refinement
    // cannot poison any cached result. When no refinement applies, falls through to the ordinary arith/type
    // range below.
    if let Core::Param { binder } | Core::LocalRef { binder } = core_of(db, id)
        && let Some((rlo, rhi)) = db.refined_range(binder)
    {
        // Intersect with the declared-type bounds (a refinement only NARROWS a real value).
        let type_range = match crate::infer::type_of(db, id) {
            crate::ty::Ty::Int(it) => {
                resolved_int_bounds(it).and_then(|(lo, hi)| lo.map(|lo| (lo, hi)))
            }
            _ => None,
        };
        let (tlo, thi) = type_range.unwrap_or((i64::MIN, Some(i64::MAX)));
        let lo = rlo.max(tlo);
        let hi = match (rhi, thi) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) => Some(a),
            (None, b) => b,
        };
        return Some((lo, hi));
    }
    // A kept `let`-binding reference (`Core::LocalRef { binder }`, no active refinement) carries the range
    // of its INITIALIZER — `binder` IS the initializer's occurrence (see `lower_let`). So a multi-use
    // masked binding `(let ((y (& x 255))) (+ y y))` propagates `[0,255]` through `y`, letting `(+ y y)`
    // shed its overflow guard exactly as the inlined `(+ (& x 255) (& x 255))` does. The initializer is a
    // distinct, earlier node (a binding never references itself), so the recursion bottoms out. No
    // refinement applies here (the block above returned early if one did).
    if let Core::LocalRef { binder } = core_of(db, id)
        && let Some(r) = value_range(db, binder)
    {
        return Some(r);
    }
    // An ARITHMETIC / BITWISE / SHIFT node's range PROPAGATES from its operands' ranges — this is the
    // dataflow layer that lets a bounded sub-expression bound its enclosing op (`(+ (& x 15) (& y 15))`
    // → [0,30], and a further `(+ … (& z 15))` → [0,45]). All interval math is in `i128` so endpoints
    // never wrap; the result is clamped back to `i64` range (a bound outside i64 becomes "unbounded" on
    // that side). A `None` operand range makes the whole node unbounded (falls through to the type).
    if let Core::Arith { op, lhs, rhs } = core_of(db, id)
        && let Some(r) = arith_range(db, op, lhs, rhs)
    {
        return Some(r);
    }
    // A CONDITIONAL's range is the UNION of its branches' ranges — the value IS one branch's value, so it
    // lies within `[min(lo), max(hi)]`. This bounds a bool-materialized `(if c 1 0)` to `[0,1]`, so a
    // downstream `(* bool C)` / `(+ bool C)` sheds its overflow guard (`bool*C ∈ [0,C]` can't overflow),
    // and any small-constant conditional (`(if c 10 20)` → [10,20]) bounds its consumer. Both branches
    // must have a known range (an unbounded branch makes the union unbounded → `None`). A `None` upper on
    // EITHER branch makes the union's upper `None` (unbounded above). Match arms union the same way.
    let branch_union = |db: &mut Db, branches: &[StructId]| -> Option<(i64, Option<i64>)> {
        let mut lo = i64::MAX;
        let mut hi: Option<i64> = Some(i64::MIN);
        for &b in branches {
            let (blo, bhi) = value_range(db, b)?;
            lo = lo.min(blo);
            hi = match (hi, bhi) {
                (Some(h), Some(bh)) => Some(h.max(bh)),
                _ => None, // an unbounded-above branch makes the union unbounded above
            };
        }
        Some((lo, hi))
    };
    match core_of(db, id) {
        Core::If { then_, else_, .. } => {
            if let Some(r) = branch_union(db, &[then_, else_]) {
                return Some(r);
            }
        }
        Core::Match { arms, .. } => {
            let bodies: Vec<StructId> = arms.iter().map(|a| a.body).collect();
            if !bodies.is_empty()
                && let Some(r) = branch_union(db, &bodies)
            {
                return Some(r);
            }
        }
        // A COLLECTION COUNT — `List.len`/`Bytes.len`/`Map.size`/`Set.len` — is a NON-NEGATIVE `Int64`
        // whose backend value is an i32 count zero-extended to i64 (`I64ExtendI32U`), so it lives in
        // `[0, 2^32 − 1]`. This lets the range folds fire on a length: `(>= (List.len xs) 0)` → true,
        // `(< (List.len xs) 0)` → false, a `(match (List.len xs) (-1 …) …)` drops the impossible arm, and
        // a length used as a mask/shift/dividend operand sheds guards. The `2^32-1` upper bound is the true
        // representable maximum (a count can't exceed the i32 the runtime returns); a tighter real cap
        // (heap size) is unknown here, so `2^32-1` is the sound envelope.
        Core::ListLen { .. }
        | Core::BytesLen { .. }
        | Core::StrScalarLen { .. }
        | Core::MapSize { .. }
        | Core::SetLen { .. } => {
            return Some((0, Some(u32::MAX as i64)));
        }
        _ => {}
    }
    // Else the declared integer type's bounds. `min` must be i64-representable (it always is: signed MIN
    // and unsigned 0 both fit); `max` may be absent (unsigned-64).
    match crate::infer::type_of(db, id) {
        crate::ty::Ty::Int(it) => match resolved_int_bounds(it) {
            Some((Some(lo), hi)) => Some((lo, hi)),
            _ => None,
        },
        _ => None,
    }
}

/// The range of a `Core::Arith { op, lhs, rhs }` result, propagated from the operand ranges. `None` when
/// the op is not range-tracked or an operand's range is unknown (→ the node falls back to its type). All
/// arithmetic is `i128` (endpoints never wrap); each endpoint is clamped to `i64` (an out-of-i64 bound
/// becomes `None` on that side = "unbounded there"). Covers: `&` (bounded by the smaller nonneg operand),
/// `|`/`^` (bounded by `2^max(bits)` for nonneg operands), `<<`/`>>ᵤ` by a constant, `%` by a constant
/// divisor (`[-(|C|-1), |C|-1]`, tightened to `[0, |C|-1]` for a nonneg dividend), and `+`/`-`/`*`
/// interval arithmetic. Verified sound by exhaustive endpoint checks.
fn arith_range(db: &mut Db, op: Prim, lhs: StructId, rhs: StructId) -> Option<(i64, Option<i64>)> {
    let clamp = |lo: i128, hi: i128| -> (i64, Option<i64>) {
        let lo = lo.max(i64::MIN as i128) as i64;
        let hi = if hi <= i64::MAX as i128 {
            Some(hi as i64)
        } else {
            None
        };
        (lo, hi)
    };
    // Significant-bit count of a NON-NEGATIVE constant (`⌈log2(v+1)⌉`), for the bitwise-op bounds.
    let const_nonneg_bits = |db: &mut Db, o: StructId| -> Option<u32> {
        match core_of(db, o) {
            Core::ConstInt(v) => v
                .to_i64()
                .filter(|&x| x >= 0)
                .map(|x| 64 - (x as u64).leading_zeros()),
            _ => None,
        }
    };
    match op {
        // `v & M`: a nonneg operand (constant mask OR a value with a known `[0,hi]`) caps the result at
        // its significant-bit count; the AND fits the MIN of whatever bounds are known. A nonneg mask on
        // either side alone bounds it — `(& x:Int64 15)` → [0,15] regardless of `x`'s sign.
        Prim::BitAnd => {
            let bits = |db: &mut Db, o: StructId| -> Option<u32> {
                const_nonneg_bits(db, o).or_else(|| unsigned_value_bits(db, o))
            };
            let (a, b) = (bits(db, lhs), bits(db, rhs));
            let bound = a.into_iter().chain(b).min()?;
            Some((0, Some((1i64 << bound.min(62)) - 1)))
        }
        // `v | w` / `v ^ w`: a nonneg result sets no bit above `max` of the operands' bit counts. BOTH
        // operands must be provably nonnegative (a negative operand's high bits would set the sign bit).
        Prim::BitOr | Prim::BitXor => {
            let a = const_nonneg_bits(db, lhs).or_else(|| unsigned_value_bits(db, lhs))?;
            let b = const_nonneg_bits(db, rhs).or_else(|| unsigned_value_bits(db, rhs))?;
            Some((0, Some((1i64 << a.max(b).min(62)) - 1)))
        }
        // `v <<ᵤ k` (constant `k`): a nonneg `v ∈ [0,hi]` shifts to `[0, hi << k]` (in i128, clamped).
        Prim::Shl => {
            let (0, Some(hi)) = value_range(db, lhs)? else {
                return None;
            };
            let Core::ConstInt(k) = core_of(db, rhs) else {
                return None;
            };
            let k = k.to_i64().filter(|&k| (0..64).contains(&k))?;
            Some(clamp(0, (hi as i128) << k))
        }
        // `v >>ᵤ k` (constant `k`, LOGICAL — an unsigned/nonneg operand): a nonneg `v` shifted right by
        // `k` loses its low `k` bits. Two cases, both requiring `v` proven NONNEGATIVE (`value_range` lo
        // == 0; a signed `>>ₛ` sign-extends and is excluded):
        //   • a KNOWN finite `v ∈ [0, hi]` → `[0, hi >> k]`;
        //   • an UNBOUNDED-above nonneg `v` (a bare `UInt64`, whose max `2^64-1` is not i64-representable)
        //     → still bounded by the TYPE WIDTH: an unsigned width-`W` value is `< 2^W`, so `v >>ᵤ k <
        //     2^(W−k)` → `[0, 2^(W−k) − 1]`. This is what makes `(& (>> x 56) 255)` on a UInt64 drop its
        //     redundant mask (`x >>ᵤ 56 ∈ [0,255]` already fits the mask). `W − k` may be ≥ 64 for small
        //     `k` at width 64 → the bound is not i64-representable, so leave it unbounded (`None`).
        Prim::Shr => {
            let (0, hi_opt) = value_range(db, lhs)? else {
                return None;
            };
            let Core::ConstInt(k) = core_of(db, rhs) else {
                return None;
            };
            let k = k.to_i64().filter(|&k| (0..64).contains(&k))?;
            match hi_opt {
                Some(hi) => Some((0, Some(hi >> k))),
                None => {
                    // Unbounded-above nonneg: bound by the type width. `W − k` significant bits remain.
                    let crate::ty::Ty::Int(it) = crate::infer::type_of(db, lhs) else {
                        return None;
                    };
                    let crate::ty::Width::Fixed(w) = it.width else {
                        return None;
                    };
                    let bits = (w as i64) - k;
                    // `bits` in `1..=62` → `2^bits − 1` fits i64 and is computed by the shift. `bits == 63`
                    // is exactly `2^63 − 1 = i64::MAX` (the shift `1i64 << 63` is `i64::MIN`, so `− 1` would
                    // OVERFLOW in a checked build — a latent panic; handle it directly). `bits ≥ 64` or `≤ 0`
                    // → not i64-representable, so nonneg but no finite upper bound.
                    match bits {
                        1..=62 => Some((0, Some((1i64 << bits) - 1))),
                        63 => Some((0, Some(i64::MAX))),
                        _ => Some((0, None)), // still nonneg, but no finite i64 upper bound
                    }
                }
            }
        }
        // `v % C` (constant divisor `C`): the truncated-toward-zero remainder has magnitude `< |C|`, so
        // its range is `[-(|C|-1), |C|-1]` in general, tightened to `[0, |C|-1]` when the DIVIDEND is
        // provably non-negative (a nonneg dividend yields a nonneg remainder). `(% x 10)` → `[-9,9]`, and
        // `(% (& x 255) 10)` → `[0,9]`. Only a compile-time constant divisor (the common `% C` idiom); a
        // runtime divisor's range is unknown here. `C = 0` never reaches this (a constant `÷0`/`%0` is a
        // constant-trap poison in `lower`); `|C| = 1` folds to `0` (the `% 1` identity) before here.
        Prim::Rem => {
            let Core::ConstInt(c) = core_of(db, rhs) else {
                return None;
            };
            let d = c.to_i64().map(|c| c.unsigned_abs()).filter(|&d| d >= 1)?;
            let hi = (d - 1).min(i64::MAX as u64) as i64;
            let lo = if value_provably_nonneg(db, lhs) {
                0
            } else {
                -hi
            };
            Some((lo, Some(hi)))
        }
        // `+`/`-`/`*`: interval arithmetic over the operands' CLOSED ranges.
        Prim::Add | Prim::Sub | Prim::Mul => {
            let (alo, ahi) = closed_range(db, lhs)?;
            let (blo, bhi) = closed_range(db, rhs)?;
            let (alo, ahi, blo, bhi) = (alo as i128, ahi as i128, blo as i128, bhi as i128);
            let (lo, hi) = match op {
                Prim::Add => (alo + blo, ahi + bhi),
                Prim::Sub => (alo - bhi, ahi - blo),
                _ => {
                    let c = [alo * blo, alo * bhi, ahi * blo, ahi * bhi];
                    (*c.iter().min().unwrap(), *c.iter().max().unwrap())
                }
            };
            Some(clamp(lo, hi))
        }
        _ => None,
    }
}

/// A CLOSED range `[min, max]` (both bounds i64) for a value, or `None` if either bound is unknown — the
/// form INTERVAL ARITHMETIC needs. Wraps `value_range`, demanding a finite `max` (an unbounded value can
/// overflow any op, so no fit proof).
fn closed_range(db: &mut Db, id: StructId) -> Option<(i64, i64)> {
    match value_range(db, id) {
        Some((lo, Some(hi))) => Some((lo, hi)),
        _ => None,
    }
}

/// Whether a CHECKED `+`/`-`/`*` at `id` provably CANNOT overflow its result type — so its overflow guard
/// (and narrow range-check) can be elided at emit. Computes the exact result INTERVAL from the operands'
/// `closed_range`s (in `i128`, so the interval endpoints never wrap) and checks it lies within the result
/// type's `[min, max]`. The interval per op: `+` → `[alo+blo, ahi+bhi]`; `-` → `[alo−bhi, ahi−blo]`; `*` →
/// the min/max of the four corner products. A `None` operand range → `false` (unknown, keep the guard).
/// Verified sound by exhaustive endpoint check. Lets a masked/narrowed operand (`(+ (& x 15) (& y 15))`,
/// sum ≤ 30) shed its guard.
pub(crate) fn arith_provably_in_range(
    db: &mut Db,
    op: Prim,
    lhs: StructId,
    rhs: StructId,
    result: crate::ty::IntTy,
) -> bool {
    // The RESULT type's inclusive bounds come from `result` — the op's AUTHORITATIVE machine width/sign
    // (the caller's `Machine`, derived from the ARITH NODE's solved type), NOT from an operand's node
    // type. WARNING: An operand's node type can be misleading: a bare-literal-branch `if` (`(if c 1 0)`) or a
    // bare literal is still DEFERRED, and its default (Int64) is WIDER than the op when the true width
    // comes from CONTEXT — `(: (+ (if (< n 5) 100 0) 100) Int8)` is an Int8 op even though both operands'
    // nodes are deferred, and `(- 0 n)` at Int8 is Int8 though the `0` is deferred. Using the op's real
    // width is the only sound choice. (An unsigned-64 result has no i64 max → cannot prove a fit here.)
    let (Some(tmin), Some(tmax)) = (match resolved_int_bounds(result) {
        Some(b) => b,
        None => return false,
    }) else {
        return false;
    };
    let (Some((alo, ahi)), Some((blo, bhi))) = (closed_range(db, lhs), closed_range(db, rhs))
    else {
        return false;
    };
    let (alo, ahi, blo, bhi) = (alo as i128, ahi as i128, blo as i128, bhi as i128);
    let (rlo, rhi) = match op {
        Prim::Add => (alo + blo, ahi + bhi),
        Prim::Sub => (alo - bhi, ahi - blo),
        Prim::Mul => {
            let c = [alo * blo, alo * bhi, ahi * blo, ahi * bhi];
            (*c.iter().min().unwrap(), *c.iter().max().unwrap())
        }
        _ => return false,
    };
    rlo >= tmin as i128 && rhi <= tmax as i128
}

/// Whether a discharged verification proof licenses eliding the overflow guard at Core node `id` — the
/// proof-guided-elision oracle (v-verification × v-core-opt seam, DESIGN-verification-program-conditions.md
/// §3, fork #3). A discharged `Thm` whose conclusion is `no-overflow@id` (and whose hypotheses are covered
/// by the node's precondition, per v-verification's `licenses` match predicate) makes this return `true`,
/// so [`provably_no_overflow`] elides the checked op's guard.
///
/// STUB (Slice-5): returns `false` unconditionally, so the disjunction in [`provably_no_overflow`] is
/// exactly `arith_provably_in_range` — i.e. today's behavior, byte-identical on both backends. This lands
/// the seam behavior-neutrally. v-verification's Inc-b b3 fills the real body: a compile-time `eval` of the
/// discharge program for `id`, returning the licensing boolean and FAIL-CLOSED on an eval trap (an
/// overflowing obligation traps during eval → return `false`, keep the guard). Editing this one function is
/// the ENTIRE b3 change; the wrapper, the predicate, and both emit sites stay put — unforgeability still
/// gates elision (no discharged `Thm` ⇒ `false` ⇒ the check stays; the default is always the check).
pub(crate) fn discharged_no_overflow(_db: &mut Db, _id: StructId) -> bool {
    false
}

/// The FLOW-FACT disjunct of [`provably_no_overflow`] — whether the value-facts lattice proves the checked
/// `op` at Core node `id` stays in `result`'s range, licensing the overflow-guard elision (design
/// `DESIGN-flow-sensitive-value-facts.md` §4.1, the third independently-sound disjunct alongside
/// `arith_provably_in_range` and v-verification's `discharged_no_overflow`). Ownership is partitioned: this
/// is v-value-facts' seam, `discharged_no_overflow` is v-verification's, and v-core-opt owns the wrapper.
///
/// SLICE 1–2: the integer facet's proof IS the generalized interval analysis, so this delegates to
/// [`arith_provably_in_range`] — a redundant-but-SOUND disjunct (it can only make the same or more ops
/// elide, never a false positive), behavior-neutral because that disjunct already fires in the wrapper. It
/// exists as a NAMED, stable seam so the later non-integer facets (a `nonzero` divisor, a `len_range`
/// bounds, a `variant_tags` discriminant — each consumed at ITS own guard, not here) plug into the same
/// fail-closed shape without re-touching the wrapper. FAIL-CLOSED: any uncertainty / missing fact /
/// unrefined operand ⇒ `false` (the default is always to KEEP the check), so the `OR` is sound.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fact_proven_safe(
    db: &mut Db,
    op: Prim,
    lhs: StructId,
    rhs: StructId,
    result: crate::ty::IntTy,
    _id: StructId,
) -> bool {
    // Slice 1–2: the flow-fact proof for an INTEGER overflow is exactly the interval analysis (which
    // already consults the flow-sensitive refinement env via `value_range`). A dedicated fact path for the
    // non-integer facets attaches at their own consumers, not this arith guard.
    arith_provably_in_range(db, op, lhs, rhs, result)
}

/// The both-backend overflow-guard-elision decision for a checked `+`/`-`/`*` at Core node `id`: elide iff
/// the result provably stays in the type by INTERVAL ANALYSIS ([`arith_provably_in_range`]), OR the
/// value-facts lattice proves it ([`fact_proven_safe`]), OR a discharged verification PROOF licenses it
/// ([`discharged_no_overflow`]). This is the single Core-tier decision BOTH backends consult (rust
/// `emit_arith`, wasm `emit_checked_arith_to`), so range analysis, a flow-fact, and a discharged `Thm` all
/// elide uniformly with no emit change. Each disjunct is INDEPENDENTLY SOUND and FAIL-CLOSED, so the `OR`
/// only ever makes MORE ops elide, and only on a real proof — sound by the same argument as any one alone
/// (default = keep the check). Ownership is partitioned across the three disjuncts (v-core-opt / v-value-
/// facts / v-verification); this wrapper is v-core-opt's seam.
#[allow(clippy::too_many_arguments)]
pub(crate) fn provably_no_overflow(
    db: &mut Db,
    op: Prim,
    lhs: StructId,
    rhs: StructId,
    result: crate::ty::IntTy,
    id: StructId,
) -> bool {
    arith_provably_in_range(db, op, lhs, rhs, result)
        || fact_proven_safe(db, op, lhs, rhs, result, id)
        || discharged_no_overflow(db, id)
}

/// Whether `val << k` provably CANNOT overflow the type of `val` — so the shift's overflow round-trip
/// guard (and narrow range-check) can be elided. A left shift is exact multiplication by `2^k`, so it
/// overflows iff the interval `[vlo << k, vhi << k]` leaves the type's `[min, max]`. `<<` is monotone for
/// `k ≥ 0`, so checking both endpoints (in `i128`, never wrapping) suffices. Used by both the `<<` emit
/// and the `* 2^k → <<` strength-reduction emit — a masked/bounded operand (`(<< (& x 15) 2)` = `[0,60]`)
/// sheds its guard. `None` operand range → `false` (keep the guard). Verified sound by endpoint check.
pub(crate) fn shl_provably_in_range(db: &mut Db, val: StructId, k: u32) -> bool {
    let crate::ty::Ty::Int(it) = crate::infer::type_of(db, val) else {
        return false;
    };
    let (Some(tmin), Some(tmax)) = (match resolved_int_bounds(it) {
        Some(b) => b,
        None => return false,
    }) else {
        return false;
    };
    // CLEAR-LOW-BITS IDIOM: `(v >> k) << k` — a right shift by `k` then a left shift by the SAME `k` — just
    // CLEARS the low `k` bits of `v`, so the result `floor(v / 2^k) * 2^k` NEVER leaves `v`'s type, for
    // BOTH shift kinds. Signed (`>>ₛ`, floor toward −∞): `MIN` is a multiple of `2^k` (k ≤ 63), so the
    // rounded-down value stays ≥ `MIN` and ≤ 0 ≤ `MAX`. Unsigned (`>>ᵤ`): `q·2^k ≤ v`, stays in `[0, max]`.
    // The INTERVAL analysis below cannot prove this — a signed `v >>ₛ k` over a full-range `v` has no
    // finite range, so `closed_range` returns the type bounds and `[MIN<<k, MAX<<k]` spuriously overflows.
    // This structural recognizer sees the correlation the intervals lose. `val`'s type IS `v`'s type
    // (`>>` preserves width), so the fit is against the right bounds. The inner count must be the SAME
    // constant `k`.
    if let Core::Arith {
        op: Prim::Shr,
        rhs: inner_count,
        ..
    } = core_of(db, val)
        && let Core::ConstInt(ic) = core_of(db, inner_count)
        && ic.to_i64() == Some(k as i64)
    {
        return true;
    }
    let Some((vlo, vhi)) = closed_range(db, val) else {
        return false;
    };
    // `k < 128` guaranteed (a valid shift count is `< width ≤ 64`); the i128 shift never overflows for a
    // real operand interval, and the fit-check against the i64-representable type bounds catches any
    // out-of-type result.
    let (rlo, rhi) = ((vlo as i128) << k, (vhi as i128) << k);
    rlo >= tmin as i128 && rhi <= tmax as i128
}

/// The RUNTIME-COUNT companion of [`shl_provably_in_range`]: whether `(<< val count)` provably stays in
/// `val`'s type for a count whose range is only known at compile time (not a fixed constant) — the
/// masked-count idiom `(<< (& x 15) (& k 3))`, where `val ∈ [0,15]` and `count ∈ [0,7]` gives a max
/// `15 << 7 = 1920` that fits. Requires BOTH the value's `closed_range` `[vlo, vhi]` AND the count's
/// `value_range` `[clo, chi]` to be known, with a valid count range `0 <= clo <= chi < width` (an
/// out-of-range count is a genuine trap the guard must keep). `<<` is monotonic in the value for a fixed
/// nonneg count, so the result's bounding box is spanned by the four corners `{vlo,vhi} << {clo,chi}`
/// (a negative `vlo` shifts MORE negative as the count grows; a positive `vhi` shifts MORE positive);
/// the box fits iff its min ≥ tmin and max ≤ tmax. All in i128 (a real interval × count < width never
/// overflows i128). Returns false on any unknown bound (keep the overflow round-trip).
pub(crate) fn shl_provably_in_range_dynamic(db: &mut Db, val: StructId, count: StructId) -> bool {
    let crate::ty::Ty::Int(it) = crate::infer::type_of(db, val) else {
        return false;
    };
    let (Some(tmin), Some(tmax)) = (match resolved_int_bounds(it) {
        Some(b) => b,
        None => return false,
    }) else {
        return false;
    };
    let crate::ty::Width::Fixed(width) = it.width else {
        return false;
    };
    // The count must have a known, VALID range `[clo, chi]` with `0 <= clo` and `chi < width` — an
    // out-of-range count still traps (its guard is handled separately), so we only reason about the
    // in-range shift amounts here.
    let Some((clo, chi)) = value_range(db, count) else {
        return false;
    };
    let Some(chi) = chi else { return false };
    if clo < 0 || chi < 0 || chi >= width as i64 {
        return false;
    }
    let Some((vlo, vhi)) = closed_range(db, val) else {
        return false;
    };
    // `<<` by a fixed nonneg count is monotonic in the value AND (in magnitude) in the count, so the
    // result's bounding box corners are `{vlo, vhi} << {clo, chi}`.
    let corners = [
        (vlo as i128) << clo,
        (vlo as i128) << chi,
        (vhi as i128) << clo,
        (vhi as i128) << chi,
    ];
    let rlo = corners.iter().copied().min().unwrap();
    let rhi = corners.iter().copied().max().unwrap();
    rlo >= tmin as i128 && rhi <= tmax as i128
}

/// Whether the divisor at `id` could be `-1`. The narrow-signed-division range-check exists SOLELY for
/// the `MIN_N / -1` overflow (the only quotient that leaves the type); if the divisor provably is NOT
/// `-1`, that check is dead. Returns `true` (keep the check) unless the divisor's range EXCLUDES `-1` —
/// a constant `≠ -1`, or a value whose `value_range` does not straddle `-1` (e.g. an unsigned/nonneg
/// value, or a masked `(& y 7)` ∈ [0,7]). Conservative: an unknown range → `true`.
pub(crate) fn divisor_can_be_neg_one(db: &mut Db, id: StructId) -> bool {
    match value_range(db, id) {
        Some((lo, Some(hi))) => lo <= -1 && -1 <= hi, // -1 within [lo, hi]
        Some((lo, None)) => lo <= -1,                 // unbounded above; can reach -1 iff lo <= -1
        None => true,                                 // unknown → assume it can
    }
}

/// Whether the value at `id` is provably NON-NEGATIVE (its `value_range` lower bound is `≥ 0`). Consults
/// the same lattice as the guard-elision checks — so it sees a mask (`(& x 255)` ∈ [0,255]), an unsigned
/// type, AND a FLOW-SENSITIVE refinement (`x` under `(> x 0)`). Used by the signed `/`/`%` by a power of
/// two: a non-negative dividend truncates toward zero identically to a plain shift/mask, so the
/// round-toward-zero BIAS sequence (needed only to correct negatives) is DEAD. Conservative: an unknown
/// or possibly-negative range → `false` (keep the bias).
pub(crate) fn value_provably_nonneg(db: &mut Db, id: StructId) -> bool {
    matches!(value_range(db, id), Some((lo, _)) if lo >= 0)
}

/// Whether the index at `index` is flow-provably `< len(list)` — the operator-greenlit BOUNDS facet, keyed
/// on COLLECTION IDENTITY. Resolves both operands to their binders and consults `refined_below_len`: the
/// fact `below_len[index_binder] = list_binder` (established by an enclosing `(< i (List.len xs))` guard)
/// licenses `List.at list index` to shed its OWN `index < len` bounds check. SOUND ONLY when the accessed
/// list is the SAME binder the fact names — a guard on a DIFFERENT list must never elide this check (that
/// would be an out-of-bounds read), so the collection binders must match exactly. Conservative: no fact, a
/// non-binder operand, or a mismatched collection → `false` (keep the runtime check). Both operands are
/// leaf occurrences (a param/kept-local ref), so `core_of` here never inlines a payload out of a dup-site
/// (the slice-5 demand-order hazard does not arise for a leaf ref).
pub(crate) fn index_provably_below_len(db: &mut Db, index: StructId, list: StructId) -> bool {
    let binder_of = |db: &mut Db, id: StructId| -> Option<StructId> {
        match core_of(db, id) {
            Core::Param { binder } | Core::LocalRef { binder } => Some(binder),
            _ => None,
        }
    };
    let (Some(i), Some(xs)) = (binder_of(db, index), binder_of(db, list)) else {
        return false;
    };
    db.refined_below_len(i) == Some(xs)
}

/// Whether the value at `id` provably lies within the inclusive `[lo, hi]` — its `value_range` is known
/// AND fully contained. Consults the same lattice as the guard-elision checks (a mask, an unsigned type,
/// a flow-refinement). Used by `emit_wrap`'s truncation-elision: a `wrap` to width N is a no-op when the
/// operand already lies in the target's `[min_N, max_N]` (an unsigned target's `[0, 2^N-1]`), even when
/// the operand's TYPE is wider — `UInt8.wrap(& x 255)` needs no re-mask. Conservative: an unknown range,
/// or one that exceeds either bound, → `false` (keep the truncation).
pub(crate) fn value_range_within(db: &mut Db, id: StructId, lo: i64, hi: i64) -> bool {
    matches!(value_range(db, id), Some((vlo, Some(vhi))) if vlo >= lo && vhi <= hi)
}

/// Whether the value at `id` provably CANNOT equal the constant `c` — its `value_range` is known and `c`
/// lies strictly OUTSIDE it (`c < min` or `c > max`). Consults the same lattice as the guard/comparison
/// folds (a mask, an unsigned type, a flow-refinement). Used by the match probe chain to drop a DEAD arm
/// whose literal probe the scrutinee's range excludes (`(match (& x 7) (100 …) …)`: `x & 7 ∈ [0,7]`, so
/// the `100` arm can never match). Conservative: an unknown range, or one that contains `c`, → `false`
/// (keep the arm). A `None` upper bound (unsigned-64) only excludes on the low side.
pub(crate) fn value_excludes(db: &mut Db, id: StructId, c: i64) -> bool {
    match value_range(db, id) {
        Some((lo, hi)) => c < lo || hi.is_some_and(|h| c > h),
        None => false,
    }
}

/// Structurally compare two CONSTANT compound values at `a`/`b`, returning `Some(true/false)` if BOTH are
/// compile-time-visible constants (a `SumNew`/`Tuple`/`Record`/`ListNew`, or a scalar leaf), else `None`
/// (a runtime operand — the caller declines, deferring to the heap walk). Equality is STRUCTURAL
/// (`core-semantics.md §Equality Is Structural`): two values are equal iff same shape + component-wise
/// equal. A `SumNew` compares its discriminant then its payloads pairwise; a `Tuple`/`ListNew` its
/// elements pairwise (unequal length → not equal); a `Record` its fields (the field SET is fixed by the
/// type, so same-typed records share keys — compare each). Scalar leaves compare by value. Two DIFFERENT
/// compound KINDS (a tuple vs a sum) never fold here — the type checker rejects a cross-shape `=` before
/// lowering, so a kind mismatch reaching here is a compiler bug → `None` (decline).
pub(crate) fn const_compound_eq(db: &mut Db, a: StructId, b: StructId) -> Option<bool> {
    match (core_of(db, a), core_of(db, b)) {
        (Core::ConstInt(x), Core::ConstInt(y)) => Some(x.eq_value(&y)),
        (Core::ConstBool(x), Core::ConstBool(y)) => Some(x == y),
        (Core::ConstStr(x), Core::ConstStr(y)) => Some(x == y),
        // Two chars: equal iff their scalar values match.
        (Core::ConstChar(x), Core::ConstChar(y)) => Some(x == y),
        // Two floats: equal iff their canonical BITS match AT THE OPERAND'S OWN FLOAT WIDTH (a nested
        // `Float32` pair demotes to binary32 first, `const_float_bits_at_operand_width`) — so a `-0.0` is distinct from
        // `0.0` (`(= (tuple -0.0) (tuple 0.0))` → false). By-bits, NOT `f64` `==`, precisely so `-0.0`/
        // `0.0` differ — the structural byte-form rule.
        (Core::ConstFloat(x), Core::ConstFloat(y)) => {
            // Demote each to the operand's own float width first — a nested `Float32` pair compares at
            // binary32, not at the un-demoted f64 payload (adv-61; mirrors the scalar `=` fold).
            let bx = const_float_bits_at_operand_width(db, a, x.to_f64_bits());
            let by = const_float_bits_at_operand_width(db, b, y.to_f64_bits());
            Some(bx == by)
        }
        // A nested NaN under the canonical byte form: every NaN equals every NaN (`(= (tuple nan) (tuple
        // nan))` → true, `(= (Some nan) (Some nan))` → true), and `nan` is unequal to any finite float —
        // the SAME rule the scalar `=` fold applies, recursed through a compound.
        (Core::ConstFloatNan, Core::ConstFloatNan) => Some(true),
        (Core::ConstFloatNan, Core::ConstFloat(_)) | (Core::ConstFloat(_), Core::ConstFloatNan) => {
            Some(false)
        }
        // A nested +∞: equal only to +∞ (single canonical bit form), unequal to any finite float and to a
        // NaN — the same byte-form rule the scalar `=` fold applies, recursed through a compound.
        (Core::ConstFloatInf, Core::ConstFloatInf) => Some(true),
        (Core::ConstFloatInf, Core::ConstFloat(_))
        | (Core::ConstFloat(_), Core::ConstFloatInf)
        | (Core::ConstFloatInf, Core::ConstFloatNan)
        | (Core::ConstFloatNan, Core::ConstFloatInf) => Some(false),
        (Core::Unit, Core::Unit) => Some(true),
        // Two sum values: equal iff same discriminant AND equal payloads (pairwise). A different disc is
        // not-equal WITHOUT comparing payloads (`(Some 1)` ≠ `None`). Same disc ⇒ same variant ⇒ same
        // payload arity (the type fixes it), so a pairwise payload compare is well-formed.
        (
            Core::SumNew {
                disc: da,
                payloads: pa,
            },
            Core::SumNew {
                disc: db_,
                payloads: pb,
            },
        ) => {
            if da != db_ {
                return Some(false);
            }
            if pa.len() != pb.len() {
                return Some(false);
            }
            for (&x, &y) in pa.iter().zip(pb.iter()) {
                if !const_compound_eq(db, x, y)? {
                    return Some(false);
                }
            }
            Some(true)
        }
        // A baked `Core::ConstBytes` on either side (the `Ast.encode` fold now produces one): equal iff the
        // same raw bytes. `const_byte_slice` also reads a `Core::BytesOf` of constants, so a ConstBytes-vs-
        // BytesOf pair (e.g. `(= (Ast.encode …) (Bytes.of …))`) compares too. Placed before the element-wise
        // BytesOf arm; a BytesOf/BytesOf pair still takes that arm.
        (Core::ConstBytes(_), _) | (_, Core::ConstBytes(_)) => {
            match (const_byte_slice(db, a), const_byte_slice(db, b)) {
                (Some(x), Some(y)) => Some(x == y),
                _ => None,
            }
        }
        (Core::Tuple { elems: ea }, Core::Tuple { elems: eb })
        | (Core::ListNew { elems: ea }, Core::ListNew { elems: eb })
        // Two constant byte sequences: equal iff the same bytes in the same order — the SAME
        // element-wise compare as a tuple/list, since a `Core::BytesOf`'s elements are constant `ConstInt`
        // bytes (`0..=255`, range-checked at `lower_bytes_of`). `(= (Bytes.of (list 1 2)) (Bytes.of (list
        // 1 2)))` → true; a different length or a differing byte → false. This is what folds the corpus's
        // `(= (Bytes.concat …) (Bytes.of …))` / `(= (Bytes.compact …) …)` witnesses (concat/compact
        // already fold to a constant `Core::BytesOf`, so both operands reach here constant).
        | (Core::BytesOf { elems: ea }, Core::BytesOf { elems: eb }) => {
            if ea.len() != eb.len() {
                return Some(false);
            }
            for (&x, &y) in ea.iter().zip(eb.iter()) {
                if !const_compound_eq(db, x, y)? {
                    return Some(false);
                }
            }
            Some(true)
        }
        (Core::Record { fields: fa }, Core::Record { fields: fb }) => {
            if fa.len() != fb.len() {
                return Some(false);
            }
            // Same-typed records share the field SET; compare each field's value by key. A key present in
            // one but not the other (a shape mismatch the type checker would have caught) is not-equal.
            for (key, &va) in fa.iter() {
                match fb.get(key) {
                    Some(&vb) => {
                        if !const_compound_eq(db, va, vb)? {
                            return Some(false);
                        }
                    }
                    None => return Some(false),
                }
            }
            Some(true)
        }
        // Two constant SETS — equal iff same size AND every element of one is present in the other (by
        // VALUE, order-independent — a set is unordered; collections-and-text.md §A Set Is A Collection Of
        // Unique Elements: two sets are equal when they contain equal elements, independent of order). Both
        // are already dedup'd (the `Set.of`/insert folds), so equal size + one-way containment suffices.
        //
        // This fold is sound ONLY when EVERY element of BOTH sets is a compile-time constant. A RUNTIME
        // element (a parameter, a call result) breaks it two ways: `lower_set_of` dedups only the CONSTANT
        // elements of its source list, so a `Core::SetOf` carrying a runtime element is NOT dedup'd —
        // `elems.len()` is then the source count, not the true cardinality (`(Set.of (list 1 2 x))` holds
        // three `elems` even when `x` = 1 at run time), so the size test is meaningless — and
        // `set_has_const_elem` reports a runtime element ABSENT (its `const_compound_eq` is `None`), so
        // containment silently treats an undecidable element as missing. Together these mis-folded a
        // runtime-element set comparison to a definite `false` — even `(= (Set.of (list x)) (Set.of (list
        // x)))` (a set equal to itself) folded to `false`. So a non-constant element on EITHER side declines
        // here (`None`) and defers to the runtime `value-eq` heap walk, which compares two `Set` handles
        // correctly (the CHAMP is order-independent + canonical by construction, `ty_heap_walkable`'s
        // `Ty::Set` arm). The sibling `MapNew` arm below already guards its runtime keys this way.
        (Core::SetOf { elems: ea, .. }, Core::SetOf { elems: eb, .. }) => {
            for &e in ea.iter().chain(eb.iter()) {
                if const_compound_eq(db, e, e) != Some(true) {
                    return None; // a runtime element — undecidable at compile time, defer to the walk
                }
            }
            if ea.len() != eb.len() {
                return Some(false);
            }
            for &x in ea.iter() {
                if !set_has_const_elem(db, &eb, x) {
                    return Some(false);
                }
            }
            Some(true)
        }
        // Two constant MAPS — equal iff same size AND every entry `k ↦ v` of one has a KEY-EQUAL entry in
        // the other whose VALUE is equal (collections-and-text.md §Two Maps Are Equal When They Associate
        // The Same Keys With Equal Values — order-independent, by value, exactly like a set but with a
        // value per key). A map's KEY SET is runtime data, NOT its type, so two maps of DIFFERENT key sets
        // are the SAME `Map<K,V>` type and compare `false` here (not a type error) — this is what folds `(=
        // (map ("a" 1)) (map ("b" 2)))` → false AND lets a `(list (map …) (map …))` element-compare fold
        // (the recursion that was declining). Entries are already dedup'd (the `map` literal / insert folds
        // keep one entry per key), so equal size + one-way key-and-value containment suffices.
        (Core::MapNew { entries: ea, .. }, Core::MapNew { entries: eb, .. }) => {
            if ea.len() != eb.len() {
                return Some(false);
            }
            for &(ka, va) in ea.iter() {
                // Find `ka` among `eb`'s keys (by value). Track whether any key comparison was AMBIGUOUS
                // (a non-const nested key → `None`): if the key is not found AND every comparison was a
                // definite `Some(false)`, the key is genuinely ABSENT, so — sizes being equal — the maps
                // differ (`false`); but if a comparison was ambiguous we cannot conclude absence, so decline.
                let mut value_at_key = None;
                let mut ambiguous_key = false;
                for &(kb, vb) in eb.iter() {
                    match const_compound_eq(db, ka, kb) {
                        Some(true) => {
                            value_at_key = Some(vb);
                            break;
                        }
                        Some(false) => {}
                        None => ambiguous_key = true,
                    }
                }
                match value_at_key {
                    Some(vb) => match const_compound_eq(db, va, vb)? {
                        true => {}                        // key present, value equal — continue
                        false => return Some(false),      // key present, value differs
                    },
                    None if ambiguous_key => return None, // couldn't rule out the key being present
                    None => return Some(false),           // key genuinely absent → maps differ
                }
            }
            Some(true)
        }
        // Any other pairing includes a runtime operand (not a constant compound) — decline the fold.
        _ => None,
    }
}

/// The CANONICAL KEY ORDER between two CONSTANT map keys `a`/`b` — the deterministic order a map's
/// entries render in (collections-and-text.md §A Map Renders As Its Entries In Canonical Key Order). The
/// compiler owns the sort (the runtime iterates hash order; the canonical form sorts by the key's value).
/// Scalar keys order by VALUE: an integer by numeric value, a string lexicographically, a bool false<true,
/// unit is a singleton. `None` when a key is not a compile-time-orderable constant (a nested compound key,
/// or a runtime key) — the caller then declines the constant escape (the runtime walker is deferred).
fn const_key_order(db: &mut Db, a: StructId, b: StructId) -> Option<std::cmp::Ordering> {
    match (core_of(db, a), core_of(db, b)) {
        // Compare by numeric value. `to_i64` covers the ≤64-bit keys the corpus uses; a wider key
        // declines the constant sort (deferred to the runtime walker).
        (Core::ConstInt(x), Core::ConstInt(y)) => Some(x.to_i64()?.cmp(&y.to_i64()?)),
        (Core::ConstStr(x), Core::ConstStr(y)) => Some(x.cmp(&y)),
        (Core::ConstBool(x), Core::ConstBool(y)) => Some(x.cmp(&y)),
        // Char orders by Unicode SCALAR value (`char: Ord`) — the runtime to-list Char order (rust targets
        // sort `{#\c,#\a,#\b}` to head `#\a`, 19-sets ckr1). The RUNTIME wasm to-list op declines a Char
        // element (a wasm-EMIT gap, v-rb's lane), but a const-fold BAKES the sorted list and never calls that
        // op, so folding here is sound + backend-uniform (matches the rust runtime; the wasm-emit gap is
        // orthogonal). `Char.from-int` builds one fallibly, so a const Char element is a `Core::ConstChar`.
        (Core::ConstChar(x), Core::ConstChar(y)) => Some(x.cmp(&y)),
        // Bytes order is UNSIGNED byte-lexicographic — the raw canonical bytes compared element-wise, shorter
        // prefix less (Rust `[u8]: Ord`). This is EXACTLY the runtime `set-to-list`/`map-to-list` Bytes order
        // (0x80 sorts as 128, not signed −128) pinned in `19-sets` (the Set-element + Map-key Bytes-order
        // cases). A `Bytes` element/key can be a baked `Core::ConstBytes` OR a `Core::BytesOf` of constant
        // bytes, so read both via `const_byte_slice`.
        (Core::ConstBytes(_), _)
        | (_, Core::ConstBytes(_))
        | (Core::BytesOf { .. }, _)
        | (_, Core::BytesOf { .. }) => {
            let xb = const_byte_slice(db, a)?;
            let yb = const_byte_slice(db, b)?;
            Some(xb.cmp(&yb))
        }
        (Core::Unit, Core::Unit) => Some(std::cmp::Ordering::Equal),
        // Tuples order ELEMENT-WISE lexicographically (position 0, then 1, …), recursing via THIS canonical
        // order — the runtime to-list tuple order (19-sets pins tuple Set.to-list lexicographic; the operator
        // wants full generality across ALL shapes). A homogeneous Set/Map holds same-arity tuples; a
        // non-orderable element (float / another nested collection / a runtime element) declines the whole
        // compare (`?`), keeping the fold sound + matching the runtime's own non-orderable decline.
        (Core::Tuple { elems: xe }, Core::Tuple { elems: ye }) => {
            if xe.len() != ye.len() {
                return None;
            }
            for (&xi, &yi) in xe.iter().zip(ye.iter()) {
                match const_key_order(db, xi, yi)? {
                    std::cmp::Ordering::Equal => {}
                    ord => return Some(ord),
                }
            }
            Some(std::cmp::Ordering::Equal)
        }
        // Lists order LEXICOGRAPHICALLY element-wise (position 0, then 1, …) via THIS canonical order — the
        // runtime `value_cmp_shaped` List order (03-equality "a runtime list orders lexicographically by its
        // elements" + "a proper prefix orders less than its extension"; Rust `[T]: Ord`). The first non-equal
        // element decides; when one list is a proper PREFIX of the other the SHORTER is less. UNLIKE a tuple
        // (fixed arity — a length mismatch is a DIFFERENT type, so it declines), two lists of DIFFERENT length
        // are the SAME type and order by prefix-then-length. A non-orderable element declines the compare (`?`).
        (Core::ListNew { elems: xe }, Core::ListNew { elems: ye }) => {
            for (&xi, &yi) in xe.iter().zip(ye.iter()) {
                match const_key_order(db, xi, yi)? {
                    std::cmp::Ordering::Equal => {}
                    ord => return Some(ord),
                }
            }
            Some(xe.len().cmp(&ye.len()))
        }
        // Sums order by DISCRIMINANT first (the variant index, as the canonical byte form encodes it), then by
        // PAYLOAD within the same variant — the runtime `value_cmp_shaped` Sum order. Different discs decide
        // immediately; same disc compares payloads element-wise via this canonical order. A non-orderable
        // payload declines (`?`). (Covers Option/Result and user sums as Set/Map elements/keys.)
        (
            Core::SumNew {
                disc: dx,
                payloads: px,
            },
            Core::SumNew {
                disc: dy,
                payloads: py,
            },
        ) => match dx.cmp(&dy) {
            std::cmp::Ordering::Equal => {
                if px.len() != py.len() {
                    return None;
                }
                for (&xi, &yi) in px.iter().zip(py.iter()) {
                    match const_key_order(db, xi, yi)? {
                        std::cmp::Ordering::Equal => {}
                        ord => return Some(ord),
                    }
                }
                Some(std::cmp::Ordering::Equal)
            }
            ord => Some(ord),
        },
        // Records order FIELD-WISE in the canonical (name-lexicographic) field order — the runtime
        // `value_cmp_shaped` Record order. A record's runtime rep is a `tuple` arr in the descriptor's field
        // order, documented as "the same canonical order equality/encode use"; that descriptor order IS
        // name-lexicographic (`Symbol` orders by (namespace, name), `resolved.rs`), which is EXACTLY the
        // iteration order of `Core::Record`'s `BTreeMap<Symbol, _>`. So walk both maps in lockstep: a differing
        // field NAME (a different record type — impossible for a homogeneous Set/Map, but defensive) declines;
        // matching names compare their VALUES via this canonical order, deciding at the first non-equal field. A
        // non-orderable field value declines (`?`). This is the LAST shape — with it a const Set/Map of records
        // folds its to-list (operator: full generality across every shape).
        (Core::Record { fields: fx }, Core::Record { fields: fy }) => {
            if fx.len() != fy.len() {
                return None;
            }
            for ((kx, &vx), (ky, &vy)) in fx.iter().zip(fy.iter()) {
                if kx != ky {
                    return None;
                }
                match const_key_order(db, vx, vy)? {
                    std::cmp::Ordering::Equal => {}
                    ord => return Some(ord),
                }
            }
            Some(std::cmp::Ordering::Equal)
        }
        // A runtime key this stage cannot rank declines.
        _ => None,
    }
}

/// Canonical VALUE total order over `CVal`s — the const-EVALUATOR twin of [`const_key_order`] (which orders
/// `Core` nodes). Orders the SAME classes it does — `Int` numerically (≤64-bit; a wider value declines), `Str`
/// lexicographically, `Bool`, `Unit` — and declines (`None`) every other class (Char / Bytes / Float / a
/// nested collection / …). Keeping the orderable set IDENTICAL to `const_key_order` (and to the runtime
/// `set-to-list` op's own non-orderable decline) is what makes the const_eval `Set.to-list` materialization
/// byte-match the `core_of` fold (#3765) AND the runtime op — same spec-pinned canonical order, three impls.
fn cval_key_order(a: &CVal, b: &CVal) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (CVal::Int(x), CVal::Int(y)) => Some(x.to_i64()?.cmp(&y.to_i64()?)),
        (CVal::Str(x), CVal::Str(y)) => Some(x.cmp(y)),
        (CVal::Bool(x), CVal::Bool(y)) => Some(x.cmp(y)),
        // Char: Unicode SCALAR order (`char: Ord`) = the runtime to-list Char order (19-sets ckr1, rust).
        (CVal::Char(x), CVal::Char(y)) => Some(x.cmp(y)),
        // Bytes: UNSIGNED byte-lexicographic (Rust `[u8]: Ord`) = the runtime to-list Bytes order (19-sets).
        (CVal::Bytes(x), CVal::Bytes(y)) => Some(x.cmp(y)),
        (CVal::Unit, CVal::Unit) => Some(std::cmp::Ordering::Equal),
        // Tuples order ELEMENT-WISE lexicographically, recursing via this canonical order — the const_eval
        // twin of `const_key_order`'s Tuple arm (runtime tuple to-list order, 19-sets). A non-orderable
        // element declines.
        (CVal::Tuple(xe), CVal::Tuple(ye)) => {
            if xe.len() != ye.len() {
                return None;
            }
            for (xi, yi) in xe.iter().zip(ye.iter()) {
                match cval_key_order(xi, yi)? {
                    std::cmp::Ordering::Equal => {}
                    ord => return Some(ord),
                }
            }
            Some(std::cmp::Ordering::Equal)
        }
        // Lists: LEXICOGRAPHIC element-wise, prefix-then-length tiebreak — the const_eval twin of
        // `const_key_order`'s List arm (runtime `value_cmp_shaped` List order; Rust `[T]: Ord`). Two lists of
        // different length are the SAME type (order by prefix, then shorter-is-less), UNLIKE a fixed-arity tuple.
        (CVal::List(xe), CVal::List(ye)) => {
            for (xi, yi) in xe.iter().zip(ye.iter()) {
                match cval_key_order(xi, yi)? {
                    std::cmp::Ordering::Equal => {}
                    ord => return Some(ord),
                }
            }
            Some(xe.len().cmp(&ye.len()))
        }
        // Sums: DISCRIMINANT first, then payload element-wise — the const_eval twin of const_key_order's Sum
        // arm (runtime value_cmp_shaped Sum order). A non-orderable payload declines.
        (
            CVal::Sum {
                disc: dx,
                payloads: px,
            },
            CVal::Sum {
                disc: dy,
                payloads: py,
            },
        ) => match dx.cmp(dy) {
            std::cmp::Ordering::Equal => {
                if px.len() != py.len() {
                    return None;
                }
                for (xi, yi) in px.iter().zip(py.iter()) {
                    match cval_key_order(xi, yi)? {
                        std::cmp::Ordering::Equal => {}
                        ord => return Some(ord),
                    }
                }
                Some(std::cmp::Ordering::Equal)
            }
            ord => Some(ord),
        },
        // Records: FIELD-WISE in canonical (name-lexicographic) field order — the const_eval twin of
        // `const_key_order`'s Record arm (runtime `value_cmp_shaped` Record order). `CVal::Record` is a
        // `BTreeMap<Symbol, CVal>` in that same canonical order; walk both in lockstep, a differing field name
        // declines, matching names compare their values. A non-orderable field declines.
        (CVal::Record(fx), CVal::Record(fy)) => {
            if fx.len() != fy.len() {
                return None;
            }
            for ((kx, vx), (ky, vy)) in fx.iter().zip(fy.iter()) {
                if kx != ky {
                    return None;
                }
                match cval_key_order(vx, vy)? {
                    std::cmp::Ordering::Equal => {}
                    ord => return Some(ord),
                }
            }
            Some(std::cmp::Ordering::Equal)
        }
        _ => None,
    }
}

/// Whether the operand at `id` has a type the runtime `value-eq` heap walk (`champ_eq`) compares
/// CORRECTLY — a compound whose leaves are all SCALAR (Int/Bool/Unit), reached through tuples, records,
/// and sum variants. Such a value is CANONICAL BY CONSTRUCTION: it holds no embedded RRB vector, CHAMP
/// map/set, or Bytes/String rope, each of whose byte form is canonical only AFTER a compaction the
/// compiler would have to emit first (`deterministic-value-form.md` §A Value Has One Canonical Byte
/// Form). Restricting the runtime `=` to this class keeps the walk EXACT without that extra machinery;
/// a compound carrying a collection/text leaf still declines (a later increment). Recurses structurally:
/// a sum is walkable iff EVERY variant's payload type is (read via `payload_ty_at_instantiation`, so a
/// generic sum's payload is checked at its actual instantiation). A cyclic type (a recursive sum whose
/// payload mentions itself — a cons list) terminates because the recursion is bounded by the DISTINCT
/// declaration occurrences visited, tracked in `seen`.
fn compound_eq_heap_walkable(db: &mut Db, id: StructId) -> bool {
    let ty = crate::infer::type_of(db, id);
    // A DIRECT `Bytes` operand is walkable even though a NESTED `Bytes` leaf is not (`ty_heap_walkable`'s
    // `Ty::Bytes => false`). The distinction is canonicalization: a runtime `Bytes` can be a `bytes-concat`
    // ROPE (physical bytes ≠ a flat leaf of equal content), so a raw `champ_eq` walk would misread it — but
    // the `Core::ValueEq` EMIT compacts every DIRECT String/Bytes operand with `bytes-compact` before the
    // compare (exactly as it does for a direct String, which is admitted the same way). A NESTED Bytes has
    // no such per-operand compaction: it rides the construction-site element compaction for tuple/list/
    // record/sum elements (`elem_needs_rope_compaction` DOES include Bytes) — but a `Set Bytes`/`Map Bytes`
    // KEY does NOT (`key_needs_compaction` is String/Symbol only), so a blanket walkable-Bytes would admit a
    // rope-keyed CHAMP miscompile. Scope to the DIRECT operand until the key path also compacts Bytes.
    if matches!(ty, crate::ty::Ty::Bytes) {
        return true;
    }
    ty_heap_walkable(db, &ty, &mut Vec::new())
}

/// The type-level recursion behind [`compound_eq_heap_walkable`]. `seen` holds the sum declarations
/// currently on the recursion stack, so a recursive sum (`(type IntList (Cons (Tuple Int64 IntList))
/// Nil)`) does not loop: re-entering a decl already in progress is treated as walkable (its scalar-leaf
/// obligation is discharged by the OTHER variants / the outer visit — a purely self-referential cycle
/// carries no non-scalar leaf of its own).
fn ty_heap_walkable(db: &mut Db, ty: &crate::ty::Ty, seen: &mut Vec<StructId>) -> bool {
    use crate::ty::Ty;
    match ty {
        // Scalar leaves — the base case the walk compares directly (equal canonical raw bytes).
        Ty::Int(_) | Ty::Bool | Ty::Unit => true,
        // An UNCONSTRAINED type variable — a PHANTOM parameter no value in this comparison instantiates.
        // It arises for a SIBLING variant of a multi-parameter sum whose param the compared values do not
        // use: `(= (Ok 6) (Ok 6))` types the operand `Result Int64 ?b` (the `Err` parameter `b` is free —
        // no `Err` value exists here), so walking `Result`'s variants reaches `Err`'s payload `?b`. A bare
        // unconstrained var is SCALAR-SAFE for the walk: it can only ground to a phantom (unit-like) type,
        // NEVER to a concrete non-canonical leaf — a `List`/`Bytes`/`String` is a concrete `Ty` (it reaches
        // the arm below), and a var CONSTRAINED to a collection is substituted to that concrete `Ty` before
        // reaching here. Treating it as walkable admits only the genuinely-phantom case; rejecting it was
        // over-conservative and declined `(= (Ok x) (Ok 6))` though the compared `Ok` values ARE walkable.
        Ty::Var(_) => true,
        // A tuple/record — walkable iff every element/field is. (An empty tuple is unit, trivially so.)
        Ty::Tuple(elems) => {
            let elems: Vec<Ty> = elems.to_vec();
            elems.iter().all(|e| ty_heap_walkable(db, e, seen))
        }
        Ty::Record(fields) => {
            let vals: Vec<Ty> = fields.values().cloned().collect();
            vals.iter().all(|v| ty_heap_walkable(db, v, seen))
        }
        // A sum — walkable iff every variant's payload type is. A recursive sum is broken by `seen`.
        Ty::Sum { decl, .. } => {
            if seen.contains(decl) {
                return true;
            }
            seen.push(*decl);
            let Some(variant_count) = db.type_decl_by_occ(*decl).map(|t| t.variants.len()) else {
                seen.pop();
                return false;
            };
            let mut ok = true;
            for disc in 0..variant_count {
                let ctor = db
                    .type_decl_by_occ(*decl)
                    .and_then(|t| t.variants.get(disc))
                    .and_then(|v| v.ctor);
                // A NULLARY variant (no ctor arrow → no payload) carries only its discriminant — a scalar
                // leaf, walkable. A payload-carrying variant's payload type must itself be walkable.
                if let Some(ctor) = ctor
                    && let Some(payload_ty) =
                        crate::infer::payload_ty_at_instantiation(db, ctor, ty)
                    && !ty_heap_walkable(db, &payload_ty, seen)
                {
                    ok = false;
                    break;
                }
            }
            seen.pop();
            ok
        }
        // A MAP handle is CANONICAL by construction (the runtime CHAMP is order-independent — equal maps
        // are byte-identical under `champ_eq`/`champ_hash`), so two runtime maps compare correctly by the
        // `value-eq` structural walk — walkable iff its KEY and VALUE types are themselves walkable (their
        // canonical forms are stable). This is what makes map equality independent of insertion order AND
        // independent of const-fold-vs-runtime construction (both build the same canonical CHAMP handle),
        // and makes two maps with different key SETS compare `false` (well-typed, same `Map<K,V>` type) —
        // NOT a rejection. A key/value whose canonical form needs machinery not yet emitted (Bytes rope,
        // String, a nested collection) makes the map decline, deferring to that increment.
        Ty::Map(k, v) => {
            // An `Any` key/value is a DEFERRED type of an EMPTY map (`(map)` — no entries, so its key/
            // value never determined), a PHANTOM exactly like a `Var`: an empty map carries no key or
            // value to compare, so the walk never reaches a concrete non-canonical leaf through it. Treat
            // it as walkable (else `(= (map) (map (1 10)))` — comparing an empty map to a one-entry one,
            // which MUST yield `false` — would decline). A CONCRETE key/value (Int/String/…) is checked
            // normally; a genuinely non-canonical one (a nested collection, a Bytes rope) still declines.
            let (k, v) = ((**k).clone(), (**v).clone());
            let key_ok = matches!(k, Ty::Any) || ty_heap_walkable(db, &k, seen);
            let val_ok = matches!(v, Ty::Any) || ty_heap_walkable(db, &v, seen);
            key_ok && val_ok
        }
        // A SET handle is CANONICAL by construction (the same order-independent CHAMP as a map), so two
        // runtime sets compare correctly by the `value-eq` walk — walkable iff its ELEMENT type is. An
        // `Any` element is the deferred type of an EMPTY set (`(Set.of (list))` — no elements), a phantom
        // like a map's `Any` key/value, so treat it as walkable (else `(= (Set.of (list)) (Set.of (list
        // 1)))`, which MUST yield `false`, would decline). The map analogue, one axis instead of two.
        Ty::Set(elem) => {
            let elem = (**elem).clone();
            matches!(elem, Ty::Any) || ty_heap_walkable(db, &elem, seen)
        }
        // A STRING is a flat UTF-8 byte LEAF, CANONICAL by construction — every runtime String rep is a
        // flat leaf (`str-new`, or the compiler's `bytes-alloc`+`bytes-set` build, both `alloc(Vec::new(),
        // bytes)`), NEVER a rope: `String.concat` declines for a runtime/non-ASCII operand (only a constant
        // ASCII pair folds), so no non-canonical string reaches here. So two runtime strings (and two
        // String-keyed maps) compare correctly by `value-eq`'s raw-byte walk (`champ_eq`). The type checker
        // unifies both `=` operands to `String` before lowering, so a String is only ever compared against
        // a String (never a Bytes of the same bytes). CONTRAST `Ty::Bytes` below (NON-walkable): a
        // `Bytes.concat` DOES emit a non-canonical rope (`bytes-concat`), whose byte form is canonical only
        // after a `bytes-compact` the compiler would first have to emit — so a runtime Bytes still declines.
        Ty::String => true,
        // A SYMBOL is a nominal over a flat UTF-8 String leaf — canonical by construction (its identity
        // is content-derived), so two symbols compare correctly by the raw-byte walk, exactly like the
        // `String` it wraps. (This increment folds constant symbol equality; a runtime symbol reaching
        // `=` compares by the same walk.)
        Ty::Symbol => true,
        // A nominal — walkable iff its underlying value is (the tag is erased at run time, so the walk
        // compares the underlying values directly). A recursive nominal is impossible here (a recursive
        // single-variant sum is never erased — it stays a `Ty::Sum`, handled above).
        Ty::Nominal { inner, .. } => {
            let inner = (**inner).clone();
            ty_heap_walkable(db, &inner, seen)
        }
        // A quantity ERASES to its inner numeric type, so it is walkable iff that inner type is — a `(Qty
        // Int64 u)` compares by its erased `Int64`. (A quantity `=` folds at compile time in Layer 1, but
        // classifying it by the inner type keeps this correct if a runtime quantity ever reaches `=`.)
        Ty::Qty { inner, .. } => {
            let inner = (**inner).clone();
            ty_heap_walkable(db, &inner, seen)
        }
        // A FLOAT leaf IS walkable: a float boxed into a heap compound is CANONICALIZED at construction
        // (`op_box_float`/`op_box_float32` normalize a NaN to one canonical byte form — runtime test
        // `box_float_canonicalizes_nan_to_one_byte_form` — and preserve a zero's sign bit), so the tagless
        // `champ_eq` raw-byte walk compares a nested float by its canonical bytes: `nan == nan` TRUE,
        // `-0.0 != +0.0` — the SAME canonical-byte-form rule the scalar `Core::FloatCompare` gives at top
        // level. This is the compound companion of runtime scalar float equality; the constant-compound
        // fold (`const_compound_eq`) already applies the canonical-byte float rule to a nested constant
        // float, and this makes the RUNTIME heap-walk agree. (The old decline here predated both the
        // canonicalize-on-construct invariant and scalar `FloatCompare`.)
        //= spec/capabilities/core-semantics.md#floating-point-equality-follows-the-canonical-byte-form
        //# A floating-point value MUST be equal to another floating-point value exactly when their canonical byte forms are identical, so that a negative zero is distinct from a positive zero and all not-a-number values are equal to one another.
        Ty::Float(_) => true,
        // A BYTES leaf is CANONICAL by construction wherever the walk can reach it: as a DIRECT compound
        // element (tuple/list/record/sum payload) it is `bytes-compact`ed at the construction site
        // (`elem_needs_rope_compaction` includes Bytes), and as a Map/Set KEY it is `bytes-compact`ed at
        // the key site (`key_needs_compaction` now includes Bytes) — exactly as a `String` is. So a Bytes
        // stored in any compound reaching `champ_eq` has the canonical flat byte form and the physical
        // byte-walk compares it EXACTLY (a `Bytes.concat` rope equals its flat twin). This is the nested/
        // keyed companion of the DIRECT-operand Bytes `=` (`Core::ValueEq` compacts a direct Bytes operand).
        // The ONE residual deferred edge — a Bytes NESTED inside a compound that is itself a Map/Set KEY —
        // is the SAME rarer case the String story leaves (`key_needs_compaction` compacts a direct key, not
        // a rope buried inside a compound key); it is vanishingly rare and shared with String, not a new gap.
        Ty::Bytes => true,
        // A BIGINT leaf is CANONICAL by construction: the runtime `box_bigint` serializes to the canonical
        // sign-magnitude bytes (`to_sign_magnitude_bytes`, byte-identical inline-or-heap), the SOLE producer,
        // so a runtime BigInt in a compound has one byte form and `champ_eq`'s physical byte-walk compares it
        // EXACTLY. A RATIONAL is a NORMALIZED 2-BigInt-handle node (lowest terms, sign on the numerator —
        // runtime `box_rational_normalized`, 06-numeric-model "one canonical byte form"), so `champ_eq`
        // descends its two canonical BigInt children and compares by value. Both admissions mirror the Float
        // one (`04f206e90`/`e6017d04b`): canonical-by-construction → the raw walk is sound. This unblocks a
        // whole-compound `=` over a Rational/BigInt leaf (a `V3r(Rational,Rational,Rational)`, a
        // BigInt-keyed/valued map) — was a decline forcing componentwise comparison (the CAD Rational
        // redirect's blocker). A DIRECT scalar BigInt/Rational `=` already folds/compares; this is the
        // NESTED-leaf face.
        Ty::BigInt | Ty::Rational => true,
        // A collection / char / function / type-value / unresolved leaf is NOT walkable here (its canonical
        // form needs machinery this increment does not emit, or it is not a runtime value that reaches a
        // compound equality — `Ty::Type`/`Ty::Any`/`Ty::Fn` never cross `=`). A `Char` has no runtime
        // machine rep yet (its equality folds at compile time).
        Ty::List(_) | Ty::Char | Ty::Fn(_, _) | Ty::Cont { .. } | Ty::Type | Ty::Any => false,
    }
}

/// Lower a CONVERSION application (`T.wrap`). Truncating: keeps the low `N` bits of the operand at the
/// TARGET width/signedness (`Prim::Wrap`), NEVER traps. The target type is the CONVERSION NODE's own
/// solved type (`type_of(db, id)`), read here so the fold and the runtime path agree on the width. A
/// constant operand FOLDS via `IntValue::wrap_to` to a `ConstInt` already at the target width; a runtime
/// operand becomes a `Core::Convert` the backend emits as a mask-and-reinterpret. A poison propagates.
/// Lower a sum variant CONSTRUCTOR application `(Option.Some 5)`. The discriminant is read off the
/// head's `(meta variant)` channel; the args are the payloads (an empty payload for a nullary variant,
/// which normally reaches here bare — handled in the `Resolved::Record` arm — but an explicit `(None)`
/// application is fine too). Produces `Core::SumNew` the backend builds as `sum-new(disc, payload)`.
fn lower_sum_new(db: &mut Db, id: StructId, head: StructId, args: &[StructId]) -> Core {
    let Some(disc) = crate::eval::variant_disc_of(db, head) else {
        return Core::Poison(Reject::decline(
            "a sum constructor has no discriminant metadata",
        ));
    };
    // A PARTIALLY-APPLIED constructor — FEWER args than the payload arity — reaching here is a first-class
    // FUNCTION value (the curried residual `(-> …remaining… Sum)`), NOT a constructed sum. Building a
    // `SumNew` (or, for a NEWTYPE, erasing to the short payload — the `10`) MISCOMPILES: the node's type is
    // an ARROW, but the emitted core is a bare value / short sum, so a downstream site that projects +
    // applies this value `call_indirect`s a malformed handle → INVALID WASM (`func N failed to validate:
    // expected i32, found i64` — the list/tuple-element case `(list (T.Mk 10))` / `(tuple (T.Mk 10) …)` +
    // project + apply). CHECKED FIRST — before newtype-erasure and the nullary-unit arm — because a
    // single-variant sum (a NEWTYPE) applied SHORT of arity would otherwise erase to its partial payload
    // (`args.len()==1` on a 2-payload newtype → `core_of(args[0])`), silently dropping the arity check and
    // treating the partial as if fully applied (the runtime miscompile v-runtime localized). A partial
    // ctor held in a COMPILE-TIME-VISIBLE compound (a tuple/record literal projected + applied in scope) is
    // COMPLETED to full arity by `peel_ref_annot` BEFORE reaching here, so it never trips this guard. The
    // genuine RUNTIME case (a partial ctor stored where its value is not statically visible) needs a runtime
    // eta-closure lift (not yet built — the naive synthesis miscompiles through the reducer's half-inline);
    // until then DECLINE cleanly (reject-don't-miscompile) rather than emit the malformed value.
    if let Some(arity) = crate::eval::variant_payload_arity(db, head)
        && args.len() < arity
    {
        // Synthesize the equivalent explicit lambda over the REMAINING payloads, CAPTURING the supplied
        // args — `(T.Mk 10)` → `(fn (__eta0) (T.Mk 10 __eta0))`, the shape a hand-written lambda already
        // lowers+runs correctly. The supplied args become free vars captured into the closure env; the
        // lifted func reads them back, so no bare scalar lands in a handle slot (the erstwhile miscompile).
        if let Some(c) = partial_ctor_eta_closure(db, head, args) {
            trace!(target: "rcdzc::lower", head = head.0, n_args = args.len(), arity, "sum-new: partial constructor as a runtime value → eta-closure over the remaining payloads");
            return c;
        }
        // Fallback (synthesis could not classify — a non-representable payload/result type): decline cleanly
        // (reject-don't-miscompile) rather than emit a malformed value.
        trace!(target: "rcdzc::lower", head = head.0, n_args = args.len(), arity, "sum-new: partial constructor — eta synthesis bailed, declines");
        return Core::Poison(Reject::decline(
            "a partially-applied constructor as a runtime value is not yet lowered to a runtime closure",
        ));
    }
    // NEWTYPE ERASURE: if the constructor's owning sum is an erasable NEWTYPE (a single-variant sum), the
    // value IS its payload — NO `sum-new` box, no discriminant (`type-system.md §156`, the tag adds
    // nothing to the runtime representation). Emit the payload directly: 0 payloads → unit, 1 → the
    // payload's core, n → the payload TUPLE (the same shape a multi-payload variant already boxes, now
    // the value itself). The result node's TYPE is `Ty::Nominal`, so `valtype_of` reads its underlying
    // slot — the erased core and the declared type agree.
    if let Some(decl) = crate::eval::variant_owner_decl(db, head)
        && db.newtype_inner.contains_key(&decl)
    {
        // @invariant ESTABLISH DIVERT (the (D) run-time establish enforcement — design §10.2). If the newtype
        // being constructed carries an `@invariant`, route the single-payload construction through the
        // synthesized CHECKED CONSTRUCTOR `__invariant_construct_<T>` instead of erasing straight to the
        // payload — so a value that VIOLATES the invariant TRAPS at construction rather than escaping. The
        // checked constructor builds the value, calls `__invariant_check_<T>`, and yields it or traps
        // (`invariant_establish`). EXEMPT its OWN inner `((. T V) __inv_p)` (recorded in
        // `invariant_exempt_ctors`): that construction IS the checked constructor, so re-routing it would
        // recurse forever — it keeps the plain erasure below. Only the SINGLE-payload case diverts (the
        // checked constructor takes one payload param); a 0/n-payload newtype with an invariant is not the
        // modeled shape (`invariant_establish` synthesizes a construct-def only for a single-payload newtype),
        // so it falls through to plain erasure. A `Core::Call` to the (monomorphic) construct-def emits it
        // standalone (layout reaches it via the call — NOT inlined, so the exempt inner id is preserved).
        // @invariant ESTABLISH DIVERT for a NEWTYPE (erased). A single-PAYLOAD newtype routes through
        // `__invariant_construct_<T>`; a single-variant MULTI-payload newtype (which erases to a `Ty::Tuple`,
        // not a single value) routes through its sole variant's `__invariant_construct_<T>__d0`. The whole
        // divert lives in an `#[inline(never)]` helper (see `establish_divert`) so `lower_sum_new`'s own frame
        // stays small — this function is called at every level of the layout reachability walk's `core_of`
        // descent of a `Core::SumNew` payload chain, and a frame that grew with the divert's locals overflowed
        // the compile worker's (walk-depth-bounded but fixed) stack on a pathological deep construction. The
        // helper is gated on `has_invariants()`, so a no-`@invariant` program never even calls it.
        if db.has_invariants()
            && let Some(c) = establish_divert(db, id, head, disc, args)
        {
            return c;
        }
        return match args.len() {
            0 => Core::Unit,
            1 => core_of(db, args[0]),
            _ => Core::Tuple { elems: args.into() },
        };
    }
    // A NULLARY variant is CONSTRUCTED by applying it to the unit value — `(None unit)` / `(Nil ())` —
    // the canonical form (core-semantics.md §Construction MUST Be Via Application: "(None unit)"; a
    // nullary variant carries unit). Its ctor `(meta t)` is the bare sum (no arrow → `variant_payload_type`
    // is `None`), so the single `unit` argument is NOT a payload — the payload of a nullary variant IS the
    // unit value, built as an empty array by the backend (`SumNew` with no payloads). Drop the unit arg so
    // it is not boxed as a spurious payload. (A bare `None` used as a value takes the no-arg path directly.)
    if crate::eval::variant_payload_type(db, head).is_none() && args.len() == 1 {
        // @invariant ESTABLISH DIVERT for a NULLARY variant of an @invariant type (routed through the same
        // `#[inline(never)]` helper — a nullary variant is still a VALUE that must satisfy the invariant, e.g.
        // an `@invariant` making the nullary variant uninhabitable must trap when it is constructed). Gated on
        // `has_invariants()` so a no-`@invariant` program never calls the helper (keeping this hot,
        // walk-recursion-reachable function's frame small).
        if db.has_invariants()
            && let Some(c) = establish_divert(db, id, head, disc, args)
        {
            return c;
        }
        // The argument must BE the unit value — a nullary variant applied to a non-unit is an arity error
        // the type-checker reports; here, lower it as the nullary construction (the type fault surfaces in
        // `type_errors`, and an over-payloaded nullary is caught there, not silently given a payload).
        return Core::SumNew {
            disc,
            payloads: Vec::new().into(),
        };
    }
    // NON-CANONICAL Ast.Float GUARD (uniform decline — operator-ruled A, adv-ast-float-nan differential).
    // Reifying a NON-canonical float (a NaN or an infinity) into an `Ast.Float` node has no canonical value
    // form: the wasm host-encode boundary TRAPS on the bit pattern ("encode bytes are not a valid canonical
    // value form") while the rust backend would accept it — a backend DISAGREEMENT on an accepted program.
    // The sibling non-canonical-float-in-AST paths (`,@(list nan)` splice-lift, `(Ast.Float (/ 1.0 0.0))`
    // +inf) already DECLINE at compile time; make the direct `(Ast.Float nan)` / `(Ast.Float Infinity)`
    // construction consistent by declining a CONSTANT non-canonical payload here (reject-don't-miscompile,
    // uniform on both backends). `Infinity` follows `nan` exactly: it is a USABLE float value but is NOT
    // reifiable into an `Ast.Float` (the value-accessibility directive does not make it Ast-canonical). A
    // runtime-produced non-canonical payload is caught at the escape boundary (v-runtime's canonical-encode
    // piece); this is the compile-time-constant half. Only the `Ast` sum's Float variant is guarded — an
    // ordinary float value crosses fine (a bare NaN/inf round-trips identically on both backends).
    if args.len() == 1
        && matches!(
            core_of(db, args[0]),
            Core::ConstFloatNan | Core::ConstFloatInf
        )
        && let Some(ast_disc) = ast_variant_discs(db)
        && disc == ast_disc.float
        && matches!(&ast_disc.ty, crate::ty::Ty::Sum { decl, .. }
            if crate::eval::variant_owner_decl(db, head).is_some_and(|d| d == *decl))
    {
        return Core::Poison(Reject::decline(
            "an `Ast.Float` node cannot carry a non-canonical float (a NaN or infinity has no canonical \
             value form); a finite float reifies, matching how `,@` of a non-canonical-float list and \
             `Ast.Float` of a NaN/infinity decline",
        ));
    }
    // @invariant ESTABLISH DIVERT for a MULTI-VARIANT sum (the boxed path). A multi-variant sum is not erased
    // (it boxes as this `Core::SumNew`), so it never hit the newtype block above. If its owning type carries an
    // `@invariant`, route this construction through the PER-VARIANT checked constructor
    // `__invariant_construct_<T>__d<disc>` (keyed by the discriminant, which we have in hand) so a value that
    // VIOLATES the invariant TRAPS at construction. EXEMPT the checked constructor's OWN inner `((. T V) …)`
    // (recorded in `invariant_exempt_ctors`) so it builds the plain `Core::SumNew` below — no recursion. A
    // nullary variant has no per-variant construct-def (its establish is a later injection point), so the
    // `def_by_name` lookup misses and it falls through to the plain boxed construction.
    // @invariant ESTABLISH DIVERT for a MULTI-VARIANT sum (the boxed path) — same `#[inline(never)]` helper,
    // gated on `has_invariants()`. The gate is LOAD-BEARING: the layout reachability walk descends a
    // `Core::SumNew` payload chain level-by-level via `core_of`, and running the divert's `variant_owner_decl`
    // (a `scheme_of` inference query) per level on a pathological unbounded self-application chain (e.g.
    // `(fn v (Some (v v)))` self-applied) would add native recursion depth on top of the walk. Keeping the
    // divert in an out-of-line helper AND gating on `has_invariants()` means a no-`@invariant` program's
    // `lower_sum_new` frame + call graph is byte-identical to trunk (the helper is neither entered nor inlined).
    if db.has_invariants()
        && let Some(c) = establish_divert(db, id, head, disc, args)
    {
        return c;
    }
    Core::SumNew {
        disc,
        payloads: args.to_vec().into(),
    }
}

/// The `@invariant` ESTABLISH divert, factored OUT of [`lower_sum_new`] and marked `#[inline(never)]` so its
/// locals (a `String` from `format!`, the resolved decl/type-name) never enlarge `lower_sum_new`'s stack
/// frame — which matters because `lower_sum_new` is called at every level of the layout reachability walk's
/// `core_of` descent of a `Core::SumNew` payload chain, and a frame that grew with these locals overflowed the
/// (walk-depth-bounded but fixed-size) compile worker stack on a pathological deep construction. Callers gate
/// on [`Db::has_invariants`], so for a program with no `@invariant` this function is never entered.
///
/// Returns `Some(Core::Call …)` to route the construction of an `@invariant` type through its synthesized
/// checked constructor (so a value violating the invariant TRAPS at construction — design §10.2, the (D)
/// run-time establish enforcement), or `None` to keep the plain construction. Covers all shapes: a single-
/// payload newtype (`__invariant_construct_<T>`), and — keyed by discriminant — a multi-payload newtype, a
/// multi-variant sum, and a nullary variant (`__invariant_construct_<T>__d<disc>`). The construct-def's OWN
/// inner construction is EXEMPT (`invariant_exempt_ctors`), so it builds the plain value — no recursion.
#[inline(never)]
fn establish_divert(
    db: &mut Db,
    id: StructId,
    head: StructId,
    disc: u32,
    args: &[StructId],
) -> Option<Core> {
    if db.invariant_exempt_ctors.contains(&id) {
        return None;
    }
    let decl = crate::eval::variant_owner_decl(db, head)?;
    db.invariant_of(decl)?;
    let type_name = db.type_decl_by_occ(decl).map(|d| d.name.clone())?;
    // A single-PAYLOAD newtype has the bare `__invariant_construct_<T>`; every other shape (multi-payload
    // newtype, multi-variant sum, nullary variant) has a per-variant `__invariant_construct_<T>__d<disc>`.
    let callee = db
        .def_by_name(&format!("__invariant_construct_{type_name}"))
        .filter(|_| args.len() == 1)
        .or_else(|| db.def_by_name(&format!("__invariant_construct_{type_name}__d{disc}")))?;
    trace!(target: "rcdzc::lower", node = id.0, callee, disc, "sum-new: @invariant type → checked-constructor establish divert");
    db.called.insert(callee);
    // The synthesized construct-def takes the payloads as its params (a nullary variant's def is no-arg — the
    // `(T.V unit)` unit is supplied inside the def, so pass no args); the bare single-payload path passes its
    // one payload; the per-disc path passes all payloads.
    let call_args = if crate::eval::variant_payload_type(db, head).is_none() {
        Vec::new()
    } else {
        args.to_vec()
    };
    Some(Core::Call {
        callee,
        args: call_args.into(),
    })
}

/// The `(Some-discriminant, None-discriminant)` of the `Option` sum that is the type at `id` (a
/// `List.at`/fallible-access node's result). Reads the sum's declaration by its `decl` occurrence and
/// finds the `Some`/`None` variant positions (a variant's index in the decl IS its discriminant).
/// `None` if the type is not a two-variant `Some`/`None` sum — a fallible-access result is always the
/// built-in `Option`, so a non-Option here is a compiler bug and the caller declines.
fn option_discs(db: &mut Db, id: StructId) -> Option<(u32, u32)> {
    let crate::ty::Ty::Sum { decl, .. } = crate::infer::type_of(db, id) else {
        return None;
    };
    let decl_ref = db.type_decl_by_occ(decl)?;
    let mut some_disc = None;
    let mut none_disc = None;
    for (i, v) in decl_ref.variants.iter().enumerate() {
        match v.name.as_str() {
            "Some" => some_disc = Some(i as u32),
            "None" => none_disc = Some(i as u32),
            _ => {}
        }
    }
    Some((some_disc?, none_disc?))
}

/// The `(Ok, Err)` discriminants of the built-in `Result` sum that `id`'s type resolves to (the analogue
/// of [`option_discs`] for a `(Result a e)`-typed node). `None` if `id` is not a Result sum.
fn result_discs(db: &mut Db, id: StructId) -> Option<(u32, u32)> {
    let crate::ty::Ty::Sum { decl, .. } = crate::infer::type_of(db, id) else {
        return None;
    };
    let decl_ref = db.type_decl_by_occ(decl)?;
    let mut ok_disc = None;
    let mut err_disc = None;
    for (i, v) in decl_ref.variants.iter().enumerate() {
        match v.name.as_str() {
            "Ok" => ok_disc = Some(i as u32),
            "Err" => err_disc = Some(i as u32),
            _ => {}
        }
    }
    Some((ok_disc?, err_disc?))
}

/// The SUCCESS-variant discriminant of a fallible value `id` — `Some`'s disc for an `Option`, `Ok`'s for a
/// `Result` (read off `id`'s solved sum type by variant NAME, never assumed positionally). `None` if `id`
/// is neither. This is what the `?`/try desugar unwraps: `(try e)` on a success value yields the payload;
/// the fallible variant (`None`/`Err`) is the short-circuit (the boundary break — BRICK 3). Used to fold a
/// CONSTANT success operand (`Core::SumNew` at this disc) to its payload.
fn success_disc_of(db: &mut Db, id: StructId) -> Option<u32> {
    option_discs(db, id)
        .map(|(some, _)| some)
        .or_else(|| result_discs(db, id).map(|(ok, _)| ok))
}

/// Lower `(List.at list index)` — the fallible indexed read. FOLD when the `list` operand is a
/// compile-time-visible list literal AND the `index` folds to a constant: an in-range index (`0 <= i <
/// arity`) yields `(Some elem)` — a `Core::SumNew` of the element's core at the `Some` discriminant —
/// and an out-of-range index (negative or `>= arity`) yields `None` (`Core::SumNew` with no payloads at
/// the `None` discriminant). Both fold to the ordinary sum construction, so a constant `List.at` renders
/// through the sum escape/fold with no heap. Otherwise emit the runtime `Core::ListAt` (a bounds-checked
/// `vec-get`). A poison list/index propagates.
/// The ELEMENT occurrences of a compile-time-visible constant list at `list`, whether it is a DIRECT
/// list literal (`(list 1 2 3)` → `Core::ListNew`) or a `let`-BOUND one (`(let ((xs (list 1 2 3))) …)`,
/// whose kept multi-use binding lowers the reference to a `Core::LocalRef` — in which case we follow the
/// binder to the value it names. `None` when the list is not a visible constant literal (a runtime list,
/// a `List.push`/`concat` result, a param). Used by the `List.len`/`List.at` folds so a let-bound constant
/// list folds exactly like the inline form. Following the binder does NOT erase the binding — the list is
/// still built if used elsewhere; only the len/at READ becomes a constant. The returned StructIds are the
/// bound literal's own element occurrences (the value heap is immutable, so sharing them is sound).
//
// TERMINATION (no cycle detection needed): this follows a `LocalRef` binder exactly ONE hop and does NOT
// self-recurse — the inner `match core_of(db, binder)` only matches `ListNew` or returns `None`, never
// re-enters `const_list_elems`. (amazon-q PR#556 flagged this as an unbounded recursive walk; that is a
// misread — there is no recursion here. Dismissed.)
fn const_list_elems(db: &mut Db, list: StructId) -> Option<Vec<StructId>> {
    match core_of(db, list) {
        Core::ListNew { elems } => Some(elems.to_vec()),
        Core::LocalRef { binder } => match core_of(db, binder) {
            Core::ListNew { elems } => Some(elems.to_vec()),
            _ => None,
        },
        _ => None,
    }
}

/// The FIELD MAP of a compile-time-visible constant record at `record` — a DIRECT `Core::Record`, or a
/// `let`-BOUND one whose kept multi-use binding lowers a reference to a `Core::LocalRef` (follow the
/// binder to the record it names). `None` for a genuinely RUNTIME record (a param, a call result). The
/// record analogue of [`const_list_elems`]: it lets a record ROW OP (`Record.project`/`Record.without`/
/// `Record.with`/…) fold over a shared constant-record binding exactly like the inline literal form,
/// instead of the misleading "over a RUNTIME record" decline (every field is a compile-time constant; a
/// single-use `r` already folds because it copy-propagates, so multi-use should too). Following the binder
/// does NOT erase the binding — the record is still built if used as a whole value elsewhere.
fn const_record_fields(
    db: &mut Db,
    record: StructId,
) -> Option<std::rc::Rc<std::collections::BTreeMap<crate::resolved::Symbol, StructId>>> {
    match core_of(db, record) {
        Core::Record { fields } => Some(fields),
        Core::LocalRef { binder } => match core_of(db, binder) {
            Core::Record { fields } => Some(fields),
            _ => None,
        },
        _ => None,
    }
}

/// The field→value map for a RUNTIME record `record` (a parameter, a call result, a PROJECTION `(. o pos)`
/// — one that `const_record_fields` cannot fold to a compile-time `Core::Record`), for building a row-op's
/// result. A record's field set is STATICALLY known from its solved type (`Ty::Record`), and the value heap
/// is immutable-SHARED, so a row op yields a NEW record whose UNCHANGED fields each read the source field —
/// `(. record field)` — a `Core::Proj` at that field's sorted slot (`type-system.md` §A Record Row Operation
/// Yields A New Value). This SYNTHESIZES a `(. record #field)` member node per field and RESOLVES it (via
/// `resolve_subtree`, the `fuse_match_into_if` synth-node-needs-resolution precedent), so each `core_of`s to
/// a `Core::Proj`. Returns the field→synth-projection-node map, or `None` if the record's solved type is not
/// a concrete `Ty::Record` (a still-`Any`/free record → the caller keeps its clean decline). The result is
/// used by the row-op lowerings exactly as `const_record_fields`' map is — each field is an `StructId` that
/// lowers on demand; here the StructId is the synth projection instead of a literal's field occurrence.
fn runtime_record_fields(
    db: &mut Db,
    record: StructId,
) -> Option<std::collections::BTreeMap<crate::resolved::Symbol, StructId>> {
    // The field set comes from the record's SOLVED type (peel a nominal wrapper). A non-record / not-yet-
    // ground type has no static field set → decline (unchanged behavior for a genuinely unknown operand).
    let rec_ty = crate::infer::type_of(db, record);
    let crate::ty::Ty::Record(fields) = rec_ty.strip_nominal() else {
        return None;
    };
    let field_syms: Vec<crate::resolved::Symbol> = fields.keys().cloned().collect();
    let dot = db.push_name(".");
    let mut out = std::collections::BTreeMap::new();
    for sym in field_syms {
        // Synthesize `(. record <field>)`. The key is a bare NAME node (`read_key` → `Symbol::plain`); a
        // plain (no-namespace) field name round-trips through it. (A namespaced label can't arise from a
        // source record type today — `Symbol::namespace` is always `None` here.)
        let key_node = db.push_name(&sym.name);
        let member = db.push_list(vec![dot, record, key_node]);
        // RESOLVE the synth member subtree so `core_of(member)` sees `Resolved::Member{operand: record, key}`
        // → `Core::Proj` at the field's sorted slot (a synth node is not in the load-time resolve column;
        // `resolve_subtree` fills it, exactly as `fuse_match_into_if` does for its synth arms).
        crate::resolve::resolve_subtree(db, member);
        out.insert(sym, member);
    }
    Some(out)
}

fn lower_list_at(db: &mut Db, id: StructId, list: StructId, index: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, list) {
        return Core::Poison(r);
    }
    if let Core::Poison(r) = core_of(db, index) {
        return Core::Poison(r);
    }
    let Some((disc_some, disc_none)) = option_discs(db, id) else {
        return Core::Poison(Reject::decline(
            "List.at result is not the built-in Option sum",
        ));
    };
    // FOLD a constant list literal indexed by a constant integer. The list may be inline (`(list …)`) or
    // `let`-bound (a kept multi-use binding lowers the reference to a `LocalRef`, followed by the helper).
    if let (Some(elems), Core::ConstInt(i)) = (const_list_elems(db, list), core_of(db, index)) {
        // The index is a signed Int64; a negative value or one `>= arity` is out of bounds → `None`.
        let kept: Option<usize> = i
            .to_i64()
            .and_then(|n| (n >= 0 && (n as usize) < elems.len()).then_some(n as usize));
        // STRICT LIST CONSTRUCTION (operator ruling A, #5194): a list constructor is REACHED here (it is the
        // `List.at` operand), so EVERY element ARG is evaluated even though the read selects only one — the
        // optimizer may drop the list OBJECT but MUST preserve arg evaluation. This fold DISCARDS the
        // non-selected elements, so it is sound ONLY when every DROPPED element is trap-free; if a dropped
        // sibling could trap, keep the runtime `Core::ListAt` (whose `ListNew` operand builds the list,
        // evaluating all args → the trap fires), mirroring the `List.len` trap-preservation gate. The
        // SELECTED element's own eval is preserved — it is the returned payload. (Out-of-bounds → `None`
        // drops ALL elements, so all must be trap-free.)
        let mut dropped_trap_free = true;
        for (j, &e) in elems.iter().enumerate() {
            if Some(j) != kept && !is_trap_free(db, e) {
                dropped_trap_free = false;
                break;
            }
        }
        if dropped_trap_free {
            return match kept {
                Some(n) => {
                    trace!(target: "rcdzc::fold", node = id.0, index = n as i64, "List.at folds to Some (in-bounds constant index; dropped siblings trap-free)");
                    Core::SumNew {
                        disc: disc_some,
                        payloads: vec![elems[n]].into(),
                    }
                }
                None => {
                    trace!(target: "rcdzc::fold", node = id.0, "List.at folds to None (out-of-bounds constant index; all elements trap-free)");
                    Core::SumNew {
                        disc: disc_none,
                        payloads: Vec::new().into(),
                    }
                }
            };
        }
        trace!(target: "rcdzc::fold", node = id.0, "List.at const-index fold DECLINED — a dropped element can trap; keep runtime ListAt to preserve strict-construction arg eval (ruling A)");
        // fall through to the runtime read below (its ListNew operand evaluates all element args).
    }
    // A runtime list or runtime index — emit the bounds-checked runtime read.
    Core::ListAt {
        list,
        index,
        disc_some,
        disc_none,
    }
}

/// The solved KEY and VALUE types of a map operand — `(k, v)` from its `Ty::Map(k, v)`. `None` if the
/// operand's type is not a map (a poison or an unsolved type), so the caller declines rather than
/// guessing a box op. Used by the `Map.*` lowerings to pick the key/value box ops.
fn map_kv_types(db: &mut Db, map: StructId) -> Option<(crate::ty::Ty, crate::ty::Ty)> {
    match crate::infer::type_of(db, map) {
        crate::ty::Ty::Map(k, v) => Some((*k, *v)),
        _ => None,
    }
}

/// The solved ELEMENT type of a set operand — `T` from its `Ty::Set(T)`. `None` if not a solved set type.
fn set_elem_type(db: &mut Db, set: StructId) -> Option<crate::ty::Ty> {
    match crate::infer::type_of(db, set) {
        crate::ty::Ty::Set(elem) => Some(*elem),
        _ => None,
    }
}

/// Whether the CONSTANT elements of a set (a `Core::SetOf`) contain one `const_compound_eq` to `elem`.
/// Used by the const folds (contains / insert-dedup). Both must be compile-time-visible constants.
fn set_has_const_elem(db: &mut Db, elems: &[StructId], elem: StructId) -> bool {
    elems
        .iter()
        .any(|&e| const_compound_eq(db, e, elem) == Some(true))
}

/// The canonical, HASHABLE identity of a SCALAR constant value at `id` — `None` for a compound (a
/// tuple/record/sum/list/…) or a runtime value. Reproduces `const_compound_eq`'s SCALAR-leaf equality
/// EXACTLY, so two scalar constants share a token iff `const_compound_eq` on them is `Some(true)`: an int
/// by its trimmed, sign-normalized magnitude (matching `IntValue::eq_value`), a float by canonical
/// `Float64` bits (so `-0.0` ≠ `0.0`), NaN a singleton distinct from every finite float, string/bool/char
/// by value, unit a singleton. This is the O(1) basis of the LINEAR set dedup: a scalar element hashes to
/// its token and dedups in one hash-set probe, replacing an O(elements²) pairwise `const_compound_eq`
/// scan that re-derived and deep-cloned each element's `Core` on every comparison.
#[derive(PartialEq, Eq, Hash)]
enum ScalarKey {
    Int { negative: bool, magnitude: Vec<u8> },
    Bool(bool),
    Str(String),
    Char(char),
    FloatBits(u64),
    FloatNan,
    Unit,
}

fn scalar_const_key(db: &mut Db, id: StructId) -> Option<ScalarKey> {
    match core_of(db, id) {
        Core::ConstInt(v) => {
            // Trim leading zero bytes + normalize a zero's sign — the SAME canonicalization
            // `IntValue::eq_value` applies, so equal tokens ⟺ `eq_value` true.
            let start = v.magnitude.iter().take_while(|&&b| b == 0).count();
            let magnitude = v.magnitude[start..].to_vec();
            let negative = !magnitude.is_empty() && v.negative;
            Some(ScalarKey::Int {
                negative,
                magnitude,
            })
        }
        Core::ConstBool(b) => Some(ScalarKey::Bool(b)),
        Core::ConstStr(s) => Some(ScalarKey::Str(s.to_string())),
        Core::ConstChar(c) => Some(ScalarKey::Char(c)),
        Core::ConstFloat(d) => Some(ScalarKey::FloatBits(d.to_f64_bits())),
        Core::ConstFloatNan => Some(ScalarKey::FloatNan),
        // +∞ has a definite bit pattern, so it keys by its bits like any finite float (no NaN-style
        // singleton needed) — distinct from every finite float and from NaN, so `(Set.of (list Infinity
        // Infinity 1.0))` dedups the two infinities to one element.
        Core::ConstFloatInf => Some(ScalarKey::FloatBits(f64::INFINITY.to_bits())),
        Core::Unit => Some(ScalarKey::Unit),
        // A compound (tuple/record/sum/list/set/map/bytes) or a runtime value has no scalar token — the
        // caller keeps it on the O(compounds²) pairwise path (compounds are rare + small).
        _ => None,
    }
}

/// Lower `(Set.of list)` — construct a set from a list, DEDUPLICATING. When the `list` operand is a
/// compile-time-visible `Core::ListNew`, fold to a canonical constant `Core::SetOf` (dropping later
/// duplicates by value — `const_compound_eq`), so it bakes/compares/renders as a constant set; a runtime
/// list emits `Core::SetOf` over the list's elements (a later increment reads a runtime list — declines
/// for now). The element type comes from the RESULT node's own solved `Ty::Set`. A poison propagates.
fn lower_set_of(db: &mut Db, id: StructId, list: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, list) {
        return Core::Poison(r);
    }
    let Some(elem_ty) = (match crate::infer::type_of(db, id) {
        crate::ty::Ty::Set(e) => Some(*e),
        _ => None,
    }) else {
        return Core::Poison(Reject::decline("Set.of result is not a solved set type"));
    };
    match core_of(db, list) {
        // A constant list → a canonical DEDUP'd constant set. Keep the FIRST occurrence of each element
        // value (order-independent equality means which copy is kept is unobservable; the render sorts).
        // LINEAR for scalar elements (the corpus shape): a scalar's canonical `ScalarKey` dedups in one
        // hash-set probe. A COMPOUND element (rare) has no scalar token, so it falls back to the pairwise
        // `const_compound_eq` against only the OTHER kept compounds (`compounds` list). This replaced an
        // O(elements²) `set_has_const_elem` scan over ALL kept elements (each comparison re-cloning a
        // `Core`) — a `(Set.of (list 0 1 … N))` of N distinct ints was quadratic (N=3200 spent ~82% of
        // the compile in `const_compound_eq`); the scalar fast path makes it linear.
        Core::ListNew { elems } => build_const_set(db, id, &elems, elem_ty),
        // A runtime list source — building a set from a runtime list needs a runtime dedup loop (a later
        // increment); decline cleanly (a constant list is the corpus shape).
        _ => Core::Poison(Reject::decline(
            "Set.of over a runtime list is not yet built (a constant list literal only)",
        )),
    }
}

/// Build a `Core::SetOf` from statically-enumerated element occurrences, dedup'ing CONSTANT elements at
/// compile (keeping the FIRST occurrence of each value — order-independent set equality makes which copy
/// is kept unobservable; the render sorts). Shared by `Set.of` over a constant list ([`lower_set_of`]) and
/// the first-class `("set" …)` construction (`Resolved::Set`). Linear for scalar elements (canonical
/// `ScalarKey` hash-set); a compound element falls back to pairwise `const_compound_eq` against the other
/// kept compounds. A RUNTIME element value has no const key, so it is kept (the backend's `set-insert`
/// dedups it at run time).
fn build_const_set(db: &mut Db, id: StructId, elems: &[StructId], elem_ty: crate::ty::Ty) -> Core {
    let mut deduped: Vec<StructId> = Vec::with_capacity(elems.len());
    let mut seen_scalars: crate::fxhash::FxHashSet<ScalarKey> = crate::fxhash::FxHashSet::default();
    let mut compounds: Vec<StructId> = Vec::new();
    for &e in elems {
        match scalar_const_key(db, e) {
            Some(key) => {
                if seen_scalars.insert(key) {
                    deduped.push(e);
                }
            }
            None => {
                if !set_has_const_elem(db, &compounds, e) {
                    compounds.push(e);
                    deduped.push(e);
                }
            }
        }
    }
    trace!(target: "rcdzc::fold", node = id.0, elems = deduped.len(), "folds elements to a canonical set");
    Core::SetOf {
        elems: deduped.into(),
        elem_ty,
    }
}

/// Lower `(Set.contains set elem)` — the total membership predicate. FOLD a constant set (`Core::SetOf`)
/// with a constant element to `ConstBool` (by value, `const_compound_eq`), else emit the runtime
/// `Core::SetContains` (a `bool`). A poison propagates.
fn lower_set_contains(db: &mut Db, set: StructId, elem: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, set) {
        return Core::Poison(r);
    }
    if let Core::Poison(r) = core_of(db, elem) {
        return Core::Poison(r);
    }
    // FOLD only when the ENTIRE set is constant (`is_const_value` — every `SetOf` element is const),
    // not merely a `Core::SetOf` shape. A `SetOf` can carry a RUNTIME element (a call/param result that
    // did not fold — `(Set.of (list (f x)))`), and `set_has_const_elem` compares the query only against
    // the CONSTANT elements (`const_compound_eq(runtime, const)` is `None`), so folding such a set would
    // wrongly answer `false` for a query that equals the runtime element at run time (a silent
    // miscompile — `(Set.contains (Set.of (list (add 2 3))) 5)` folded to `false` though `add 2 3 = 5`).
    // A set with any non-constant element must run the real champ membership test (`Core::SetContains`).
    if let Core::SetOf { elems, .. } = core_of(db, set)
        && is_const_value(db, set)
        && is_const_value(db, elem)
    {
        let present = set_has_const_elem(db, &elems, elem);
        trace!(target: "rcdzc::fold", result = present, "Set.contains folds against a constant set");
        return Core::ConstBool(present);
    }
    let Some(elem_ty) = set_elem_type(db, set) else {
        return Core::Poison(Reject::decline(
            "Set.contains operand is not a solved set type",
        ));
    };
    Core::SetContains { set, elem, elem_ty }
}

/// Lower `(Set.to-list set)` → `Core::SetToList`. Always emits the runtime op (no const-fold): the
/// canonical element ORDER is the runtime's sorted walk, so even a NON-EMPTY constant set must go through
/// the runtime to observe that order (mirrors the runtime-element collection-fold rule). The element type
/// comes from the operand set's solved `Ty::Set`, and bakes the shape descriptor the runtime orders by.
///
/// The ONE compile-time fold: a provably-EMPTY constant set (`Core::SetOf` with no elements) folds to the
/// empty `Core::ListNew` — its canonical enumeration is `[]` regardless of element type, so no ordering
/// descriptor is needed. This is also SOUNDNESS-load-bearing: an empty set LITERAL (`Set.of([])`) leaves
/// its element type UNDETERMINED (a free `Ty::Var` — no element ever constrained it), and a var has no
/// orderable shape descriptor. Without this fold `Set.to-list` on such a set declined at the BACKEND
/// ("no orderable descriptor") though the type-checker accepted the program — a check/compile divergence.
/// Folding it here (the element type is irrelevant to an empty enumeration) keeps the emit total.
fn lower_set_to_list(db: &mut Db, set: StructId) -> Core {
    // Bind the operand's core ONCE — `core_of` is a non-trivial lowering pass, so the Poison check and
    // the empty-`SetOf` check reuse the one result rather than re-lowering (Copilot PR #415).
    let set_core = core_of(db, set);
    if let Core::Poison(r) = &set_core {
        return Core::Poison(r.clone());
    }
    // A compile-time-visible EMPTY constant set enumerates to the empty list — no descriptor, no element
    // type needed.
    if let Core::SetOf { elems, .. } = &set_core
        && elems.is_empty()
    {
        trace!(target: "rcdzc::fold", node = set.0, "Set.to-list folds an empty constant set to the empty list");
        return Core::ListNew {
            elems: std::rc::Rc::from([]),
        };
    }
    // A NON-EMPTY CONSTANT set folds to a baked list of its elements in CANONICAL VALUE ORDER — the SAME
    // order the runtime `set-to-list` op produces. This is SOUND (not a presumed CHAMP layout): the runtime
    // op EXPLICITLY re-sorts by the canonical value total order (`value_cmp_shaped`), which the spec pins as
    // MUST-level (`collections-and-text.md` §Set Iteration Is Deterministic — the visit order MUST agree with
    // the canonical byte form), and the compiler's `const_key_order` is the SAME order (confirmed by v-runtime
    // as a contract, not an implementation detail; witnessed by the runtime `set-to-list` corpus vectors). So
    // materializing the elements sorted by `const_key_order` byte-matches the runtime op. This turns a
    // `(const (… Set.to-list …))` DEMAND (which forced the runtime op → declined) into a fold. Elements the
    // canonical order cannot rank as compile-time constants (float / bytes / nested-collection / a runtime
    // element — `const_key_order` returns `None`, EXACTLY the non-orderable classes the runtime op declines
    // too) keep the runtime op. The set already dedups by value (`Set.of`/insert folds), so this only reorders.
    if let Core::SetOf { elems, .. } = &set_core {
        let mut sorted: Vec<StructId> = elems.to_vec();
        // EVERY element must be individually canonically-orderable (a `sort_by` over 0/1 elements never calls
        // the comparator, so the flag alone would miss a lone non-orderable element and materialize it — a
        // divergence from the runtime op, which declines a non-orderable element regardless of count). Probe
        // each element against itself; the sort's flag then catches any residual pairwise `None`.
        let mut orderable = sorted.iter().all(|&e| const_key_order(db, e, e).is_some());
        sorted.sort_by(|&x, &y| {
            const_key_order(db, x, y).unwrap_or_else(|| {
                orderable = false;
                std::cmp::Ordering::Equal
            })
        });
        if orderable {
            trace!(target: "rcdzc::fold", node = set.0, n = sorted.len(), "Set.to-list folds a constant set to a canonically-sorted list");
            return Core::ListNew {
                elems: sorted.into(),
            };
        }
    }
    let Some(elem_ty) = set_elem_type(db, set) else {
        return Core::Poison(Reject::decline(
            "Set.to-list operand is not a solved set type",
        ));
    };
    Core::SetToList { set, elem_ty }
}

/// Synthesize a fresh node carrying `core` with solved type `ty` (its `core`/`ty` columns pre-filled, so
/// it lowers/types directly without re-resolution — the same trick `Bytes.slice`'s fold payload uses).
/// Used by the runtime bin matcher to build the `if`-chain + per-arm predicate out of `Core` directly,
/// and by select's equal-refined-branch collapse to materialize the shared constant.
/// `List.prepend list elem` → insert `elem` at the FRONT. Kept `#[inline(never)]` so its locals stay off
/// `core_of`'s recursive frame (the printer-arm-locals-bloat family — a deep-input decline test overflows
/// the compile thread's stack if the lowering hub's frame grows). FOLD onto a constant list (element at
/// index 0); a runtime list lowers to the dedicated `Core::ListPrepend` (runtime `vec-prepend` op — the
/// front-growth twin of `vec-push`), replacing the old `concat(list-new(elem), list)` path.
#[inline(never)]
fn lower_list_prepend(db: &mut Db, id: StructId, list: StructId, elem: StructId) -> Core {
    match (core_of(db, list), core_of(db, elem)) {
        (Core::Poison(r), _) | (_, Core::Poison(r)) => Core::Poison(r),
        (Core::ListNew { elems }, _) => {
            let mut a = Vec::with_capacity(elems.len() + 1);
            a.push(elem);
            a.extend(elems.iter().copied());
            trace!(target: "rcdzc::fold", node = id.0, len = a.len(), "List.prepend folds onto a constant list");
            Core::ListNew { elems: a.into() }
        }
        // A RUNTIME list operand lowers to the dedicated `vec-prepend` op (front-growth twin of `vec-push`),
        // REPLACING the old `concat(singleton, list)` path — which invoked the full RRB merge per prepend and
        // leaked the superseded front-spine (~17 cells/prepend, the corpus's single biggest leak). `elem`'s
        // boxing is applied by the backend exactly as for `ListPush`.
        _ => Core::ListPrepend { list, elem },
    }
}

/// [PA1a] Whether `head` is (transitively through `Ref`/`Annot`) a PROJECTION out of a compound — a
/// stored fn read via `(. compound i)` / a record field. Used to gate the partial-application keep+lift.
fn head_reached_through_projection(db: &mut Db, head: StructId) -> bool {
    match resolved_of(db, head) {
        Resolved::Proj { .. } => true,
        Resolved::Member { .. } => {
            // A member of a RUNTIME compound (not a prelude op / ctor / a const-folding record).
            crate::eval::meta_apply_of(db, head).is_none()
                && crate::eval::variant_disc_of(db, head).is_none()
        }
        Resolved::Ref { value }
        | Resolved::Annot { expr: value, .. }
        | Resolved::ConstBlock { expr: value } => head_reached_through_projection(db, value),
        _ => false,
    }
}

/// [PA1a] Peel the SYNTACTIC application-head chain (nested `Apply` heads, transparent through `Ref`/
/// `Annot`) WITHOUT fold-through reduction, returning the innermost non-`Apply` head. Unlike `apply_spine`
/// (which β-reduces the bottom to a lambda), this stops at the syntactic head — so a `((. t 0) a) b` chain
/// bottoms at the `(. t 0)` PROJECTION, not the lambda it folds to.
fn syntactic_head(db: &mut Db, head: StructId) -> StructId {
    match resolved_of(db, head) {
        Resolved::Apply { head: inner, .. } => syntactic_head(db, inner),
        Resolved::Ref { value }
        | Resolved::Annot { expr: value, .. }
        | Resolved::ConstBlock { expr: value } => syntactic_head(db, value),
        _ => head,
    }
}

/// [PA1a] Gather the args of a curried application spine LEFT-TO-RIGHT (the deeper `Apply` head's args
/// bind the function's leading params), stopping at the innermost syntactic head. `(((. t 0) a) b)` →
/// `[a, b]`. Transparent through `Ref`/`Annot` (they wrap the same application).
fn syntactic_spine_args(db: &mut Db, id: StructId) -> Vec<StructId> {
    match resolved_of(db, id) {
        Resolved::Apply { head, args } => {
            let args: Vec<StructId> = args.to_vec();
            let mut acc = syntactic_spine_args(db, head);
            acc.extend_from_slice(&args);
            acc
        }
        Resolved::Ref { value }
        | Resolved::Annot { expr: value, .. }
        | Resolved::ConstBlock { expr: value } => syntactic_spine_args(db, value),
        _ => Vec::new(),
    }
}

/// [PA1a] Whether `head` is itself an APPLICATION whose SYNTACTIC spine bottoms at a PROJECTED fn — the
/// current node is `((proj a) b …)`, a CURRIED application of a stored lambda. Its inner `(proj a)` is a
/// partial application that would β-reduce to a dangling residual; the whole spine must instead collapse to
/// one runtime `Core::CallClosure` (the projected element lifts + applies via `call_indirect` at full
/// arity). A DIRECT full application `(proj a)` (head IS the projection, not an application of it) is NOT
/// matched — it folds inline (the capturing-single-closure fold guard, tests.rs:61008 / 09-functions).
fn is_curried_application_of_projected_fn(db: &mut Db, head: StructId) -> bool {
    if !matches!(resolved_of(db, head), Resolved::Apply { .. }) {
        return false;
    }
    let bottom = syntactic_head(db, head);
    head_reached_through_projection(db, bottom)
}

pub(crate) fn synth_core(db: &mut Db, core: Core, ty: crate::ty::Ty) -> StructId {
    let id = db.push_atom(crate::ast::Leaf::Bytes(Vec::new().into())); // placeholder leaf; core/ty are authoritative
    db.core.fill(id, core);
    db.types.fill(id, ty);
    id
}

/// A synthesized `(if cond then_ else_)` occurrence (its `Core::If` + result type pre-filled). The result
/// type is the arm bodies' type (they agree — a well-typed match). Used to chain runtime bin-match arms.
fn synth_if(db: &mut Db, cond: StructId, then_: StructId, else_: StructId) -> StructId {
    let ty = crate::infer::type_of(db, then_);
    synth_core(db, Core::If { cond, then_, else_ }, ty)
}

/// A synthesized `(if cond then_ else_)` that RE-APPLIES the common-constructor / common-operator hoists
/// to itself before materializing — so the hoists COMPOSE when one hoist's differing position is itself
/// a common constructor across the arms. The plain `synth_if` fills a raw `Core::If` that never re-enters
/// the `Resolved::If` fold pipeline, so a hoist that pushes a differing position into a fresh `if` would
/// otherwise leave a nested common constructor un-hoisted:
///   `(if c1 (tuple (Some a) 1) (if c2 (tuple (Some b) 1) (tuple (Some a) 1)))`
/// hoists the outer tuple to `(tuple (if c1 (Some a) (if c2 (Some b) (Some a))) 1)`, but the synthesized
/// field-0 `if` — a nested-if of `Some` — must ALSO hoist to `(Some (if c1 a (if c2 b a)))` (one sum-new,
/// not three). Used at every differing-position site in `hoist_common_ctor`/`hoist_common_arith`.
///
/// Terminates: the re-applied hoists recurse on strictly SMALLER sub-arms (the payloads/operands of the
/// current arms), so the mutual recursion (synth_if_hoisted → hoist_common_* → synth_if_hoisted) bottoms
/// out. The result type is unchanged (a hoist rewrites the head, not the type).
fn synth_if_hoisted(db: &mut Db, cond: StructId, then_: StructId, else_: StructId) -> StructId {
    // JOIN the two branch types, not just `then_`: a bare literal branch carries a DEFERRED width that
    // inference leaves un-ground when the sibling branch pins the concrete width (`(if b (Some 2.0) (Some
    // v0:Float32))` hoists the `Some` out, leaving the payload `(if b 2.0 v0)` — the `2.0` stays
    // `Float(Deferred)`). Typing the hoisted inner node from `then_` alone stamps that Deferred → the emit
    // branchless-select defaults a deferred float to `f64` beside the concrete `f32` sibling → an INVALID
    // `select`. `Ty::join` adopts the fixed width from whichever branch pinned it — the same fix the match
    // `sink_ctor_through_match_arms` payload-typing applies (a genuine two-fixed-width mix is CDZ0201, caught
    // before lowering).
    let ty = crate::infer::type_of(db, then_).join(&crate::infer::type_of(db, else_));
    if let Some(core) = hoist_common_ctor(db, cond, then_, else_)
        .or_else(|| hoist_common_arith(db, cond, then_, else_))
    {
        return synth_core(db, core, ty);
    }
    synth_core(db, Core::If { cond, then_, else_ }, ty)
}

/// The DEPENDENT-SIZE size occurrence `n` of a segment whose byte length is a RUNTIME value naming an
/// earlier segment binder — a `(bytes body n)` (`size: Some(n)`) or a `(utf8 s n)` whose size is a NAME.
/// `None` for a fixed-width segment, a final unsized `(bytes rest)`, or a CONSTANT-size utf8 (whose `C` is
/// in the static prefix, not the `Σn` accounting). This is what unifies utf8 with bytes in the runtime
/// length / offset plumbing — both fold their runtime `n` into `total + Σn` and `off_plus`.
fn bin_dependent_size_occ(db: &Db, seg: &crate::resolved::Segment) -> Option<StructId> {
    use crate::resolved::SegKind;
    match &seg.kind {
        SegKind::Bytes { size: Some(n) } => Some(*n),
        SegKind::Utf8 { size } if db.ast.as_name(*size).is_some() => Some(*size),
        _ => None,
    }
}

/// Build the runtime PREDICATE for a `(bin …)` arm over a runtime `Bytes` `scrutinee`: a boolean `Core`
/// occurrence that holds exactly when the arm matches — `bytes-len(scrutinee) == total_width` AND, for
/// each LITERAL segment, `BinIntRead(that segment) == literal`. All segments are fixed-width ints (the
/// caller guarded that), so the total width is static and each segment's offset is static. A binder
/// segment adds no probe (it binds); a literal segment must be a constant int. Returns the predicate
/// occurrence, or a `Reject` if a literal slot is not a constant integer.
fn build_bin_arm_predicate(
    db: &mut Db,
    scrutinee: StructId,
    segs: &[crate::resolved::Segment],
) -> Result<StructId, Reject> {
    use crate::resolved::SegKind;
    // The fixed BYTE prefix: int segment widths + bit-field bits (a byte-aligned `bits` run — CDZ0220 —
    // contributes a whole number of bytes; summing each field's `k/8` … but a run's fields may not each be
    // byte-aligned, so sum BITS then divide). Sum bits across all `bits` fields and int widths (in bits),
    // then the fixed prefix is that total in bytes (a well-formed bin is byte-aligned overall).
    let total_bits: u32 = segs
        .iter()
        .map(|s| match &s.kind {
            SegKind::Int { width, .. } => (*width as u32) * 8,
            SegKind::Bits { k } => *k,
            // A CONSTANT-size `(utf8 s C)` contributes its `C` bytes to the fixed prefix (`bytes-len ==
            // total + …`). A DEPENDENT-size (name) utf8's size is runtime, so it contributes 0 here and
            // instead enters the `total + Σn` accounting via `dep_seg_indices` (exactly like `(bytes … n)`).
            SegKind::Utf8 { size } => db
                .ast
                .as_int(*size)
                .and_then(|v| v.to_i64())
                .map(|c| c.max(0) as u32 * 8)
                .unwrap_or(0),
            _ => 0,
        })
        .sum();
    let total: u32 = total_bits / 8;
    // Length probe. Whole-scrutinee accounting: a `bin` pattern with no trailing variable segment matches
    // only the EXACT fixed length (`bytes-len == total`); one ending in a final `(bytes rest)` matches any
    // length `>= total` (the fixed int prefix, rest absorbs the remainder); one ending in a DEPENDENT-SIZE
    // `(bytes payload n)` matches EXACTLY `total + n` (n = the runtime value of an earlier int segment).
    // `BytesLen` yields Int64; type both compare operands FIXED Int64 so the literal grounds to i64 (an
    // i32-vs-i64 compare is an invalid module).
    let has_final_rest = matches!(
        segs.last().map(|s| &s.kind),
        Some(SegKind::Bytes { size: None })
    );
    // Collect the DEPENDENT-SIZE segments — `(bytes body n)` at ANY position (§4a). Each contributes its
    // runtime size `n` to the length accounting. For trap safety under a SINGLE floor (below), every size
    // field must sit at a STATIC offset — i.e. within the fixed prefix `total`, so `bytes-len >= total`
    // proves the size read in bounds. A size field at a DYNAMIC offset (a second dependent size before it —
    // an interleaved chain needing incremental flooring) is not yet lowered → decline cleanly. Store each
    // dependent segment's index (for `bin_size_len_read`, which resolves the size field by name).
    let mut dep_seg_indices: Vec<usize> = Vec::new();
    for (i, s) in segs.iter().enumerate() {
        // Both `(bytes body n)` and dependent-size `(utf8 s n)` fold their runtime `n` into `total + Σn`.
        if let Some(n_occ) = bin_dependent_size_occ(db, s) {
            let Some(size_name) = db.ast.as_name(n_occ) else {
                return Err(Reject::decline(
                    "a runtime dependent-size bin match needs a named size field",
                ));
            };
            let Some(j) = segs
                .iter()
                .take(i)
                .position(|t| db.ast.as_name(t.slot) == Some(size_name))
            else {
                return Err(Reject::decline(
                    "a runtime dependent-size bin match size field is not an earlier segment",
                ));
            };
            if bin_static_offset(segs, j).is_none() {
                return Err(Reject::decline(
                    "a runtime bin match with a dynamically-offset dependent size field is not yet lowered",
                ));
            }
            dep_seg_indices.push(i);
        }
    }
    let len_node = synth_core(
        db,
        Core::BytesLen { operand: scrutinee },
        crate::ty::Ty::Int(crate::ty::IntTy::i64()),
    );
    let total_node = synth_core(
        db,
        Core::ConstInt(IntValue::from_i64(total as i64)),
        crate::ty::Ty::Int(crate::ty::IntTy::i64()),
    );
    // The length predicate. With no dependent size it is a single compare: `== total` (whole-scrutinee) or
    // `>= total` (a final `(bytes rest)` absorbs the remainder). With DEPENDENT sizes, the exact byte count
    // accounts for the fixed prefix PLUS each dependent body: `total + Σn`. If a final `(bytes rest)` follows
    // (the §4a non-final shape `(bytes body n) … (bytes rest)`), the rest absorbs whatever is left, so the
    // test relaxes to `bytes-len >= total + Σn`; otherwise (the tail IS a dependent size, or all-fixed) it is
    // exact `bytes-len == total + Σn`. Each size read is a `BinIntRead` at a STATIC offset, which needs the
    // first `total` bytes PRESENT; an UNGUARDED read on a too-short scrutinee TRAPS, where the const-fold
    // reference (`bin_match_decode`) FALLS THROUGH at every overrun. So FLOOR the predicate:
    //   (bytes-len >= total)  AND  (each n >= 0)  AND  (bytes-len {==|>=} total + Σn)
    // with short-circuiting `And { is_and: true }` (`if lhs then rhs else false`), so the size reads + the
    // length compare are reached ONLY when at least `total` bytes are present. A too-short scrutinee fails
    // the floor → the arm predicate is `false` → fall through, matching the reference. (Reviewer finding
    // 2026-07-18 (corpus-bugfix): a truncated length-prefixed frame trapped instead of falling through.)
    let mut pred = if dep_seg_indices.is_empty() {
        synth_core(
            db,
            Core::Compare {
                op: if has_final_rest { Prim::Ge } else { Prim::Eq },
                lhs: len_node,
                rhs: total_node,
            },
            crate::ty::Ty::Bool,
        )
    } else {
        // FLOOR: `bytes-len >= total` — evaluated FIRST, short-circuits the size reads below when false.
        let floor = synth_core(
            db,
            Core::Compare {
                op: Prim::Ge,
                lhs: len_node,
                rhs: total_node,
            },
            crate::ty::Ty::Bool,
        );
        // `each n >= 0` — the const path's `filter(|v| *v >= 0)`; a signed size reading negative is a
        // non-match (else `total + Σn < total` could spuriously satisfy the length compare on a shorter
        // length). AND-chain the per-size guards (each reads a fresh `BinIntRead` — the read is pure).
        let mut nonneg: Option<StructId> = None;
        for &i in &dep_seg_indices {
            let Some(n_occ) = bin_dependent_size_occ(db, &segs[i]) else {
                unreachable!("dep_seg_indices holds only dependent-size segments");
            };
            let Some(n_read) = bin_size_len_read(db, scrutinee, segs, i, n_occ) else {
                return Err(Reject::decline(
                    "a runtime dependent-size bin match needs a fixed-int size segment at a static offset",
                ));
            };
            let zero_node = synth_core(
                db,
                Core::ConstInt(IntValue::from_i64(0)),
                crate::ty::Ty::Int(crate::ty::IntTy::i64()),
            );
            let g = synth_core(
                db,
                Core::Compare {
                    op: Prim::Ge,
                    lhs: n_read,
                    rhs: zero_node,
                },
                crate::ty::Ty::Bool,
            );
            nonneg = Some(match nonneg {
                None => g,
                Some(prev) => synth_core(
                    db,
                    Core::And {
                        lhs: prev,
                        rhs: g,
                        is_and: true,
                    },
                    crate::ty::Ty::Bool,
                ),
            });
        }
        // `total + Σn` — re-read each `n` for the sum (fresh `BinIntRead`s; the backend materializes each
        // independently, matching the literal-probe reads).
        let mut rhs = total_node;
        for &i in &dep_seg_indices {
            let Some(n_occ) = bin_dependent_size_occ(db, &segs[i]) else {
                unreachable!("dep_seg_indices holds only dependent-size segments");
            };
            let Some(n_read) = bin_size_len_read(db, scrutinee, segs, i, n_occ) else {
                return Err(Reject::decline(
                    "a runtime dependent-size bin match needs a fixed-int size segment at a static offset",
                ));
            };
            rhs = synth_core(
                db,
                Core::Arith {
                    op: Prim::Add,
                    lhs: rhs,
                    rhs: n_read,
                },
                crate::ty::Ty::Int(crate::ty::IntTy::i64()),
            );
        }
        // `bytes-len == total + Σn` (exact) or `>= total + Σn` (a final rest absorbs the remainder).
        let len_test = synth_core(
            db,
            Core::Compare {
                op: if has_final_rest { Prim::Ge } else { Prim::Eq },
                lhs: len_node,
                rhs,
            },
            crate::ty::Ty::Bool,
        );
        // `(Σ n>=0) AND len_test` — the guarded dependent test.
        let guarded = match nonneg {
            None => len_test,
            Some(nn) => synth_core(
                db,
                Core::And {
                    lhs: nn,
                    rhs: len_test,
                    is_and: true,
                },
                crate::ty::Ty::Bool,
            ),
        };
        // `floor AND (Σ n>=0 AND len_test)` — floor short-circuits the whole dependent read chain.
        synth_core(
            db,
            Core::And {
                lhs: floor,
                rhs: guarded,
                is_and: true,
            },
            crate::ty::Ty::Bool,
        )
    };
    // Per LITERAL segment: `<read> == literal`. A binder slot (a bare name) adds no probe. An INT literal
    // reads a `BinIntRead` at its static byte offset; a BIT-FIELD literal reads its run + shift+mask (the
    // same extraction the binder decode uses, `bin_bitfield_run`), so a `(bin (bits 1 1) (u8 x))`-style tag
    // dispatches on the sub-byte field. Offsets come from `bin_static_offset`/`bin_bitfield_run`, not a
    // running counter, so a bit-field run does not desync the following int segment's offset.
    for (i, seg) in segs.iter().enumerate() {
        // A bare-name slot is a binder, not a literal probe.
        if db.ast.as_name(seg.slot).is_some() {
            continue;
        }
        let read = match &seg.kind {
            SegKind::Int { width, signed } => {
                // The offset may be dynamic (a literal int after a dependent-size segment — §4a). The read is
                // TRAP-SAFE: the length predicate (`bytes-len {==|>=} total + Σn`) is ANDed BEFORE this probe
                // and short-circuits, so `total + off_plus + width <= total + Σn <= bytes-len` holds here.
                let Some((byte_offset, off_plus)) = bin_dynamic_offset(db, scrutinee, segs, i)
                else {
                    return Err(Reject::decline(
                        "a runtime bin literal int segment after a non-final unsized bytes / utf8 segment is not yet probed",
                    ));
                };
                // `BinIntRead` always emits an i64; type it FIXED Int64 so the compare's `operand_int_ty`
                // picks width 64 and grounds the literal operand to i64 (not a default-narrow i32).
                synth_core(
                    db,
                    Core::BinIntRead {
                        bytes: scrutinee,
                        byte_offset,
                        off_plus,
                        width: *width,
                        signed: *signed,
                        little_endian: seg.little_endian,
                    },
                    crate::ty::Ty::Int(crate::ty::IntTy::i64()),
                )
            }
            // A LITERAL bit-field probe: read its run + shift+mask, the same extraction the binder decode
            // does. `bin_bitfield_run` gives (run_byte_off, run_bits, field_bit_pos, k).
            SegKind::Bits { .. } => {
                let Some((run_byte_off, run_bits, field_bit_pos, k)) = bin_bitfield_run(segs, i)
                else {
                    return Err(Reject::decline(
                        "a runtime bin literal bit-field needs a byte-aligned run of ≤64 bits",
                    ));
                };
                let run_read = synth_core(
                    db,
                    Core::BinIntRead {
                        bytes: scrutinee,
                        byte_offset: run_byte_off,
                        // A bit-field run declines on any preceding dependent-size segment, so static.
                        off_plus: None,
                        width: (run_bits / 8) as u8,
                        signed: false,
                        little_endian: false,
                    },
                    crate::ty::Ty::Int(crate::ty::IntTy::i64()),
                );
                let shift = run_bits - field_bit_pos - k;
                let shifted = if shift == 0 {
                    run_read
                } else {
                    let shift_node = synth_core(
                        db,
                        Core::ConstInt(IntValue::from_i64(shift as i64)),
                        crate::ty::Ty::Int(crate::ty::IntTy::i64()),
                    );
                    synth_core(
                        db,
                        Core::Arith {
                            op: Prim::Shr,
                            lhs: run_read,
                            rhs: shift_node,
                        },
                        crate::ty::Ty::Int(crate::ty::IntTy::i64()),
                    )
                };
                let mask_val: i64 = if k >= 63 { -1 } else { (1i64 << k) - 1 };
                let mask_node = synth_core(
                    db,
                    Core::ConstInt(IntValue::from_i64(mask_val)),
                    crate::ty::Ty::Int(crate::ty::IntTy::i64()),
                );
                synth_core(
                    db,
                    Core::Arith {
                        op: Prim::BitAnd,
                        lhs: shifted,
                        rhs: mask_node,
                    },
                    crate::ty::Ty::Int(crate::ty::IntTy::i64()),
                )
            }
            // A literal bytes/utf8 segment is not probed here (a final bytes segment binds, it does not
            // carry a literal in this path).
            _ => continue,
        };
        // The literal value the segment must equal.
        let lit = match core_of(db, seg.slot) {
            Core::ConstInt(v) => v,
            Core::Poison(r) => return Err(r),
            _ => {
                return Err(Reject::decline(
                    "a runtime bin pattern literal segment is not a constant integer",
                ));
            }
        };
        let lit_node = synth_core(
            db,
            Core::ConstInt(lit),
            crate::ty::Ty::Int(crate::ty::IntTy::i64()),
        );
        let eq = synth_core(
            db,
            Core::Compare {
                op: Prim::Eq,
                lhs: read,
                rhs: lit_node,
            },
            crate::ty::Ty::Bool,
        );
        pred = synth_core(
            db,
            Core::And {
                lhs: pred,
                rhs: eq,
                is_and: true,
            },
            crate::ty::Ty::Bool,
        );
    }
    // Each `(utf8 s SIZE)` segment (constant or dependent size, any position) matches only if its byte range
    // is WELL-FORMED UTF-8 — decode the range to `Option String` and AND `is-some` into the predicate
    // (ill-formed bytes decode to `None` → the arm does not match → fall through, exactly as the const
    // path's `str::from_utf8(..).ok()?`). `is-some` is the RHS of the AND, so it is reached ONLY after the
    // length test (`bytes-len {==|>=} total + Σn`) holds, which puts the range in bounds. The binder `s`
    // reads the payload independently (a `SumExpect` in `decode_bin_field_runtime`); this decode + that one
    // are two independent reads of the same range.
    for (i, seg) in segs.iter().enumerate() {
        if !matches!(&seg.kind, SegKind::Utf8 { .. }) {
            continue;
        }
        let Some((opt_node, disc_some, _)) = bin_utf8_decode(db, scrutinee, segs, i) else {
            return Err(Reject::decline(
                "a runtime bin utf8 segment needs a computable byte range (offset + size)",
            ));
        };
        let is_some = bin_option_is_some(db, opt_node, disc_some);
        pred = synth_core(
            db,
            Core::And {
                lhs: pred,
                rhs: is_some,
                is_and: true,
            },
            crate::ty::Ty::Bool,
        );
    }
    Ok(pred)
}

fn lower_conversion(db: &mut Db, id: StructId, op: Prim, args: &[StructId]) -> Core {
    if args.len() != 1 {
        return Core::Poison(Reject::coded(
            Code::Malformed,
            format!("{} takes exactly 1 operand", intrinsic_name(op)),
        ));
    }
    // The target width/signedness = the conversion node's solved type (an integer). If it is not an
    // integer type (an unresolved/absurd target), decline rather than guess.
    let target = match crate::infer::type_of(db, id) {
        crate::ty::Ty::Int(it) => it,
        _ => {
            return Core::Poison(Reject::decline(
                "a conversion target is not a definite integer type",
            ));
        }
    };
    let (signed, width) = (target.ground_signed(), target.ground_width());
    match core_of(db, args[0]) {
        Core::ConstInt(v) => match op {
            // `T.wrap` — truncate to the target width at arbitrary precision (total, never traps).
            Prim::Wrap => {
                let wrapped = v.wrap_to(signed, width);
                trace!(target: "rcdzc::fold", op = intrinsic_name(op), signed, width, "folded constant wrap");
                Core::ConstInt(wrapped)
            }
            // `T.of` — the CHECKED conversion: in range → the value UNCHANGED at the target type; out of
            // range → TRAP (numeric-model.md §A Conversion Between Integer Types Is Explicit). The value is
            // not altered when it fits (`(UInt8.of 200) = 200`), distinguishing it from the truncating
            // `wrap` (which would keep the low bits of an out-of-range value instead of trapping).
            _ => {
                if v.fits_width(signed, width) {
                    trace!(target: "rcdzc::fold", op = intrinsic_name(op), signed, width, "folded checked conversion (in range)");
                    Core::ConstInt(v)
                } else {
                    // A CONSTANT operand that provably exceeds the target range is a statically-ill-formed
                    // conversion — the compiler already knows at compile time it cannot succeed. Reject it
                    // CDZ0302 (integer does not fit the target width), consistent with `(: 128 Int8)` and
                    // the const-overflow arithmetic fold, rather than emitting a RUNTIME trap for a
                    // statically-impossible conversion. (A RUNTIME `T.of` whose value is unknown until run
                    // time still traps at run time — the branch below declines it to the checked emit; only
                    // a compile-time-KNOWN out-of-range constant is rejected up front here.)
                    trace!(target: "rcdzc::fold", op = intrinsic_name(op), signed, width, "checked conversion of an out-of-range constant → CDZ0302 reject");
                    // Name the offending VALUE, the target width TYPE, and the VALID RANGE — matching the
                    // annotation-position CDZ0302 ("the valid range is 0..=255") rather than the terse
                    // "(unsigned 8-bit)". A checked conversion `(UInt8.of 300)` is the same over-range fault
                    // as the annotation `(: 300 UInt8)`, so it reads the same. Reuses `width_module_spelling`
                    // (bound name `UInt8` for an aliased width, `(UInt k)` ctor form otherwise) +
                    // `int_width_range` (min..=max) — the M191 bin-segment helpers.
                    let ty = crate::infer::width_module_spelling(&crate::ty::IntTy::fixed(
                        signed, width,
                    ));
                    let val = v.to_decimal_string();
                    let message = match crate::infer::int_width_range(signed, width) {
                        Some(range) => format!(
                            "the value {val} does not fit the checked conversion's target type {ty} \
                             (the valid range is {range}) — `.of` traps out of range; use `.wrap` to \
                             truncate, or convert a value that fits"
                        ),
                        None => format!(
                            "the value {val} does not fit the checked conversion's target type {ty} \
                             — `.of` traps out of range; use `.wrap` to truncate, or convert a value \
                             that fits"
                        ),
                    };
                    Core::Poison(Reject::coded(Code::IntOutOfRange, message))
                }
            }
        },
        Core::Poison(r) => Core::Poison(r),
        // A runtime operand: emit the mask-and-reinterpret at selection (the target is read off this
        // node's solved type there, the same `type_of(id)` used here).
        _ => {
            // `Int64.of b` / `(UInt N).of b` on a RUNTIME `BigInt` `b` — the checked narrowing back to a
            // fixed width, emitted as the runtime `bigint-to-i64-checked` (traps out of range at run time,
            // B3b). The runtime op checks the i64 range; a narrower target's over-range value is a runtime
            // concern the op will refine later (a constant over-range narrowing is already CDZ0302 at
            // compile time, B1). Checked before the generic runtime-of decline below.
            if matches!(op, Prim::CheckedOf)
                && matches!(crate::infer::type_of(db, args[0]), crate::ty::Ty::BigInt)
            {
                return Core::BigIntToI64 { operand: args[0] };
            }
            // `T.of` on a RUNTIME integer whose SOURCE type provably fits the target is TOTAL — no source
            // value can be out of range, so no trap can ever fire and the conversion is a pure
            // representation change IDENTICAL to `T.wrap` (extend-and-reinterpret; the target-width
            // normalize is a no-op for a same-sign widening, or a value-preserving reshape for a
            // sign-change that still fits). So emit it as the already-built `Prim::Wrap` convert (the
            // widening `Int64.of x:UInt8`, `UInt16.of x:UInt8`, `Int64.of x:Int8`, …). This is SOUND
            // precisely because totality is proven: a checked `of` and a truncating `wrap` differ ONLY on
            // an out-of-range value, and there is none. A NON-total runtime `of` (a genuine narrowing, or
            // `Int8.of x:UInt8` where 255 overflows) still declines below — it needs the range-check-then-
            // trap emit that is a later increment (emitting `wrap` there WOULD be a miscompile: silently
            // keeping the low bits where `of` must trap).
            if matches!(op, Prim::CheckedOf)
                && let crate::ty::Ty::Int(src) = crate::infer::type_of(db, args[0])
                && src.fits_within(crate::ty::IntTy::fixed(signed, width))
            {
                trace!(target: "rcdzc::lower", op = intrinsic_name(op), signed, width, "runtime T.of is provably total → emit as a widening Wrap convert");
                return Core::Convert {
                    op: Prim::Wrap,
                    operand: args[0],
                };
            }
            // Even when the SOURCE TYPE does not statically fit (a genuine narrowing like `Int64.of` of a
            // `UInt64`, or `UInt64.of` of an `Int64`), the conversion is TOTAL when the operand's PROVABLE
            // VALUE RANGE lies within the target's `[min, max]` — the same interval/value-facts lattice the
            // guard-elision checks consult. `(Int64.of (% n 1000))` over `n : UInt64` is total because
            // `(% n 1000) ∈ [0, 999]` fits `Int64` regardless of `n`'s wide unsigned type; no source value
            // can be out of range, so no trap can fire and `of` == `wrap` (a value in the target range is
            // unchanged by the target-width reinterpret, exactly `value_range_within`'s truncation-elision
            // rationale). SOUND precisely because totality is PROVEN per-value here, not merely per-type. A
            // target with no finite upper bound (unsigned-64) fits any provably-NONNEG operand (its max
            // `2^64-1` is the representable ceiling). Checked AFTER the static-type fit (that path is
            // cheaper and subsumes the always-total widenings); a value whose range is unknown or exceeds
            // the target still declines below to the not-yet-built checked-narrowing emit.
            if matches!(op, Prim::CheckedOf)
                && let Some((Some(tlo), thi)) =
                    resolved_int_bounds(crate::ty::IntTy::fixed(signed, width))
            {
                let fits = match thi {
                    Some(thi) => value_range_within(db, args[0], tlo, thi),
                    // Unsigned-64 target (no finite i64 max): any provably-nonneg value fits `[0, 2^64-1]`.
                    None => tlo <= 0 && value_provably_nonneg(db, args[0]),
                };
                if fits {
                    trace!(target: "rcdzc::lower", op = intrinsic_name(op), signed, width, "runtime T.of is value-range-provably total → emit as a Wrap convert");
                    return Core::Convert {
                        op: Prim::Wrap,
                        operand: args[0],
                    };
                }
            }
            // A NON-total `T.of` on a RUNTIME operand: emit a RANGE-CHECK-then-TRAP, composed from existing
            // Core (no new variant). The provably-total fast-paths above already emitted `Wrap` (an in-range
            // value is unchanged by the target-width reinterpret), so reaching here the operand CAN be out of
            // range and MUST trap on it (a bare `Wrap` here would be a miscompile — `wrap`'s truncate where
            // `of` must trap). Compose: nested `if <operand out of [tmin,tmax]> then trap else wrap(operand)`.
            // The bound consts are typed as the OPERAND's type so `Core::Compare` derives the right
            // signedness (`operand_int_ty`); an UNSIGNED operand (min 0) needs only the upper check (it can
            // never underflow a `tmin <= 0` target), a SIGNED operand needs lower + upper. High-traffic:
            // `Int64.of` of a `u64` host result (trap iff `>u i64::MAX`), `UInt64.of` of an `Int64` (trap iff
            // `<s 0`); also the narrower cross-width narrowings.
            if matches!(op, Prim::CheckedOf) {
                let operand = args[0];
                let src_ty = crate::infer::type_of(db, operand);
                let target_it = crate::ty::IntTy::fixed(signed, width);
                let target_ty = crate::ty::Ty::Int(target_it);
                let src_signed = matches!(&src_ty, crate::ty::Ty::Int(it) if it.ground_signed());
                if matches!(&src_ty, crate::ty::Ty::Int(_))
                    && let Some((tmin, tmax)) = resolved_int_bounds(target_it)
                {
                    // The out-of-range checks (`Bool` occurrences): upper (`operand > tmax`) for any operand;
                    // lower (`operand < tmin`) only for a signed operand (an unsigned one is `>= 0 >= tmin`).
                    let mut conds: Vec<StructId> = Vec::new();
                    if let Some(thi) = tmax {
                        let bound =
                            synth_core(db, Core::ConstInt(IntValue::from_i64(thi)), src_ty.clone());
                        conds.push(synth_core(
                            db,
                            Core::Compare {
                                op: Prim::Gt,
                                lhs: operand,
                                rhs: bound,
                            },
                            crate::ty::Ty::Bool,
                        ));
                    }
                    if src_signed && let Some(tlo) = tmin {
                        let bound =
                            synth_core(db, Core::ConstInt(IntValue::from_i64(tlo)), src_ty.clone());
                        conds.push(synth_core(
                            db,
                            Core::Compare {
                                op: Prim::Lt,
                                lhs: operand,
                                rhs: bound,
                            },
                            crate::ty::Ty::Bool,
                        ));
                    }
                    if !conds.is_empty() {
                        // else (in range) = the target-width reinterpret (`Wrap`), identity for an in-range value.
                        let mut else_id = synth_core(
                            db,
                            Core::Convert {
                                op: Prim::Wrap,
                                operand,
                            },
                            target_ty.clone(),
                        );
                        // Nest one `if <check> then trap else …` per check; the last becomes THIS node's Core.
                        for &cond in &conds[..conds.len() - 1] {
                            let trap = synth_core(db, Core::Trap, target_ty.clone());
                            else_id = synth_core(
                                db,
                                Core::If {
                                    cond,
                                    then_: trap,
                                    else_: else_id,
                                },
                                target_ty.clone(),
                            );
                        }
                        let last = *conds.last().expect("non-empty");
                        let trap = synth_core(db, Core::Trap, target_ty.clone());
                        let if_core = Core::If {
                            cond: last,
                            then_: trap,
                            else_: else_id,
                        };
                        // The compose names `operand` in each range-check compare AND the `Wrap` else — so
                        // materialize a HOST-LIFTED operand ONCE (else the host effect fires per reference:
                        // breaker adv-tof-host-u64 saw `Int64.of (host-u64 1000)` spuriously trap on the
                        // range-check re-invoking the host call and draining its lone queued response).
                        return materialize_host_operands_once(db, id, &[operand], if_core);
                    }
                }
                // A non-int source or unresolved target bounds can't be range-checked here — decline honestly.
                return Core::Poison(Reject::decline(
                    "a runtime checked integer conversion (T.of) that could be out of range is not yet emitted (convert a constant, widen instead of narrow, or use T.wrap)",
                ));
            }
            if is_scalar(db, args[0]) {
                // WRAP COMPOSITION: `T.wrap(U.wrap(x))` where this outer target width `N` is ≤ the inner
                // wrap's target width `M` — the inner wrap keeps the low `M` bits, and the outer keeps the
                // low `N ≤ M`, which are UNCHANGED by the inner wrap. So the inner wrap is redundant:
                // `Int8.wrap(Int16.wrap x)` = `Int8.wrap x`. Reach past the inner Convert to ITS operand,
                // eliding one mask-and-reinterpret. Verified `wrap∘wrap == outer wrap` for N ≤ M, any
                // signedness. (`N > M` is NOT redundant — the inner narrowing already discarded bits the
                // wider outer wrap cannot recover, so it stays.)
                if matches!(op, Prim::Wrap)
                    && let Core::Convert {
                        op: Prim::Wrap,
                        operand: inner,
                    } = core_of(db, args[0])
                    && let crate::ty::Ty::Int(inner_ty) = crate::infer::type_of(db, args[0])
                    && inner_ty.ground_width() >= width
                {
                    trace!(target: "rcdzc::fold", node = id.0, outer = width, inner = inner_ty.ground_width(), "wrap∘wrap: inner wrap subsumed by the narrower/equal outer");
                    return Core::Convert { op, operand: inner };
                }
                trace!(target: "rcdzc::lower", op = intrinsic_name(op), signed, width, "conversion stays runtime (scalar operand)");
                Core::Convert {
                    op,
                    operand: args[0],
                }
            } else {
                Core::Poison(Reject::decline(
                    "a conversion of a non-scalar operand has no meaning",
                ))
            }
        }
    }
}

/// Whether `ty` is an ORDERABLE COMPOUND — a tuple/record/list/sum whose leaves each offer a blessed total
/// order, so a runtime `<`/`<=`/`>`/`>=` on it routes to `Core::ValueCmp` (the `value-cmp` walk). Recursive:
/// a leaf is orderable iff Int/Bool/Unit/String/Symbol/BigInt/Rational; a Float/Float32/Bytes/Char/Set/Map
/// leaf is NOT (core-semantics.md — floats are IEEE-partial §319; no blessed Bytes/Set/Map/Char order), so a
/// compound containing one is NOT orderable (the whole compound declines). Requires the ROOT to be a genuine
/// COMPOUND (a bare scalar/String takes the scalar/StrCmp path, not here). Mirrors `ty_heap_walkable`'s
/// structure but with the ORDER-offering leaf set (v-inference's ruling), not the equality-walkable one.
//= spec/capabilities/core-semantics.md#compound-ordering-is-lexicographic
//# A compound value — a tuple, a record, a list, or a sum — MUST offer a total order exactly when every one of its component types offers a total order.
//= spec/capabilities/core-semantics.md#compound-ordering-is-lexicographic
//# A compound any of whose component types does not, transitively, offer a total order MUST NOT be treated as offering one; in particular a floating-point component makes the compound unordered, because a floating-point type offers only the IEEE partial order.
fn is_orderable_compound(db: &mut Db, ty: &crate::ty::Ty) -> bool {
    use crate::ty::Ty;
    // The root must be a compound; a bare orderable leaf is handled by the scalar / StrCmp paths.
    matches!(
        ty,
        Ty::Tuple(_) | Ty::Record(_) | Ty::List(_) | Ty::Sum { .. }
    ) && orderable_leaf_or_compound(db, ty, false, &mut Vec::new())
}

/// The recursive orderability predicate: a leaf offers a blessed total order, or a compound whose every
/// component is itself orderable. A recursive sum closes on `seen` (a self-referential type is orderable iff
/// its payloads are — the cycle doesn't change that).
///
/// `float_ok` selects WHICH order is being asked for. A FLOAT has NO blessed NUMERIC total order (IEEE `<`
/// is partial — NaN is unordered), but it DOES have a canonical-BYTE total order (its bit pattern as an
/// unsigned int: NaN collapsed on construction, ±0.0 distinct) — the order `value_cmp_shaped` uses for a
/// float Set.to-list / Map.to-list key. So:
///  - `float_ok = false` (the `<` / `Core::ValueCmp` numeric path): a float leaf is NOT orderable — decline,
///    matching the IEEE partial-order carve-out.
///  - `float_ok = true` (the Set/Map `shape_of` to-list-enumeration path): a BARE-ROOT float leaf IS
///    orderable by its canonical bytes, so a float element/key admits the deterministic enumeration order
///    (collections-and-text.md #Set Iteration Is Deterministic). Bytes/Char/Set/Map stay non-orderable in
///    BOTH modes.
///
/// KEYSTONE: `float_ok` applies to a BARE-ROOT float ONLY — it does NOT propagate into a COMPOUND's components. A
/// float leaf INSIDE a tuple/list/record/sum makes the compound un-orderable per 03:626 / §319 (a compound
/// is ordered EXACTLY WHEN every component offers a TOTAL order, and a float offers only the IEEE partial
/// order — the canonical-byte order is the enumeration convenience for a bare float, not a blessed total
/// order a compound's lexicographic walk can compose). The runtime enforces exactly this asymmetry:
/// `compare_scalar_leaf`'s bare `Shape::Float` orders by `to_bits`, but the compound work-stack walk
/// (`value_cmp_shaped`) returns `None` for a `Shape::Float` leaf mid-walk. So admitting a float-in-compound
/// here built a descriptor the runtime's compound sort then declined → `op_set_to_list` yielded an EMPTY
/// list (SILENT DATA LOSS) instead of a clean compile-time decline. Recursing the compound arms with
/// `float_ok = false` makes `Set/Map.to-list` over a float-containing compound DECLINE (uniform with `<`
/// and both backends), matching 03:626 — a bare float set still enumerates (the root keeps `float_ok`).
fn orderable_leaf_or_compound(
    db: &mut Db,
    ty: &crate::ty::Ty,
    float_ok: bool,
    seen: &mut Vec<crate::ast::StructId>,
) -> bool {
    use crate::ty::Ty;
    match ty {
        // Blessed-order leaves. Bytes joins these (§order): a Bytes value has a TOTAL order —
        // lexicographic over its UNSIGNED byte values — exactly like String/Symbol (both are byte leaves),
        // so it is orderable in BOTH modes (`<`/`ValueCmp` numeric AND to-list enumeration) and, unlike a
        // float, composes soundly INSIDE a compound. The runtime's `compare_scalar_leaf`/`value_cmp_shaped`
        // Bytes arms (over the flattened `raw` slice) realize this order; they MUST stay in lockstep with
        // this guard (the orderable-guard/shape_of divergence trap).
        //= spec/capabilities/core-semantics.md#ordering-where-offered-is-total
        //# A `Bytes` value MUST offer a total order that is lexicographic over its unsigned byte values: comparing bytes from the first, the first differing byte is decisive under the numeric order of the two bytes taken as unsigned, and a byte sequence that is a proper prefix of another MUST compare less than that longer sequence.
        Ty::Int(_)
        | Ty::Bool
        | Ty::Unit
        | Ty::String
        | Ty::Symbol
        | Ty::BigInt
        | Ty::Rational
        | Ty::Char
        | Ty::Bytes => true,
        // A float is orderable ONLY by canonical bytes as a BARE ROOT (to-list), never by numeric `<` and
        // never INSIDE a compound — see `float_ok` + the doc above.
        Ty::Float(_) => float_ok,
        // Non-orderable leaves in EITHER mode — Set/Map (no blessed order at all).
        Ty::Set(_) | Ty::Map(..) => false,
        // COMPOUND arms recurse with `float_ok = false`: a float component makes the compound un-orderable
        // (03:626 / §319), regardless of the caller's bare-root float mode.
        Ty::Tuple(elems) => elems
            .iter()
            .all(|e| orderable_leaf_or_compound(db, e, false, seen)),
        Ty::List(elem) => orderable_leaf_or_compound(db, elem, false, seen),
        Ty::Record(fields) => {
            let vals: Vec<Ty> = fields.values().cloned().collect();
            vals.iter()
                .all(|v| orderable_leaf_or_compound(db, v, false, seen))
        }
        // A sum — orderable iff every variant's payload type is. Recursive sum broken by `seen`. Mirrors
        // `ty_heap_walkable`'s Sum arm: walk each variant's ctor, resolve its payload at this instantiation.
        Ty::Sum { decl, .. } => {
            if seen.contains(decl) {
                return true; // recursive back-edge — already being checked
            }
            seen.push(*decl);
            let variant_count = db.type_decl_by_occ(*decl).map(|t| t.variants.len());
            let mut ok = variant_count.is_some();
            if let Some(vc) = variant_count {
                for disc in 0..vc {
                    let ctor = db
                        .type_decl_by_occ(*decl)
                        .and_then(|t| t.variants.get(disc))
                        .and_then(|v| v.ctor);
                    // A nullary variant (no ctor) is a bare discriminant — orderable. A payload-carrying
                    // variant's payload type must itself be orderable.
                    if let Some(ctor) = ctor
                        && let Some(payload_ty) =
                            crate::infer::payload_ty_at_instantiation(db, ctor, ty)
                        // `false`: a float payload INSIDE a sum makes it un-orderable (compound carve-out,
                        // 03:626) — same as the tuple/list/record arms above.
                        && !orderable_leaf_or_compound(db, &payload_ty, false, seen)
                    {
                        ok = false;
                        break;
                    }
                }
            }
            seen.pop();
            ok
        }
        // Anything else (a bare var, a function, Any, a nominal we can't resolve) — not orderable.
        _ => false,
    }
}

/// Whether `ty` is a LIST(-containing) compound that `=` must compare with the descriptor-guided
/// `value-eq-shaped` walk (`Core::ValueEqShaped`) rather than the physical `champ_eq` `value-eq` or the
/// `value-cmp`-based `Core::ValueCmp{op:Eq}`. Reached ONLY after both cheaper arms declined: it requires
///  (1) the root CONTAINS a `List` — a `List` is an RRB vector that is element-canonical but NOT
///      shape-canonical (a concat-built and a push-built equal list differ in bytes), so `value-eq`'s
///      physical byte-walk is unsound (`ty_heap_walkable`/`compound_eq_heap_walkable` return false for a
///      list), AND
///  (2) it is NOT orderable (`is_orderable_compound` false — it carries a FLOAT/BYTES leaf) — an
///      all-orderable list-containing compound already took `Core::ValueCmp{op:Eq}`, AND
///  (3) every leaf is equality-comparable by the shaped walk — a blessed-Eq scalar (Int/Bool/Unit/String/
///      Symbol/BigInt/Rational) OR a FLOAT/BYTES leaf (byte-canonical equality — the walk's key difference
///      from `value-cmp`, which declines a float since it has equality but no total ORDER) — through
///      tuple/record/list/nominal/qty compounds. A Sum/Map/Set leaf is NOT walked by this slice (a runtime
///      Map/Set `=` is canonical-by-construction and already takes `value-eq`; a Sum with a float leaf is a
///      later increment). Mirrors the rust backend's `ty_float_walkable` domain exactly, so both backends
///      render the SAME set of `Core::ValueEqShaped` types.
///= spec/capabilities/collections-and-text.md#a-list-is-an-ordered-homogeneous-sequence
///# Two lists MUST be equal exactly when they have equal elements in the same order.
fn is_value_eq_shaped_compound(db: &mut Db, ty: &crate::ty::Ty) -> bool {
    // (1) must contain a List, and (3) every leaf equality-walkable. `is_orderable_compound` false (2) is
    // checked at the call site (this arm is reached only after the orderable-Eq arm declined).
    ty_contains_list(db, ty, &mut Vec::new()) && eq_shaped_walkable(db, ty, &mut Vec::new())
}

/// Whether `ty` mentions a `List` anywhere in its structure (the trigger for the shaped equality walk — a
/// bare list, or a list nested in a tuple/record/nominal/qty/SUM). A SUM is descended (each variant's payload
/// via `payload_ty_at_instantiation`, recursion-guarded on `seen`) so an `Ast`-shaped sum whose `List`
/// variant carries `(List Ast)` triggers the walk. Map/Set are not descended (a runtime Map/Set is
/// canonical-by-construction and takes `value-eq`).
fn ty_contains_list(db: &mut Db, ty: &crate::ty::Ty, seen: &mut Vec<crate::ast::StructId>) -> bool {
    use crate::ty::Ty;
    match ty {
        Ty::List(_) => true,
        Ty::Tuple(elems) => {
            let elems = elems.to_vec();
            elems.iter().any(|e| ty_contains_list(db, e, seen))
        }
        Ty::Record(fields) => {
            let vals: Vec<Ty> = fields.values().cloned().collect();
            vals.iter().any(|v| ty_contains_list(db, v, seen))
        }
        Ty::Nominal { inner, .. } => {
            let inner = (**inner).clone();
            ty_contains_list(db, &inner, seen)
        }
        Ty::Qty { inner, .. } => {
            let inner = (**inner).clone();
            ty_contains_list(db, &inner, seen)
        }
        // A SUM contains a List iff any variant's payload does. Recursive sum broken by `seen` (a
        // self-referential sum whose ONLY list is its own recursive payload — e.g. `Ast.List (List Ast)` —
        // is reached via the List arm before the back-edge, so the guard returning false here is correct:
        // the back-edge itself adds no new list). Mirrors `orderable_leaf_or_compound`'s Sum descent.
        Ty::Sum { decl, .. } => {
            if seen.contains(decl) {
                return false;
            }
            seen.push(*decl);
            let mut found = false;
            if let Some(vc) = db.type_decl_by_occ(*decl).map(|t| t.variants.len()) {
                for disc in 0..vc {
                    let ctor = db
                        .type_decl_by_occ(*decl)
                        .and_then(|t| t.variants.get(disc))
                        .and_then(|v| v.ctor);
                    if let Some(ctor) = ctor
                        && let Some(payload_ty) =
                            crate::infer::payload_ty_at_instantiation(db, ctor, ty)
                        && ty_contains_list(db, &payload_ty, seen)
                    {
                        found = true;
                        break;
                    }
                }
            }
            seen.pop();
            found
        }
        _ => false,
    }
}

/// The recursive equality-walkability predicate for `Core::ValueEqShaped` — a leaf is a blessed-Eq scalar OR
/// a FLOAT/BYTES leaf (byte-canonical equality), and a compound (tuple/record/list/nominal/qty/SUM) is
/// walkable iff every component is. A SUM is descended (each variant's payload, recursion-guarded on `seen`)
/// so an `Ast`-shaped sum whose payloads carry a `List` and/or a `Float` routes here — the runtime
/// `value_eq_shaped` walk already has a `Shape::Sum` arm (disc-compare then payload descend), and the rust
/// `emit_value_eq_walk` gains a matching `Ty::Sum` arm. Map/Set/Char/Fn leaves are NOT walked (a Map/Set is
/// canonical-by-construction and takes `value-eq`). Mirrors the rust `ty_float_walkable` domain so both
/// backends agree on which types route here.
fn eq_shaped_walkable(
    db: &mut Db,
    ty: &crate::ty::Ty,
    seen: &mut Vec<crate::ast::StructId>,
) -> bool {
    use crate::ty::Ty;
    match ty {
        // Blessed-Eq scalar leaves + FLOAT/BYTES (byte-canonical equality — nan==nan, -0.0≠+0.0; a Bytes
        // leaf is byte-canonical wherever the walk reaches it, exactly as `ty_heap_walkable` admits).
        // `Char` is eq-walkable too: a scalar compared by its i32 code-point (its `shape_of` descriptor is
        // `ShapeNode::Int`), so a compound carrying a `Char` leaf — notably the `Ast` sum's `Ast.Char`
        // variant — has a walkable structural `=`. (Ordering `<` over a Char-in-compound stays a separate
        // carve-out like Bytes/float; only equality is admitted here.)
        Ty::Int(_)
        | Ty::Char
        | Ty::Bool
        | Ty::Unit
        | Ty::String
        | Ty::Symbol
        | Ty::BigInt
        | Ty::Rational => true,
        Ty::Float(_) | Ty::Bytes => true,
        Ty::Tuple(elems) => {
            let elems = elems.to_vec();
            elems.iter().all(|e| eq_shaped_walkable(db, e, seen))
        }
        Ty::List(elem) => {
            let elem = (**elem).clone();
            eq_shaped_walkable(db, &elem, seen)
        }
        Ty::Record(fields) => {
            let vals: Vec<Ty> = fields.values().cloned().collect();
            vals.iter().all(|v| eq_shaped_walkable(db, v, seen))
        }
        Ty::Nominal { inner, .. } => {
            let inner = (**inner).clone();
            eq_shaped_walkable(db, &inner, seen)
        }
        Ty::Qty { inner, .. } => {
            let inner = (**inner).clone();
            eq_shaped_walkable(db, &inner, seen)
        }
        // A SUM — walkable iff every variant's payload type is. Recursive sum broken by `seen` (a
        // self-referential payload is walkable iff its non-recursive leaves are). Mirrors
        // `orderable_leaf_or_compound`'s Sum arm.
        Ty::Sum { decl, .. } => {
            if seen.contains(decl) {
                return true; // recursive back-edge — already being checked
            }
            seen.push(*decl);
            let variant_count = db.type_decl_by_occ(*decl).map(|t| t.variants.len());
            let mut ok = variant_count.is_some();
            if let Some(vc) = variant_count {
                for disc in 0..vc {
                    let ctor = db
                        .type_decl_by_occ(*decl)
                        .and_then(|t| t.variants.get(disc))
                        .and_then(|v| v.ctor);
                    // A nullary variant (no ctor / no payload) is a bare discriminant — walkable. A
                    // payload-carrying variant's payload type must itself be walkable.
                    if let Some(ctor) = ctor
                        && let Some(payload_ty) =
                            crate::infer::payload_ty_at_instantiation(db, ctor, ty)
                        && !eq_shaped_walkable(db, &payload_ty, seen)
                    {
                        ok = false;
                        break;
                    }
                }
            }
            seen.pop();
            ok
        }
        // A Map/Set/Char/Fn/var — not walked by this slice.
        _ => false,
    }
}

/// Whether the node at `id` has a SCALAR solved type — an integer or a boolean, which occupies a
/// machine slot the backend can compare or compute on directly (as opposed to a compound/heap value).
fn is_scalar(db: &mut Db, id: StructId) -> bool {
    matches!(
        crate::infer::type_of(db, id),
        // `Ty::Char` is a scalar too (Char-rep 2/N): a char occupies an i32 code-point slot
        // (`valtype_of(Ty::Char) = I32`, Char-rep 1/N), so runtime Char equality/ordering/match route to
        // the SAME scalar i32 path an Int/Bool/enum-disc takes — `i32.eq` for `=`, `Core::Compare` (i32) for
        // `<`/`>`/`<=`/`>=`, a scalar-value `match`. Code points are 0..=0x10FFFF (always non-negative), so
        // the signed-i32 compare matches Unicode-scalar (code-point) order. Without this a runtime char fell
        // to the compound path and declined ("comparison of a compound value needs a heap walk" / "matching a
        // compound value needs a heap walk").
        crate::ty::Ty::Int(_) | crate::ty::Ty::Bool | crate::ty::Ty::Char
    )
}

/// Whether the node at `id` has a solved type of `String`, `Symbol`, or `Bytes` — a BYTE LEAF whose blessed
/// total order is content-lexicographic over its raw bytes (String/Symbol: UTF-8 == Unicode-scalar order;
/// Bytes: unsigned byte values, §order). All three are routed to `Core::StrCmp` for a runtime ordering
/// compare — its byte-lex walk (`bytes-len`/`bytes-get` on wasm; native slice `cmp` on rust) is byte-leaf-
/// generic, so a Bytes operand compares identically with NO emit change.
fn operand_is_string_or_symbol(db: &mut Db, id: StructId) -> bool {
    matches!(
        crate::infer::type_of(db, id),
        crate::ty::Ty::String | crate::ty::Ty::Symbol | crate::ty::Ty::Bytes
    )
}

/// Whether the value at `id` has an ENUM-DISCRIMINANT type — a C-style enum the backend represents as a
/// bare discriminant `i32` (no heap box; `Db::is_enum_disc`). Used to route `=` on such a value to the
/// scalar `i32.eq` compare rather than a `value-eq` heap walk. Peels a nominal wrapper (a nominal-over-
/// enum shares the representation).
fn node_ty_is_enum_disc(db: &mut Db, id: StructId) -> bool {
    match crate::infer::type_of(db, id).strip_nominal() {
        crate::ty::Ty::Sum { decl, .. } => db.is_enum_disc(*decl),
        _ => false,
    }
}

/// The user-facing `Operand.key` spelling of an operation head that is a `(. Operand key)` member access
/// (`(. List at)` → `List.at`), for a wrong-arity diagnostic. Reads the two segment names off the raw
/// `.` form — the surface the author wrote — rather than the internal intrinsic name (`list-at`). `None`
/// when the head is not a two-segment member access (e.g. a bare alias), so the caller falls back to a
/// generic phrasing.
fn op_member_name(db: &Db, head: StructId) -> Option<String> {
    let tail = db.ast.as_form(head, ".")?;
    let [operand, key] = tail else { return None };
    let operand = db.ast.as_name(*operand)?;
    let key = db.ast.as_name(*key)?;
    Some(format!("{operand}.{key}"))
}

/// Reduce an `Ordering` to the boolean the comparison `op` asks of it — the one place the relational
/// prims map to their meaning, shared by every scalar the fold compares (integers and booleans agree
/// on the ordering; only the comparison of the ordering differs). Equality is `Ordering::Equal`.
fn compare_ord(op: Prim, ord: std::cmp::Ordering) -> bool {
    use std::cmp::Ordering::*;
    match op {
        Prim::Lt => ord == Less,
        Prim::Gt => ord == Greater,
        Prim::Le => ord != Greater,
        Prim::Ge => ord != Less,
        Prim::Eq => ord == Equal,
        _ => false, // not a comparison — unreachable (only called for `is_comparison` prims).
    }
}

/// The source spelling of an intrinsic, for diagnostics.
fn intrinsic_name(op: Prim) -> &'static str {
    match op {
        Prim::Add => "+",
        Prim::Sub => "-",
        Prim::Mul => "*",
        Prim::Div => "/",
        Prim::Rem => "%",
        Prim::Shl => "<<",
        Prim::Shr => ">>",
        Prim::BitAnd => "&",
        Prim::BitOr => "|",
        Prim::BitXor => "^",
        Prim::Lt => "<",
        Prim::Gt => ">",
        Prim::Le => "<=",
        Prim::Ge => ">=",
        Prim::Eq => "=",
        Prim::Compare => "compare",
        Prim::Wrap => "wrap",
        Prim::CheckedOf => "of",
        Prim::IntCtor => "Int",
        Prim::UIntCtor => "UInt",
        Prim::FnCtor => "->",
        Prim::TupleCtor => "Tuple",
        Prim::RecordCtor => "Record",
        Prim::BoolTy => "Bool",
        Prim::UnitTy => "Unit",
        Prim::StringTy => "String",
        Prim::BigIntTy => "BigInt",
        Prim::BigIntOf => "bigint-of",
        Prim::RationalTy => "Rational",
        Prim::RationalOf => "rational-of",
        Prim::RationalOfInt => "rational-of-int",
        Prim::RationalValue => "rational-value",
        Prim::RationalNum => "rational-num",
        Prim::RationalDen => "rational-den",
        Prim::RationalTruncate => "rational-truncate",
        Prim::RationalFloor => "rational-floor",
        Prim::RationalCeil => "rational-ceil",
        Prim::RationalRound => "rational-round",
        Prim::CharTy => "Char",
        Prim::CharToInt => "char-to-int",
        Prim::CharFromInt => "char-from-int",
        Prim::ValueEncode => "value-encode",
        Prim::ValueDecode => "value-decode",
        Prim::SymbolTy => "Symbol",
        Prim::SymbolOf => "symbol-of",
        Prim::SymbolToString => "symbol-to-string",
        Prim::SumNew => "sum-new",
        Prim::SumCtor => "sum-ctor",
        Prim::TupleNew => "tuple-new",
        Prim::RecordNew => "record-new",
        Prim::RecordProject => "record-project",
        Prim::RecordWithout => "record-without",
        Prim::RecordMerge => "record-merge",
        Prim::RecordExtend => "record-extend",
        Prim::RecordWith => "record-with",
        Prim::RecordPop => "record-pop",
        Prim::TupleCat => "tuple-cat",
        Prim::TupleSplitAt => "tuple-split-at",
        Prim::TuplePop => "tuple-pop",
        Prim::ListNew => "list-new",
        Prim::ListLen => "list-len",
        Prim::ListPush => "list-push",
        Prim::ListPrepend => "list-prepend",
        Prim::ListConcat => "list-concat",
        Prim::ListUpdate => "list-update",
        Prim::ListAt => "list-at",
        Prim::AstSpliceLift => "ast-splice-lift",
        Prim::AstLift => "ast-lift",
        Prim::AstEncode => "ast-encode",
        Prim::AstDecode => "ast-decode",
        Prim::Blake3Of => "blake3-of",
        Prim::ReflectModule => "reflect-module",
        Prim::Print => "print",
        Prim::Read => "read",
        Prim::ListCtor => "List",
        Prim::BytesOf => "bytes-of",
        Prim::BytesLen => "bytes-len",
        Prim::BytesTy => "bytes-ty",
        Prim::StrScalarLen => "str-scalar-len",
        Prim::StrByteLen => "str-byte-len",
        Prim::BytesAt => "bytes-at",
        Prim::BytesConcat => "bytes-concat",
        Prim::BytesSlice => "bytes-slice",
        Prim::BytesCompact => "bytes-compact",
        Prim::StrAt => "str-at",
        Prim::StrScalarAt => "str-scalar-at",
        Prim::StrConcat => "str-concat",
        Prim::StrSlice => "str-slice",
        Prim::StrToBytes => "str-to-bytes",
        Prim::StrFromBytes => "str-from-bytes",
        Prim::SumExpect => "sum-expect",
        Prim::Trap => "trap",
        Prim::CheckedAdd => "checked-add",
        Prim::CheckedSub => "checked-sub",
        Prim::CheckedMul => "checked-mul",
        Prim::WrappingAdd => "wrapping-add",
        Prim::WrappingSub => "wrapping-sub",
        Prim::WrappingMul => "wrapping-mul",
        // The F* prims are the float MODE of the ONE arithmetic operator — a float `+`, not a distinct
        // `+.` — so a user-facing message (arity fault) names the undotted operator the author wrote.
        Prim::FAdd => "+",
        Prim::FSub => "-",
        Prim::FMul => "*",
        Prim::FDiv => "/",
        Prim::FEq => "=",
        Prim::FLt => "<",
        Prim::FLe => "<=",
        Prim::FGt => ">",
        Prim::FGe => ">=",
        Prim::FloatCtor => "Float",
        Prim::FloatOfInt => "of-int",
        Prim::FloatOf => "of",
        Prim::FloatNan => "nan",
        Prim::FloatInf => "Infinity",
        Prim::MapCtor => "Map",
        Prim::MapNew => "map-new",
        Prim::MapEmpty => "map-empty",
        Prim::MapInsert => "map-insert",
        Prim::MapLookup => "map-lookup",
        Prim::MapRemove => "map-remove",
        Prim::MapSize => "map-size",
        Prim::MapSwap => "map-swap",
        Prim::MapTake => "map-take",
        Prim::UnitOne => "unit-one",
        Prim::UnitBase => "unit-base",
        Prim::UnitMul => "unit-mul",
        Prim::UnitDiv => "unit-div",
        Prim::UnitPow => "unit-pow",
        Prim::UnitPrefix => "unit-prefix",
        Prim::UnitOf => "unit-of",
        Prim::UnitDefine => "unit-define",
        Prim::UnitIn => "unit-in",
        Prim::QtyOf => "qty-of",
        Prim::QtyValue => "qty-value",
        Prim::QtyPow => "qty-pow",
        Prim::QtyUnit => "qty-unit",
        Prim::QtyCtor => "Qty",
        Prim::TypeOf => "type-of",
        Prim::TypeEq => "type-eq",
        Prim::SetCtor => "Set",
        Prim::SetOf => "set-of",
        Prim::MapToList => "map-to-list",
        Prim::SetToList => "set-to-list",
        Prim::SetContains => "set-contains",
        Prim::SetLen => "set-len",
        Prim::SetInsert => "set-insert",
        Prim::SetRemove => "set-remove",
        Prim::SetUnion => "set-union",
        Prim::SetIntersection => "set-intersection",
        Prim::SetDifference => "set-difference",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::IntValue;
    use crate::testkit::{if_program, scalar_program};

    // ── R2 value-form TEMPLATE (the compile-time-computed byte template + runtime holes) ──────────
    //
    // A runtime compound's `encode()` copies the template into memory then fills each hole by walking
    // the heap handle. These tests SIMULATE that fill in Rust — build the template for a type, write
    // each hole from a Rust value model, decode with the shared codec, and assert the rendered text —
    // proving the template layout + hole offsets are right BEFORE any wasm emission depends on them.

    /// A tiny value model mirroring what the runtime holds: a nested tuple/record of ints and bools.
    #[derive(Clone)]
    enum V {
        Int(i64),
        Bool(bool),
        Tuple(Vec<V>),
        Record(Vec<V>), // fields in canonical (sorted) order — positional, like the heap array
    }

    /// Follow a hole's `arr-get` path from the root value to the leaf it addresses.
    fn walk<'a>(root: &'a V, path: &[u32]) -> &'a V {
        let mut v = root;
        for &i in path {
            v = match v {
                V::Tuple(es) | V::Record(es) => &es[i as usize],
                _ => panic!("path descends into a scalar"),
            };
        }
        v
    }

    /// Simulate `encode()`: fill the template's holes from `root`, returning the finished bytes. Int →
    /// 8 big-endian magnitude bytes at the hole (+ flip the kind byte to NEG for a negative); Bool →
    /// the kind byte (8/9).
    fn fill(tpl: &ValueFormTemplate, root: &V) -> Vec<u8> {
        let mut bytes = tpl.bytes.clone();
        for hole in &tpl.leaves {
            match (hole.kind, walk(root, &hole.path)) {
                (LeafFill::Int, V::Int(n)) => {
                    let mag = (n.unsigned_abs()).to_be_bytes(); // 8 bytes, big-endian
                    bytes[hole.offset..hole.offset + 8].copy_from_slice(&mag);
                    if *n < 0 {
                        // kind byte sits 2 bytes before the magnitude (kind + len=8 → len is one byte).
                        bytes[hole.offset - 2] = 3; // KIND_INT_NEG_DEC
                    }
                }
                (LeafFill::Bool, V::Bool(b)) => {
                    bytes[hole.offset] = if *b { 9 } else { 8 };
                }
                _ => panic!("hole kind / value mismatch"),
            }
        }
        bytes
    }

    /// Build a template for `ty`, fill it from `root`, decode + print — the value-form text the host
    /// would render.
    fn render(ty: &crate::ty::Ty, root: &V) -> String {
        let tpl = runtime_value_form_template(ty, &crate::ty::NameCtx::new(&[])).expect("template");
        let bytes = fill(&tpl, root);
        let arenas = cadenza_syntax::codec::decode(&bytes).expect("decode filled template");
        cadenza_syntax::sexpr::print(&arenas).trim().to_string()
    }

    fn t_int() -> crate::ty::Ty {
        crate::ty::Ty::int64()
    }

    #[test]
    fn template_fills_a_flat_runtime_tuple() {
        let ty = crate::ty::Ty::Tuple(vec![t_int(), t_int()].into());
        assert_eq!(
            render(&ty, &V::Tuple(vec![V::Int(3), V::Int(1)])),
            "(: (tuple 3 1) (Tuple Int64 Int64))"
        );
        // Different runtime values reuse the SAME template — only the holes change.
        assert_eq!(
            render(&ty, &V::Tuple(vec![V::Int(4), V::Int(8)])),
            "(: (tuple 4 8) (Tuple Int64 Int64))"
        );
    }

    #[test]
    fn template_fills_a_mixed_and_negative_tuple() {
        let ty = crate::ty::Ty::Tuple(vec![t_int(), crate::ty::Ty::Bool].into());
        assert_eq!(
            render(&ty, &V::Tuple(vec![V::Int(0), V::Bool(true)])),
            "(: (tuple 0 true) (Tuple Int64 Bool))"
        );
        let ty2 = crate::ty::Ty::Tuple(vec![t_int(), t_int()].into());
        assert_eq!(
            render(&ty2, &V::Tuple(vec![V::Int(-5), V::Int(7)])),
            "(: (tuple -5 7) (Tuple Int64 Int64))"
        );
    }

    #[test]
    fn template_fills_a_three_element_and_nested_tuple() {
        let ty3 = crate::ty::Ty::Tuple(vec![t_int(), t_int(), t_int()].into());
        assert_eq!(
            render(&ty3, &V::Tuple(vec![V::Int(10), V::Int(11), V::Int(12)])),
            "(: (tuple 10 11 12) (Tuple Int64 Int64 Int64))"
        );
        let nested = crate::ty::Ty::Tuple(
            vec![t_int(), crate::ty::Ty::Tuple(vec![t_int(), t_int()].into())].into(),
        );
        assert_eq!(
            render(
                &nested,
                &V::Tuple(vec![V::Int(2), V::Tuple(vec![V::Int(2), V::Int(2)])])
            ),
            "(: (tuple 2 (tuple 2 2)) (Tuple Int64 (Tuple Int64 Int64)))"
        );
    }

    #[test]
    fn template_fills_a_runtime_record() {
        use crate::resolved::Symbol;
        let mut fields = std::collections::BTreeMap::new();
        fields.insert(Symbol::plain("a"), t_int());
        fields.insert(Symbol::plain("b"), t_int());
        let ty = crate::ty::Ty::Record(fields.into());
        // Fields in canonical (sorted) order a, b → positional [a, b].
        assert_eq!(
            render(&ty, &V::Record(vec![V::Int(3), V::Int(1)])),
            "(: (record (= a 3) (= b 1)) (Record (: a Int64) (: b Int64)))"
        );
    }

    #[test]
    fn lowers_a_literal_to_a_const() {
        let (ast, body) = scalar_program();
        let mut db = Db::load(ast);
        assert_eq!(
            core_of(&mut db, body),
            Core::ConstInt(IntValue::from_i64(42))
        );
    }

    #[test]
    fn an_if_with_a_constant_condition_folds_to_the_taken_branch() {
        // `if_program` is `(if false 1 2)` — a CONSTANT condition, so it folds to the else-branch (2),
        // NOT a residual `Core::If`. (A constant-condition `if` is dead-branch-eliminated in `lower`.)
        let (ast, if_node) = if_program();
        let mut db = Db::load(ast);
        assert_eq!(
            core_of(&mut db, if_node),
            Core::ConstInt(IntValue::from_i64(2)),
            "if false 1 2 folds to 2"
        );
    }

    #[test]
    fn a_const_if_folds_past_an_unreachable_trap_but_not_an_illformed_branch() {
        // A `ConstTrap` (CDZ0304) in the UNTAKEN branch is reachability-gated — the const-if folds past
        // it to the taken branch (the same rule `collect_reached_poisons` applies: a trap shielded by an
        // untaken branch is not a build failure). `(if (< 1 2) 7 (% 5 0))` → 7.
        let ast =
            crate::testkit::parse("(module m (def (main) (if (< 1 2) 7 (% 5 0))) (export main))");
        let mut db = Db::load(ast);
        let body = db.defs[db.def_by_name("main").unwrap()].body.unwrap();
        assert_eq!(
            core_of(&mut db, body),
            Core::ConstInt(IntValue::from_i64(7)),
            "a const-if folds past an unreachable ConstTrap untaken branch"
        );
        // But a NON-TRAP poison in the untaken branch (an unbound name) is an ill-formedness the program
        // must be rejected for — the const-if is KEPT (not folded) so the fault surfaces.
        let ast2 =
            crate::testkit::parse("(module m (def (main) (if (< 1 2) 7 nope)) (export main))");
        let mut db2 = Db::load(ast2);
        let body2 = db2.defs[db2.def_by_name("main").unwrap()].body.unwrap();
        assert!(
            matches!(core_of(&mut db2, body2), Core::If { .. }),
            "a const-if with an ill-formed (unbound-name) untaken branch is NOT folded away"
        );
    }

    #[test]
    fn a_let_binding_a_diverging_trap_init_folds_to_the_trap_not_a_compound_compare() {
        // A `let` whose INIT is an unconditional `(trap …)` diverges before the binding is bound, so the
        // body is unreachable — the whole `let` IS the trap. Without the diverging-init short-circuit the
        // body lowered against a binding typed `Never`/`Any` (the trap's bottom type), and a comparison of
        // that binding — `(if (> it 0) it (trap))` — reached `is_scalar(it) == false` and mis-declined
        // "comparison of a compound value needs a heap walk" (a construct absent from the program). This is
        // the root of the reversed `@ensures`/`@requires`-over-an-effectful-body decline (the desugar emits
        // `(let ((it (trap "@requires…"))) (if (> it 0) it (trap "@ensures…")))` in the precondition-fail
        // branch). The let must fold to the trap. Guarded behind an `if` so the trap is in an untaken branch
        // (a bare diverging main would just be the trap) — the point is the LET's body is not lowered.
        let ast = crate::testkit::parse(
            "(module m (def (main (: b Bool)) (if b 1 (let ((it (trap \"x\"))) (if (> it 0) it (trap \"y\"))))) (export main))",
        );
        let mut db = Db::load(ast);
        let body = db.defs[db.def_by_name("main").unwrap()].body.unwrap();
        // The else-branch `(let ((it (trap …))) …)` must lower to `Core::Trap`, NOT a compound-compare
        // Poison. Assert the whole body lowers WITHOUT a poison (the `if` keeps a live then-branch `1` and a
        // trapping else — both machine-representable), which it could not if the let declined.
        assert!(
            !matches!(core_of(&mut db, body), Core::Poison(_)),
            "a let binding a diverging trap init must fold to the trap, not decline compound-compare"
        );
    }

    #[test]
    fn an_effect_performing_closure_through_two_helper_hops_folds() {
        // cx6: an effect-performing closure passed as a fn ARGUMENT through TWO nested one-shot helper hops
        // (`ap1` calls `ap2`, `ap2` applies the closure). The escaped-closure-leak recovery β-inlines both
        // hops so the perform homes inside the handle's reach; the fold then succeeds instead of declining
        // `HANDLER_NOT_REDUCIBLE`. This exercises the MULTI-hop path: hop 1's β-reduction carries the param
        // annotation onto the substituted closure (`(: closure (-> …))`), so hop 2 sees it as an annotated
        // lambda — `arg_is_lambda_valued` must see through the annotation for the second inline to fire.
        let ast = crate::testkit::parse(
            "(module m (effect E (op tick (-> Int64))) (def (ap2 (: g (-> Int64 Int64))) (g 5)) (def (ap1 (: g (-> Int64 Int64))) (ap2 g)) (def (main (: n Int64)) (handle E (% n 3) ((tick () s (resume (* s 10) (+ s 1)))) (ap1 (fn (x) (+ x (E.tick)))))) (export main))",
        );
        let mut db = Db::load(ast);
        let body = db.defs[db.def_by_name("main").unwrap()].body.unwrap();
        assert!(
            !matches!(core_of(&mut db, body), Core::Poison(_)),
            "cx6 two-hop escaped-closure handle must fold (recovery inlines both hops), not decline"
        );
    }

    #[test]
    fn a_let_bound_escaped_closure_answer_folds_not_misrejects() {
        // breaker iso-b: let-binding the escaped-closure helper-call answer inside the handle body and
        // returning the binder — `(handle E s (arms) (let ((a (ap (fn (x) (+ x (E.tick)))))) a))` — is a
        // WELL-FORMED program that must FOLD (the `let` just names cx5d's answer). The escaped-closure
        // recovery rebuilds the `let` and re-parents the load-time body ref under the fresh `let`; that ref
        // kept its stale LOAD-TIME scope-skip entry (→ its original init), so it resolved to the OLD init and
        // a later hygiene rename missed it → it re-resolved UNBOUND (a false CDZ0101). The recovery now forces
        // the rebuilt subtree's load-time nodes onto the exhaustive walk, so the ref resolves against the
        // CURRENT (rebuilt) `let` and the handle folds — NOT a poison of any kind (was Unbound).
        let ast = crate::testkit::parse(
            "(module m (effect E (op tick (-> Int64))) (def (ap (: g (-> Int64 Int64))) (g 5)) (def (main (: n Int64)) (handle E (% n 3) ((tick () s (resume (* s 10) (+ s 1)))) (let ((a (ap (fn (x) (+ x (E.tick)))))) a))) (export main))",
        );
        let mut db = Db::load(ast);
        let body = db.defs[db.def_by_name("main").unwrap()].body.unwrap();
        assert!(
            !matches!(core_of(&mut db, body), Core::Poison(_)),
            "a let-bound escaped-closure answer must FOLD (recovery forces structural resolution), not decline or mis-reject"
        );
    }

    #[test]
    fn an_escaped_closure_capturing_an_outer_let_binder_folds() {
        // breaker sk4c: a closure through the one-shot helper that CAPTURES an outer `let` binder —
        // `(handle E s (arms) (let ((a 7)) (ap (fn (y) (+ (+ y a) (E.tick))))))`. The recovery inlines `ap`
        // so the closure applies inline; `pin_free_vars` correctly SHARES the capture `a` into the reduced
        // body, but the shared node kept a STALE parent (the dead original closure), so the recovery's
        // `forget_subtree` + `force_structural_resolution_subtree` re-resolved `a` via that dead chain → a
        // false CDZ0101. `forget_subtree_keep_pinned` now preserves the pinned capture's correct memo, so it
        // folds — NOT a poison. Guards the pinned-capture-vs-force-structural interaction.
        let ast = crate::testkit::parse(
            "(module m (effect E (op tick (-> Int64))) (def (ap (: g (-> Int64 Int64))) (g 5)) (def (main (: n Int64)) (handle E (% n 3) ((tick () s (resume (* s 10) (+ s 1)))) (let ((a 7)) (ap (fn (y) (+ (+ y a) (E.tick))))))) (export main))",
        );
        let mut db = Db::load(ast);
        let body = db.defs[db.def_by_name("main").unwrap()].body.unwrap();
        assert!(
            !matches!(core_of(&mut db, body), Core::Poison(_)),
            "an escaped closure capturing an outer let binder must FOLD (pinned-capture memo preserved), not mis-reject CDZ0101"
        );
    }

    #[test]
    fn a_mixed_arm_abort_reading_growing_list_state_declines_honestly_not_cdz0101() {
        // breaker abx3: a handler with mixed resume(step)+abortive(bail) arms over a GROWING list state,
        // whose ABORT arm READS the state — `(handle E … ((step () s (resume (List.len s) (List.prepend s 0)))
        // (bail (k) s (+ k (List.len s)))) (+ (E.step) (E.bail 100)))`. The step dispatch threads the growing
        // state as a FINDING-24 `#st{node}_{slot}` name (bound only in the resume continuation's drain scope);
        // the strict-op (`+`) abort collapse does NOT drain those binds (do-form only), so the abort arm's
        // `(List.len #st…)` reference LEAKED unbound → a spurious CDZ0101 on a well-formed program. It must
        // decline HONESTLY (HANDLER_NOT_REDUCIBLE) — the full fold is a separate strict-op-abort increment.
        let src = "(module m (effect E (op step (-> Int64)) (op bail (-> Int64 Int64))) (def (main (: n Int64)) (handle E (if (> n 0) (list n) (list 9 9)) ((step () s (resume (List.len s) (List.prepend s 0))) (bail (k) s (+ k (List.len s)))) (+ (E.step) (E.bail 100)))) (export main))";
        match crate::compile::compile_component(&crate::codec::encode(&crate::testkit::parse(src)))
        {
            Err(d) => assert_ne!(
                d.code.as_deref(),
                Some("CDZ0101"),
                "abx3 must decline honestly (HANDLER_NOT_REDUCIBLE), not mis-reject CDZ0101 unbound on a well-formed program"
            ),
            Ok(_) => { /* a future strict-op-abort fold increment may make it compile — also fine */
            }
        }
    }

    #[test]
    fn an_if_with_identical_branches_folds_to_the_branch() {
        // `(if p x x)` — both branches are the same value, so the `if` collapses to `x` (the condition
        // `p` is a param, trap-free, so evaluating it has no effect to preserve). Result: `Core::Param`
        // (the `x`), NOT a `Core::If`.
        let ast = crate::testkit::parse(
            "(module m (def (f (: p Bool) (: x Int64)) (if p x x)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let body = db.defs[db.def_by_name("f").unwrap()].body.unwrap();
        assert!(
            matches!(core_of(&mut db, body), Core::Param { .. }),
            "an if with identical branches over a trap-free condition folds to the branch"
        );
    }

    #[test]
    fn if_true_false_folds_to_the_condition() {
        // `(if c true false)` is a boolean coercion no-op — it computes `c` itself. `(< a b)` is a
        // comparison, so the body folds to `Core::Compare`, NOT a `Core::If` wrapping two ConstBools.
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64) (: b Int64)) (if (< a b) true false)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let body = db.defs[db.def_by_name("f").unwrap()].body.unwrap();
        assert!(
            matches!(core_of(&mut db, body), Core::Compare { .. }),
            "if c true false folds to the condition c"
        );
        // The dual `(if c false true)` is a NEGATION `!c` — it folds to `Core::Not { operand: c }` (the
        // backend emits `<c> ; i32.eqz`), NOT the bare condition (that would leave the result uninverted).
        let ast2 = crate::testkit::parse(
            "(module m (def (f (: a Int64) (: b Int64)) (if (< a b) false true)) (def (main) 0) (export main))",
        );
        let mut db2 = Db::load(ast2);
        let body2 = db2.defs[db2.def_by_name("f").unwrap()].body.unwrap();
        assert!(
            matches!(core_of(&mut db2, body2), Core::Not { .. }),
            "if c false true folds to the negation !c"
        );
    }

    #[test]
    fn an_if_with_identical_branches_keeps_a_possibly_trapping_condition() {
        // `(if (g x) x x)` where `g` is a RECURSIVE call (possibly-trapping) — the branches are equal,
        // but the condition is NOT trap-free, so the `if` is KEPT to preserve the condition's evaluation
        // (and any trap). Result stays a `Core::If`.
        let ast = crate::testkit::parse(
            "(module m (def (g (: n Int64)) (if (= n 0) true (g (- n 1)))) \
               (def (f (: x Int64)) (if (g x) x x)) (export f))",
        );
        let mut db = Db::load(ast);
        let body = db.defs[db.def_by_name("f").unwrap()].body.unwrap();
        assert!(
            matches!(core_of(&mut db, body), Core::If { .. }),
            "identical branches do NOT fold away a possibly-trapping condition"
        );
    }

    #[test]
    fn lowers_a_runtime_if_referencing_its_child_ids() {
        // A RUNTIME condition (a bool parameter `p`) is NOT foldable, so it stays a `Core::If` carrying
        // its child occurrences: `(def (f (: p Bool)) (if p 1 2))`.
        let ast = crate::testkit::parse(
            "(module m (def (f (: p Bool)) (if p 1 2)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let d = db.def_by_name("f").expect("def f");
        let body = db.defs[d].body.expect("body");
        match core_of(&mut db, body) {
            Core::If { then_, else_, .. } => {
                assert_eq!(
                    core_of(&mut db, then_),
                    Core::ConstInt(IntValue::from_i64(1))
                );
                assert_eq!(
                    core_of(&mut db, else_),
                    Core::ConstInt(IntValue::from_i64(2))
                );
            }
            other => panic!("expected If, got {other:?}"),
        }
    }

    // ── A-normalization: the keep-or-propagate decision at the core column ────────────────────────
    //
    // A `let` whose value is a RUNTIME computation used more than once is kept as a `Core::Let`
    // (named once); a single-use or constant binding is propagated (no residual `Let`). These inspect
    // the core form directly — the module's own concern — separate from the wasm behavior tests.

    /// Locate def `name`'s body occurrence (the root of the expression `core_of` is asked about).
    fn body_of(db: &mut Db, name: &str) -> StructId {
        let d = db.def_by_name(name).expect("def present");
        db.defs[d].body.expect("body")
    }

    #[test]
    fn a_multi_use_runtime_let_lowers_to_a_core_let() {
        // `(let ((s (+ a b))) (+ s s))` in a function body — `s` is a runtime add used twice, so the
        // body's core is a `Core::Let` naming `s`, with the binding keyed by `s`'s initializer.
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64) (: b Int64)) (let ((s (+ a b))) (+ s s))) (export f))",
        );
        let mut db = Db::load(ast);
        let body = body_of(&mut db, "f");
        match core_of(&mut db, body) {
            Core::Let { bindings, .. } => {
                assert_eq!(bindings.len(), 1, "exactly one binding kept");
                // The kept binding's value lowers to a runtime arithmetic op (the `(+ a b)`).
                let (_, value) = bindings[0];
                assert!(matches!(core_of(&mut db, value), Core::Arith { .. }));
            }
            other => panic!("expected Core::Let, got {other:?}"),
        }
    }

    #[test]
    fn a_single_use_runtime_let_leaves_no_core_let() {
        // `(let ((s (+ a b))) (* s 2))` — `s` used ONCE, so it is copy-propagated: the body's core is
        // the `(* (+ a b) 2)` multiplication directly, with NO enclosing `Core::Let`.
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64) (: b Int64)) (let ((s (+ a b))) (* s 2))) (export f))",
        );
        let mut db = Db::load(ast);
        let body = body_of(&mut db, "f");
        assert!(
            matches!(core_of(&mut db, body), Core::Arith { op: Prim::Mul, .. }),
            "a single-use binding must propagate, leaving no Core::Let"
        );
    }

    #[test]
    fn a_multi_use_constant_let_folds_and_is_not_named() {
        // `(let ((k (+ 1 2))) (+ k k))` — `k` used twice but its value FOLDS to the constant 3, so
        // there is no runtime computation to share: the whole body folds to `ConstInt(6)`, no `Let`.
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64)) (let ((k (+ 1 2))) (+ k k))) (export f))",
        );
        let mut db = Db::load(ast);
        let body = body_of(&mut db, "f");
        assert_eq!(
            core_of(&mut db, body),
            Core::ConstInt(IntValue::from_i64(6)),
            "a constant binding folds; nothing is named"
        );
    }

    #[test]
    fn list_len_over_a_let_bound_constant_list_folds_through_the_binder() {
        // `(let ((xs (list 1 2 3))) (+ (List.len xs) (List.len xs)))` — `xs` is a KEPT multi-use binding
        // (used twice), so `List.len xs`'s operand core is a `LocalRef`, not a direct `ListNew`. The fold
        // must FOLLOW the binder to the bound list literal and fold the len to its arity (3), so the `let`
        // BODY folds to `ConstInt(6)` (3 + 3). The binding itself is KEPT (the multi-use keep decision was
        // taken before folding, so the list is still built — a dead-binding DCE is a separate pass), but
        // the len reads no longer touch it. Pins the both-backend Core fold gap v-wasm-opt routed.
        let mut db = Db::load(crate::testkit::parse(
            "(module m (def (g) (let ((xs (list 1 2 3))) (+ (List.len xs) (List.len xs)))) (export g))",
        ));
        let body = body_of(&mut db, "g");
        match core_of(&mut db, body) {
            Core::Let { body: let_body, .. } => {
                assert_eq!(
                    core_of(&mut db, let_body),
                    Core::ConstInt(IntValue::from_i64(6)),
                    "the let body folds to 6 (both List.len reads fold through the binder to arity 3)"
                );
            }
            // If a future DCE erases the now-dead binding, a bare ConstInt(6) is also correct.
            Core::ConstInt(v) => assert_eq!(v, IntValue::from_i64(6)),
            other => panic!("expected a Let whose body folds to 6 (or a bare 6), got {other:?}"),
        }
    }

    #[test]
    fn list_len_over_a_let_bound_list_folds_the_len_but_keeps_the_binding_when_used_elsewhere() {
        // When the bound list is ALSO used elsewhere, the len read still folds to the arity (a constant),
        // but the binding is NOT erased — the list is still built for the other use. Here `xs` feeds both a
        // `List.len` (folds to 3) and a `List.concat` (a runtime op that keeps the list live). The `let`
        // body is an `Add` whose lhs is the folded `ConstInt(3)` — pinning that following the binder folds
        // ONLY the direct len, not the runtime concat's len, and keeps the binding.
        let mut db = Db::load(crate::testkit::parse(
            "(module m (def (g) (let ((xs (list 10 20 30))) (+ (List.len xs) (List.len (List.concat xs xs))))) (export g))",
        ));
        let body = body_of(&mut db, "g");
        let add = match core_of(&mut db, body) {
            Core::Let { body: let_body, .. } => core_of(&mut db, let_body),
            other => other,
        };
        match add {
            Core::Arith {
                op: Prim::Add, lhs, ..
            } => {
                assert_eq!(
                    core_of(&mut db, lhs),
                    Core::ConstInt(IntValue::from_i64(3)),
                    "the direct List.len xs folds to the arity 3"
                );
            }
            other => panic!("expected an Add over a folded len, got {other:?}"),
        }
    }

    #[test]
    fn list_at_over_a_let_bound_constant_list_folds_through_the_binder() {
        // `(let ((xs (list 10 20 30))) (List.at xs 1))` — a CONSTANT list bound to a multi-use `xs`,
        // indexed by a constant. `List.at` over an inline list literal already folds to `Some(elem)`; the
        // let-bound form must fold identically by following the `LocalRef` binder to the bound `ListNew`
        // (via `const_list_elems`). The IN-BOUNDS index 1 folds to `Some 20` — a `Core::SumNew` on the
        // Option's Some disc carrying the bound list's element occurrence. Pins the both-backend Core fold.
        let mut db = Db::load(crate::testkit::parse(
            "(module m (def (g) (let ((xs (list 10 20 30))) (List.at xs 1))) (export g))",
        ));
        let body = body_of(&mut db, "g");
        // The multi-use `xs` (also read below in the OOB test) here is single-use, so the body may be the
        // folded `SumNew` directly or wrapped in a residual `Let`; unwrap a `Let` body if present.
        let core = match core_of(&mut db, body) {
            Core::Let { body: let_body, .. } => core_of(&mut db, let_body),
            other => other,
        };
        match core {
            Core::SumNew { payloads, .. } => {
                assert_eq!(payloads.len(), 1, "Some carries one payload");
                assert_eq!(
                    core_of(&mut db, payloads[0]),
                    Core::ConstInt(IntValue::from_i64(20)),
                    "List.at xs 1 folds to Some(20) through the binder"
                );
            }
            other => panic!("expected a folded Some(20) SumNew, got {other:?}"),
        }
    }

    #[test]
    fn list_at_over_a_let_bound_list_folds_out_of_bounds_to_none_through_the_binder() {
        // The OOB companion: `(List.at xs 5)` over a 3-element let-bound constant list folds to `None` (a
        // `Core::SumNew` on the None disc with no payload) — the constant-index/constant-list bounds check
        // resolves at compile time through the binder, exactly like the inline form. `xs` is multi-use here
        // (also fed to `List.len`), so the binding is kept but the `List.at` read folds.
        let mut db = Db::load(crate::testkit::parse(
            "(module m (def (g) (let ((xs (list 10 20 30))) (match (List.at xs 5) ((Some v) v) ((None) (List.len xs))))) (export g))",
        ));
        let body = body_of(&mut db, "g");
        // The `List.at xs 5` folds to None, so the match takes the None arm → `List.len xs` folds to 3.
        // The whole body folds to `ConstInt(3)` (possibly under a residual `Let` for the kept binding).
        let core = match core_of(&mut db, body) {
            Core::Let { body: let_body, .. } => core_of(&mut db, let_body),
            other => other,
        };
        assert_eq!(
            core,
            Core::ConstInt(IntValue::from_i64(3)),
            "OOB List.at folds to None → the None arm (List.len xs = 3) is taken"
        );
    }

    #[test]
    fn a_multi_use_binding_whose_every_use_folds_is_reclaimed_no_dead_build() {
        // Post-fold dead-binding reclaim: `(let ((xs (list 1 2 3))) (+ (List.len xs) (List.len xs)))` —
        // `xs` is read twice, so `should_keep_binding` keeps it (pre-fold count 2); then BOTH `List.len xs`
        // fold to 3 through the binder, so the kept slot has ZERO live `LocalRef` reads. The reclaim drops
        // the now-dead pure `ListNew` binding, so the whole body is the bare `ConstInt(6)` — NO residual
        // `Core::Let` (the list is not built at runtime for nothing). Pins that the fold→DCE phase-ordering
        // gap is closed: a binding whose only uses folded away is not emitted.
        let mut db = Db::load(crate::testkit::parse(
            "(module m (def (g) (let ((xs (list 1 2 3))) (+ (List.len xs) (List.len xs)))) (export g))",
        ));
        let body = body_of(&mut db, "g");
        assert_eq!(
            core_of(&mut db, body),
            Core::ConstInt(IntValue::from_i64(6)),
            "the dead let-binding is reclaimed; the body is a bare constant, no Core::Let"
        );
    }

    #[test]
    fn a_multi_use_binding_with_one_live_and_one_folded_use_stays_kept() {
        // Partial fold: `xs` feeds a `List.len` (folds to 3) AND a `List.concat` (a runtime op that reads
        // the list live). The concat keeps a live `LocalRef` to `xs`, so the reclaim must NOT drop it — the
        // body stays a `Core::Let`. Guards the reclaim against over-eagerly dropping a still-read binding.
        let mut db = Db::load(crate::testkit::parse(
            "(module m (def (g) (let ((xs (list 10 20 30))) (+ (List.len xs) (List.len (List.concat xs xs))))) (export g))",
        ));
        let body = body_of(&mut db, "g");
        assert!(
            matches!(core_of(&mut db, body), Core::Let { .. }),
            "a binding with a surviving live use (List.concat) is kept, not reclaimed"
        );
    }

    #[test]
    fn a_genuinely_multi_use_runtime_binding_is_not_reclaimed() {
        // Regression guard for the reclaim: `(let ((s (+ a b))) (+ s s))` — `s` is a runtime add read
        // twice, its uses do NOT fold (a, b are params), so both `LocalRef` reads are live. The reclaim
        // must leave the `Core::Let` intact (dropping it would re-emit the add twice — a de-optimization).
        let mut db = Db::load(crate::testkit::parse(
            "(module m (def (f (: a Int64) (: b Int64)) (let ((s (+ a b))) (+ s s))) (export f))",
        ));
        let body = body_of(&mut db, "f");
        assert!(
            matches!(core_of(&mut db, body), Core::Let { .. }),
            "a live multi-use runtime binding stays named (reclaim only drops folded-dead bindings)"
        );
    }

    #[test]
    fn discharged_no_overflow_is_a_false_stub_so_the_wrapper_is_behavior_neutral() {
        // Slice-5 seam: `provably_no_overflow` = `arith_provably_in_range` OR `discharged_no_overflow`.
        // Until v-verification's b3 fills the oracle body, `discharged_no_overflow` is a `false` STUB, so
        // the disjunction MUST equal the predicate alone on every node — that is what makes the wrapper
        // (which both backends now call in place of the bare predicate) byte-identical to today. Pin both:
        // (1) the stub returns false for any id; (2) the wrapper's verdict == the predicate's, for BOTH a
        // provably-in-range masked add AND a full-range add. If a future edit flips the stub or the
        // disjunction without being the intended b3 body-fill, this catches the behavior change.
        let mut db = Db::load(crate::testkit::parse(
            "(module m (def (f (: a Int64) (: b Int64)) (+ (& a 15) (& b 15))) (export f))",
        ));
        let in_range = body_of(&mut db, "f");
        let (op, lhs, rhs) = match core_of(&mut db, in_range) {
            Core::Arith { op, lhs, rhs } => (op, lhs, rhs),
            other => panic!("expected an Arith node, got {other:?}"),
        };
        let ty = crate::infer::type_of(&mut db, in_range);
        let crate::ty::Ty::Int(it) = ty else {
            panic!("arith node is not Int-typed")
        };
        let grounded = crate::ty::IntTy::fixed(it.ground_signed(), it.ground_width());
        // (1) stub is false.
        assert!(
            !discharged_no_overflow(&mut db, in_range),
            "discharged_no_overflow is a false stub until b3 fills it"
        );
        // (2) wrapper == predicate for the provably-in-range case (both true here: [0,15]+[0,15]=[0,30]).
        let pred = arith_provably_in_range(&mut db, op, lhs, rhs, grounded);
        let wrap = provably_no_overflow(&mut db, op, lhs, rhs, grounded, in_range);
        assert!(pred, "masked add [0,30] is provably in range");
        assert_eq!(
            wrap, pred,
            "wrapper == predicate while the stub is false (in-range case)"
        );

        // Full-range add: predicate false, wrapper must also be false (stub adds nothing).
        let mut db2 = Db::load(crate::testkit::parse(
            "(module m (def (g (: a Int64) (: b Int64)) (+ a b)) (export g))",
        ));
        let full = body_of(&mut db2, "g");
        let (op2, lhs2, rhs2) = match core_of(&mut db2, full) {
            Core::Arith { op, lhs, rhs } => (op, lhs, rhs),
            other => panic!("expected an Arith node, got {other:?}"),
        };
        let ty2 = crate::infer::type_of(&mut db2, full);
        let crate::ty::Ty::Int(it2) = ty2 else {
            panic!("arith node is not Int-typed")
        };
        let grounded2 = crate::ty::IntTy::fixed(it2.ground_signed(), it2.ground_width());
        let pred2 = arith_provably_in_range(&mut db2, op2, lhs2, rhs2, grounded2);
        let wrap2 = provably_no_overflow(&mut db2, op2, lhs2, rhs2, grounded2, full);
        assert!(!pred2, "full-range add is NOT provably in range");
        assert_eq!(
            wrap2, pred2,
            "wrapper == predicate while the stub is false (full-range case)"
        );
    }

    #[test]
    fn parse_bigint_decimal_covers_the_i64_boundary_and_beyond() {
        // In-i64 tokens still parse (the `read` fast path handles these via `str::parse::<i64>`, but
        // the bignum path must agree so the two never diverge at the seam).
        assert_eq!(parse_bigint_decimal("0"), Some(IntValue::from_i64(0)));
        assert_eq!(parse_bigint_decimal("-0"), Some(IntValue::from_i64(0)));
        assert_eq!(parse_bigint_decimal("42"), Some(IntValue::from_i64(42)));
        assert_eq!(parse_bigint_decimal("-7"), Some(IntValue::from_i64(-7)));
        assert_eq!(
            parse_bigint_decimal("9223372036854775807"),
            Some(IntValue::from_i64(i64::MAX))
        );
        assert_eq!(
            parse_bigint_decimal("-9223372036854775808"),
            Some(IntValue::from_i64(i64::MIN))
        );

        // The EXACT handoff seam: `i64::MAX + 1` and `i64::MIN - 1` are the first magnitudes where
        // `str::parse::<i64>` overflows (Pos/NegOverflow), so `read` hands off to this bignum path HERE —
        // an off-by-one in the accumulation (or a signed-magnitude flaw at the two's-complement extreme,
        // where |i64::MIN| is not i64-representable) would first surface at these two values, not at a
        // comfortably-large 26-digit literal. Build the expected values by `IntValue` arithmetic off the
        // i64 extremes so the assertion pins the value independently of `parse_bigint_decimal`'s own logic.
        assert_eq!(
            parse_bigint_decimal("9223372036854775808"),
            Some(IntValue::from_i64(i64::MAX).add(&IntValue::from_i64(1)))
        );
        assert_eq!(
            parse_bigint_decimal("-9223372036854775809"),
            Some(IntValue::from_i64(i64::MIN).add(&IntValue::from_i64(-1)))
        );

        // Beyond i64: the point of the bignum path — a 26-digit literal `str::parse::<i64>` would reject
        // reads as an exact `IntValue`, positive and negative, matching the `12-metaprogramming.sexp`
        // print→read round-trip pins. Build the expected value by the SAME digit-accumulation so the
        // assertion pins the value, not just non-None.
        let mut expected = IntValue::from_i64(0);
        let ten = IntValue::from_i64(10);
        for _ in 0..26 {
            expected = expected.mul(&ten).add(&IntValue::from_i64(9));
        }
        assert_eq!(
            parse_bigint_decimal("99999999999999999999999999"),
            Some(expected.clone())
        );
        assert_eq!(
            parse_bigint_decimal("-99999999999999999999999999"),
            Some(expected.neg())
        );

        // Non-integer tokens decline so `read` falls through to the float/Name classification — a bare
        // sign, an empty string, a float, and a name-with-digits must NOT be misread as an `Ast.Int`.
        assert_eq!(parse_bigint_decimal(""), None);
        assert_eq!(parse_bigint_decimal("-"), None);
        assert_eq!(parse_bigint_decimal("1.5"), None);
        assert_eq!(parse_bigint_decimal("1e10"), None);
        assert_eq!(parse_bigint_decimal("12a"), None);
        assert_eq!(parse_bigint_decimal("x1"), None);

        // A leading `+` is not part of a numeric literal (v-syntax ruling A) — this fn strips only `-`, so
        // a `+`-prefixed token declines HERE at every magnitude, and `read_node`'s guard classifies it as
        // an `Ast.Name`. Pinning both sides of i64 witnesses the uniformity the ruling requires.
        assert_eq!(parse_bigint_decimal("+5"), None);
        assert_eq!(parse_bigint_decimal("+99999999999999999999999999"), None);
    }

    #[test]
    fn read_classifies_a_leading_plus_token_as_a_name_at_every_magnitude() {
        // The `read_node` guard (v-syntax ruling A): a `+`-prefixed token is a Name, not an Int/Float, so
        // the i64-boundary inconsistency (`+5`→Int via the i64 fast path, `+<huge>`→Name via the bignum
        // path) is gone. Drive the tokenizer directly and assert the SNode variant on both sides of i64.
        let name_of = |tok: &str| match SexprReader::new(tok).read_node() {
            Some(SNode::Name(s)) => Some(s),
            _ => None,
        };
        assert_eq!(name_of("+5").as_deref(), Some("+5"));
        assert_eq!(
            name_of("+99999999999999999999999999").as_deref(),
            Some("+99999999999999999999999999")
        );
        assert_eq!(name_of("+").as_deref(), Some("+"));
        assert_eq!(name_of("+1.5").as_deref(), Some("+1.5"));
        // The `-` sign is still a number (print emits it for a negative), so it must NOT become a Name.
        assert!(matches!(
            SexprReader::new("-5").read_node(),
            Some(SNode::Int(_))
        ));
    }

    // ── ty_to_wit_desc: the Ty → WIT schema-descriptor mapping (schema-hash phase-1a emit) ───────────

    /// Build the WIT descriptor for the solved type of expression `expr` (in a def `(f) EXPR`) and read it
    /// back through `wit_world::parse_wit_type` — a compact readable assertion that reuses the landed
    /// descriptor reader (the same reader the kernel-shape `parse_target_world` composes).
    fn desc_wit(src_expr: &str) -> Option<crate::wit_world::WitType> {
        let src = format!("(module m (def (f) {src_expr}) (export f))");
        let mut db = Db::load(crate::testkit::parse(&src));
        let body = body_of(&mut db, "f");
        let ty = crate::infer::type_of(&mut db, body);
        let mut b = crate::ast::Builder::new();
        let root = ty_to_wit_desc(&mut db, &mut b, &ty)?;
        let arenas = b.finish(root);
        crate::wit_world::parse_wit_type(&arenas, arenas.root)
    }

    #[test]
    fn ty_to_wit_desc_maps_prims_and_compounds_to_the_kernel_descriptor_forms() {
        use crate::wit_world::WitType as W;
        // Scalars.
        assert_eq!(
            desc_wit("42"),
            Some(W::S64),
            "a bare int literal grounds to s64"
        );
        assert_eq!(desc_wit("true"), Some(W::Bool));
        assert_eq!(desc_wit("\"hi\""), Some(W::String));
        // List<T> and the Bytes-is-list<u8> spelling (NOT a bytes prim — would hash-diverge).
        assert_eq!(
            desc_wit("(list 1 2 3)"),
            Some(W::List(Box::new(W::S64))),
            "list<s64>"
        );
        assert_eq!(
            desc_wit("b\"\\x00\""),
            Some(W::List(Box::new(W::U8))),
            "Bytes emits list<u8>, never a bytes primitive"
        );
        // A tuple is positional.
        assert_eq!(
            desc_wit("(tuple 1 true)"),
            Some(W::Tuple(vec![W::S64, W::Bool])),
            "tuple<s64, bool>, positional"
        );
        // A MAP reflects as list<tuple<K,V>> (the sorted-key value-form) — reachable as an effect-op type, so
        // covered for the schema-hash S3 hard gate. `WitType` has no Map variant, so it reads back as the
        // list<tuple> it emits.
        assert_eq!(
            desc_wit("(map (1 2))"),
            Some(W::List(Box::new(W::Tuple(vec![W::S64, W::S64])))),
            "Map<s64,s64> emits list<tuple<s64,s64>>"
        );
        // A SET reflects as list<E> (the sorted-element value-form) — reachable as an effect-op type.
        assert_eq!(
            desc_wit("(Set.of (list 1))"),
            Some(W::List(Box::new(W::S64))),
            "Set<s64> emits list<s64>"
        );
    }

    #[test]
    fn ty_to_wit_desc_spells_option_as_option_not_a_some_none_variant() {
        use crate::wit_world::WitType as W;
        // Option is a Ty::Sum over the prelude Option decl — but it MUST emit ("option" <inner>), matching a
        // built-in's wasmtime-reflected option<T>. A bare (Some|None) variant would hash-diverge. `(Some 5)`
        // has type `Option Int64`.
        assert_eq!(
            desc_wit("(Some 5)"),
            Some(W::Option(Box::new(W::S64))),
            "Option Int64 emits option<s64>, not a Some|None variant"
        );
    }

    #[test]
    fn ty_to_wit_desc_records_are_name_sorted_matching_the_kernel() {
        use crate::wit_world::WitType as W;
        // A record's fields must be NAME-SORTED (the canonical order all 3 producers agree on). rcdzc's
        // Ty::Record is a name-sorted BTreeMap, so iterating it in natural order already emits sorted fields
        // — the sort is free. Declared here in NON-sorted order (`z` before `a`) to prove the emit sorts.
        match desc_wit("(record (z 1) (a true))") {
            Some(W::Record(fields)) => {
                let names: Vec<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
                assert_eq!(names, vec!["a", "z"], "record fields emit name-sorted");
                assert_eq!(fields[0].1, W::Bool, "field a: bool");
                assert_eq!(fields[1].1, W::S64, "field z: s64");
            }
            other => panic!("expected a record descriptor, got {other:?}"),
        }
    }

    #[test]
    fn ty_to_wit_desc_declines_the_reify_path_declining_domain_bigint_rational_symbol_qty() {
        // REVIEWER PRECISION (S3 hard gate, b2843b647): the `_ => None` decline domain is NOT just
        // Fn/Cont/Var. `Ty::BigInt`, `Ty::Rational`, `Ty::Symbol`, and `Ty::Qty` ALSO have no WIT
        // descriptor form and fall to the same decline arm — and they ARE reachable as effect-op
        // param/result types (14b `Acc.grow -> BigInt`, `Avg.sample -> Rational`, `Reg.intern -> Symbol`,
        // 14 `Env.width -> Qty`). They do not regress the S3 mandatory-schema_hash flip TODAY only because
        // every such occurrence is a HANDLED effect (discharged in-language, never reaching the reify /
        // schema-hash path). This pins the decline DOMAIN explicitly so the S3 flip's invariant — "no
        // REIFYING world-effect op is typed with a declining type" — verifies the right thing: a future
        // host op returning e.g. a Qty must be COVERED here or caught by the flip's decline check, never
        // silently declining (which would regress under mandatory schema_hash).
        use crate::ty::{Ty, Unit};
        let mut db = Db::load(crate::testkit::parse("(module m (def (f) 0) (export f))"));
        let mut decline = |ty: &Ty| {
            let mut b = crate::ast::Builder::new();
            ty_to_wit_desc(&mut db, &mut b, ty)
        };
        assert!(
            decline(&Ty::BigInt).is_none(),
            "BigInt has no WIT descriptor form → declines"
        );
        assert!(
            decline(&Ty::Rational).is_none(),
            "Rational has no WIT descriptor form → declines"
        );
        assert!(
            decline(&Ty::Symbol).is_none(),
            "Symbol has no WIT descriptor form → declines"
        );
        assert!(
            decline(&Ty::Qty {
                inner: Box::new(Ty::int64()),
                unit: Unit::one()
            })
            .is_none(),
            "Qty has no WIT descriptor form → declines (it erases to its inner before emit, but ty_to_wit_desc sees the Qty)"
        );
        // A COMPOUND carrying a declining leaf declines transitively (the `?` short-circuit) — so a
        // `list<BigInt>` or `tuple<Bytes, Symbol>` op type is caught too, not just the bare scalar.
        assert!(
            decline(&Ty::List(Box::new(Ty::BigInt))).is_none(),
            "list<BigInt> declines transitively (declining leaf short-circuits the compound)"
        );
        assert!(
            decline(&Ty::Tuple(vec![Ty::Bytes, Ty::Symbol].into())).is_none(),
            "tuple<Bytes, Symbol> declines transitively"
        );
        // CONTROL: the covered domain still builds (guards against an over-broad decline regressing coverage).
        assert!(decline(&Ty::Bytes).is_some(), "Bytes is covered (list<u8>)");
        assert!(
            decline(&Ty::Map(Box::new(Ty::int64()), Box::new(Ty::int64()))).is_some(),
            "Map is covered (the b2843b647 arm) — the fix under review"
        );
    }

    #[test]
    fn effect_schema_descriptor_declines_a_whole_effect_with_a_declining_op_type() {
        // The effect-level face of the S3 invariant: `effect_schema_descriptor` (hence
        // `effect_has_schema_descriptor`, the SINGLE-SOURCE gate the reify emit + typing both call)
        // DECLINES the WHOLE effect if ANY op param/result type declines — it never bakes a partial
        // descriptor (a wrong identity). So a REIFYING world-effect typed with a declining type flips
        // `effect_has_schema_descriptor` to false, which is exactly the condition the S3 mandatory flip
        // must gate-verify (rather than silently regressing to schema_hash None under mandatory).
        // `Reg.intern -> Symbol` is the shape (14b:5720); Symbol declines → the effect declines.
        let src = "(module m (effect Reg (op intern (-> String Symbol))) (def (f) 0) (export f))";
        let mut db = Db::load(crate::testkit::parse(src));
        let synth = db.effect_decl_by_name("Reg").expect("effect Reg declared");
        let decl = db
            .effect_decl_by_synth(synth)
            .expect("effect decl present")
            .occ;
        assert!(
            !effect_has_schema_descriptor(&mut db, decl),
            "an effect with a Symbol-typed op has no schema descriptor (declines, never a partial identity)"
        );
        // CONTROL: a same-shaped effect over COVERED types builds — proving it's the declining type that
        // declines the effect, not the effect machinery.
        let src_ok = "(module m (effect Reg (op intern (-> String Bytes))) (def (f) 0) (export f))";
        let mut db_ok = Db::load(crate::testkit::parse(src_ok));
        let synth_ok = db_ok
            .effect_decl_by_name("Reg")
            .expect("effect Reg declared");
        let decl_ok = db_ok
            .effect_decl_by_synth(synth_ok)
            .expect("effect decl present")
            .occ;
        assert!(
            effect_has_schema_descriptor(&mut db_ok, decl_ok),
            "the same effect over covered types (String→Bytes) builds a descriptor"
        );
    }

    /// The (params, result) an effect op's declared arrow reduces to — used to build its schema `wit_func_sig`.
    fn op_arrow_tys(src_effect_decl: &str, op_idx: usize) -> (Vec<crate::ty::Ty>, crate::ty::Ty) {
        let src = format!("(module m {src_effect_decl} (def (f) 0) (export f))");
        let mut db = Db::load(crate::testkit::parse(&src));
        // `effect_decl_by_name` returns the effect's SYNTH RECORD occurrence; `effect_decl_by_synth` maps
        // that back to the EffectDecl (whose `occ` is the decl occurrence, distinct from the synth record).
        let synth = db.effect_decl_by_name("E").expect("effect E declared");
        let ty_occ = db
            .effect_decl_by_synth(synth)
            .expect("effect decl present")
            .ops[op_idx]
            .ty
            .expect("op has a declared type");
        op_arrow_param_result_tys(&mut db, ty_occ).expect("arrow reduces")
    }

    #[test]
    fn op_arrow_param_result_tys_peels_curried_params_and_elides_a_unit_arg() {
        use crate::ty::Ty;
        // `(op emit (-> Int64 Bytes Unit))` — two positional params, Unit result.
        let (params, result) = op_arrow_tys("(effect E (op emit (-> Int64 Bytes Unit)))", 0);
        assert_eq!(params.len(), 2, "two positional params");
        assert!(matches!(params[0], Ty::Int(_)), "param 0 is Int");
        assert_eq!(params[1], Ty::Bytes, "param 1 is Bytes");
        assert_eq!(result, Ty::Unit, "result is Unit");

        // ELIDED UNIT: `(op poll (-> Unit Bytes))` is a NO-parameter op (a unit arg is "nothing") — the
        // lone Unit param drops, matching the kernel's Unit-arg built-in shapes (wit_func_sig(&[], …)).
        let (params, result) = op_arrow_tys("(effect E (op poll (-> Unit Bytes)))", 0);
        assert!(params.is_empty(), "a lone Unit param elides to zero params");
        assert_eq!(result, Ty::Bytes, "result is Bytes");
    }

    /// Render an rcdzc descriptor subtree as `(head child…)`, atoms tagged by head kind (`N:`/`S:`), so an
    /// op sig's exact shape (name-free `(param desc)`, head kinds) is visible in the assertion.
    fn render_desc(a: &crate::ast::Arenas, id: StructId) -> String {
        match a.get(id) {
            crate::ast::Struct::Atom(_) => {
                if let Some(n) = a.as_name(id) {
                    format!("N:{n}")
                } else if let Some(s) = a.as_str(id) {
                    format!("S:{s}")
                } else {
                    panic!("descriptor atom is neither Name nor Str")
                }
            }
            crate::ast::Struct::List(children) => {
                let parts: Vec<String> = children.iter().map(|&c| render_desc(a, c)).collect();
                format!("({})", parts.join(" "))
            }
        }
    }

    #[test]
    fn effect_schema_descriptor_builds_name_free_op_sigs_sorted_by_op_name() {
        // A userspace effect with two ops declared in NON-sorted order (send before collect... actually
        // `send` > `collect`, so declaration order != name order). Bytes → list<u8>, Unit result elides.
        let src = "(module m (effect E (op send (-> Bytes Unit)) (op collect (-> Unit Bytes))) (def (f) 0) (export f))";
        let mut db = Db::load(crate::testkit::parse(src));
        let synth = db.effect_decl_by_name("E").expect("effect E declared");
        let decl = db
            .effect_decl_by_synth(synth)
            .expect("effect decl present")
            .occ;
        let mut b = crate::ast::Builder::new();
        let root = effect_schema_descriptor(&mut db, &mut b, decl).expect("descriptor builds");
        let arenas = b.finish(root);
        // Expected: (effect E (op collect (func (result (list u8)))) (op send (func (param (list u8)) (result unit))))
        //  - ops SORTED by name: collect before send (declaration order was send, collect).
        //  - collect: (-> Unit Bytes) → no params (Unit elides), result = list<u8>.
        //  - send: (-> Bytes Unit) → one positional param (list<u8>), NO param name (ruling B), result unit.
        //  - Bytes → ("list" (u8)); the "list" head is a Str (S:), u8 a Name prim (N:).
        assert_eq!(
            render_desc(&arenas, arenas.root),
            "(N:effect N:E \
             (N:op N:collect (N:func (N:result (S:list (N:u8))))) \
             (N:op N:send (N:func (N:param (S:list (N:u8))) (N:result (S:unit)))))",
            "name-free positional op sigs, ops sorted by name, Bytes as list<u8>, Unit result/elided arg"
        );
    }

    #[test]
    fn userspace_effect_family_kind_is_effect_slash_name() {
        // The async-reify `kind` family string for a userspace effect = "effect/" + the declared name
        // (ruling A — the register-by-string family a handler claims; the op is implicit in the payload).
        let src = "(module m (effect Weather (op fetch (-> Bytes Bytes))) (def (f) 0) (export f))";
        let db = Db::load(crate::testkit::parse(src));
        let synth = db
            .effect_decl_by_name("Weather")
            .expect("effect Weather declared");
        let decl = db.effect_decl_by_synth(synth).expect("effect decl").occ;
        assert_eq!(
            userspace_effect_family_kind(&db, decl).as_deref(),
            Some("effect/Weather"),
            "kind = effect/<effect-name>"
        );
    }

    #[test]
    fn reify_effect_to_tuple_builds_the_3_field_record_for_a_single_bytes_arg() {
        // reify_effect_to_tuple lowers a NON-import world-effect perform to its 3-field effect-request
        // record (v-ah ruling S2): { correlation = None, kind = "effect/<name>", payload = Some(<arg>) },
        // NO target column. Unit-test the lowering directly (the end-to-end perform fixture is gated on
        // v-inference's typing branch): feed a Bytes-typed arg node + call reify, assert the Core::Record's
        // three fields carry the right values. A Bytes arg reifies with NO encode (the arg IS the payload).
        let mut db = Db::load(crate::testkit::parse(
            "(module m (def (f) b\"hi\") (export f))",
        ));
        let arg = body_of(&mut db, "f"); // a Bytes-typed node (b"hi")
        assert!(
            matches!(crate::infer::type_of(&mut db, arg), crate::ty::Ty::Bytes),
            "the test arg is Bytes-typed"
        );
        let core = reify_effect_to_tuple(&mut db, "model", &[arg], None, None);
        let Core::Record { fields } = core else {
            panic!("reify must build a Core::Record, got {core:?}");
        };
        // Exactly three fields, name-sorted: correlation, kind, payload — NO target column (S2).
        let names: Vec<&str> = fields.keys().map(|s| s.name.as_ref()).collect();
        assert_eq!(
            names,
            vec!["correlation", "kind", "payload"],
            "3-field record, name-sorted, NO target column (ruling S2)"
        );
        // kind = the ConstStr "effect/model".
        let kind_occ = *fields
            .iter()
            .find(|(s, _)| s.name.as_ref() == "kind")
            .unwrap()
            .1;
        assert!(
            matches!(core_of(&mut db, kind_occ), Core::ConstStr(s) if &*s == "effect/model"),
            "kind = \"effect/model\""
        );
        // payload = Some(<the bytes arg>) — a SumNew (the Some variant) carrying the arg; a Bytes arg
        // reifies with no encode, so the payload's inner IS the arg node. correlation = None (a nullary
        // SumNew). Both lower to Core::SumNew (the Option variants).
        let payload_occ = *fields
            .iter()
            .find(|(s, _)| s.name.as_ref() == "payload")
            .unwrap()
            .1;
        assert!(
            matches!(core_of(&mut db, payload_occ), Core::SumNew { payloads, .. } if payloads.len() == 1),
            "payload = Some(<arg>): a single-payload SumNew (the Some variant)"
        );
        let corr_occ = *fields
            .iter()
            .find(|(s, _)| s.name.as_ref() == "correlation")
            .unwrap()
            .1;
        assert!(
            matches!(core_of(&mut db, corr_occ), Core::SumNew { payloads, .. } if payloads.is_empty()),
            "correlation = None: a nullary SumNew (the None variant)"
        );
    }

    #[test]
    fn reify_with_a_decl_emits_the_schema_descriptor_field_bytes() {
        // PHASE-3 PRODUCER-BAKE (v-effects (B)): when reify is given the effect DECL, it emits a
        // `schema_descriptor` field = the RAW `effect_schema_descriptor` tree encoded to Bytes (the kernel
        // decodes + hashes it kernel-side, the one-hasher — NOT a rcdzc-baked hash, which would mismatch the
        // kernel's re-canonicalized hash per ruling A). Assert: with a decl whose descriptor BUILDS, the
        // record gains a 4th field `schema_descriptor` whose value is a Bytes leaf carrying
        // codec::encode(descriptor). With decl=None, no such field (the base-case tests above pin that).
        let src = "(module m (effect E (op send (-> Bytes Unit))) (def (f) b\"hi\") (export f))";
        let mut db = Db::load(crate::testkit::parse(src));
        let synth = db.effect_decl_by_name("E").expect("effect E declared");
        let decl = db
            .effect_decl_by_synth(synth)
            .expect("effect decl present")
            .occ;
        let arg = body_of(&mut db, "f"); // a Bytes-typed node (b"hi")
        let core = reify_effect_to_tuple(&mut db, "E", &[arg], None, Some(decl));
        let Core::Record { fields } = core else {
            panic!("reify must build a Core::Record, got {core:?}");
        };
        // 4 fields now, name-sorted: correlation < kind < payload < schema_descriptor — the decl-bearing
        // reify adds the schema_descriptor field (the base-case tests above, decl=None, pin the 3-field
        // shape, so this delta IS the phase-3 producer-bake firing). The field's value is the encoded
        // descriptor Bytes (the kernel decodes + hashes it); the descriptor's OWN content is pinned by
        // effect_schema_descriptor_builds_name_free_op_sigs_sorted_by_op_name, so here we assert only that the
        // field is EMITTED (the wire carries it) — its lowered value is a constant bytes value like any other.
        let names: Vec<&str> = fields.keys().map(|s| s.name.as_ref()).collect();
        assert_eq!(
            names,
            vec!["correlation", "kind", "payload", "schema_descriptor"],
            "the decl-bearing reify adds the schema_descriptor field (name-sorted last)"
        );
    }

    #[test]
    fn reify_effect_to_tuple_declines_a_multi_arg_or_structured_payload() {
        // The Bytes-payload decomposition: a MULTI-arg perform (needs a value-form-of-args tuple = the
        // in-fold value-encode, R2, not built) declines cleanly. Two Bytes args → decline.
        let mut db = Db::load(crate::testkit::parse(
            "(module m (def (f) b\"a\") (def (g) b\"b\") (export f))",
        ));
        let a = body_of(&mut db, "f");
        let b = body_of(&mut db, "g");
        let core = reify_effect_to_tuple(&mut db, "emit", &[a, b], None, None);
        assert!(
            matches!(core, Core::Poison(_)),
            "a multi-arg world-effect payload (no @resource skip) needs the R2 in-fold value-encode → declines cleanly, got {core:?}"
        );
    }

    #[test]
    fn reify_effect_to_tuple_value_encodes_a_single_structured_payload() {
        // R2 CARVE-OUT (v-effects-unblocked 2026-08-15: S3 identity flip + the Value.encode prim landed). A
        // SINGLE STRUCTURED (non-Bytes) payload arg no longer declines — it reifies by VALUE-ENCODING the arg
        // in-fold: payload = Some((. Value encode) arg), which lowers to Core::ValueEncode. This unblocks
        // Model.request(ModelRequest)/close-reply(structured). (A Bytes arg still rides verbatim — see
        // reify_effect_to_tuple_builds_the_3_field_record_for_a_single_bytes_arg; MULTI-arg still declines —
        // see reify_effect_to_tuple_declines_a_multi_arg_or_structured_payload. The payload is opaque to the
        // kernel parse; the value-form desc is for the downstream consumer's Value.decode.)
        let mut db = Db::load(crate::testkit::parse(
            "(module m (def (f) (tuple 1 2)) (export f))", // a (Tuple Int64 Int64) — structured, non-Bytes
        ));
        let arg = body_of(&mut db, "f");
        assert!(
            matches!(crate::infer::type_of(&mut db, arg), crate::ty::Ty::Tuple(_)),
            "the test arg is a structured (Tuple) value, not Bytes"
        );
        let core = reify_effect_to_tuple(&mut db, "model", &[arg], None, None);
        let fields = match core {
            Core::Record { fields } => fields,
            other => panic!("a single structured payload must REIFY (not decline), got {other:?}"),
        };
        // payload = Some(<Value.encode arg>): a Some SumNew whose single payload lowers to Core::ValueEncode.
        let payload_occ = *fields
            .iter()
            .find(|(s, _)| s.name.as_ref() == "payload")
            .unwrap()
            .1;
        let inner = match core_of(&mut db, payload_occ) {
            Core::SumNew { payloads, .. } if payloads.len() == 1 => payloads[0],
            other => panic!("payload must be Some(<one>): a single-payload SumNew, got {other:?}"),
        };
        assert!(
            matches!(core_of(&mut db, inner), Core::ValueEncode { .. }),
            "the structured payload is value-encoded in-fold (Core::ValueEncode), got {:?}",
            core_of(&mut db, inner)
        );
    }

    #[test]
    fn reify_effect_to_tuple_skips_the_resource_arg_so_a_2_arg_emit_reifies() {
        // The @resource-split (ruling A): Emit.send(@resource dest, payload) is a 2-arg perform; with
        // resource=Some(0) the reify routes arg0 (dest) to its own `target` wire field (NOT dropped — the
        // dest is a runtime value SEC-F1 authorizes) and the payload is the SINGLE remaining arg (arg1) →
        // reifies with NO R2 (the resource-SPLIT, not value-encode, unblocks 2-arg emits). Both args Bytes:
        // dest (arg0 → target field) + payload (arg1 → Some(bytes)). This is v-ah's migrated reducer_kv
        // Emit.send shape. → 4-field record {correlation, kind, payload, target}.
        let mut db = Db::load(crate::testkit::parse(
            "(module m (def (f) b\"dest\") (def (g) b\"body\") (export f))",
        ));
        let dest = body_of(&mut db, "f");
        let body = body_of(&mut db, "g");
        let core = reify_effect_to_tuple(&mut db, "emit", &[dest, body], Some(0), None);
        let Core::Record { fields } = core else {
            panic!(
                "a 2-arg emit with @resource=arg0 reifies (dest→target, payload=arg1), got {core:?}"
            );
        };
        let names: Vec<&str> = fields.keys().map(|s| s.name.as_ref()).collect();
        assert_eq!(
            names,
            vec!["correlation", "kind", "payload", "target"],
            "4-field record: the @resource dest rides its own `target` field (name-sorted after payload)"
        );
        // payload = Some(arg1=body) — the resource arg0 (dest) went to `target`, not the payload.
        let payload_occ = *fields
            .iter()
            .find(|(s, _)| s.name.as_ref() == "payload")
            .unwrap()
            .1;
        assert!(
            matches!(core_of(&mut db, payload_occ), Core::SumNew { payloads, .. } if payloads.len() == 1),
            "payload = Some(the non-resource arg): the @resource dest went to `target`, payload is arg1"
        );
        // target = the bare dest value (arg0), NOT wrapped in Some — the kernel reads it via read_bytes.
        let target_occ = *fields
            .iter()
            .find(|(s, _)| s.name.as_ref() == "target")
            .unwrap()
            .1;
        assert!(
            !matches!(core_of(&mut db, target_occ), Core::SumNew { .. }),
            "target = the bare @resource dest value (arg0), a raw Bytes field (not an Option)"
        );
        // And a target-FREE effect (resource=None) with a single Bytes arg reifies to a 3-field record (NO
        // target field) — the target field only appears when a resource marker designates a dest.
        let single = body_of(&mut db, "g");
        let Core::Record { fields: tf_fields } =
            reify_effect_to_tuple(&mut db, "tool", &[single], None, None)
        else {
            panic!("a target-free single-Bytes-arg effect (resource=None) reifies to a record");
        };
        let tf_names: Vec<&str> = tf_fields.keys().map(|s| s.name.as_ref()).collect();
        assert_eq!(
            tf_names,
            vec!["correlation", "kind", "payload"],
            "a target-free effect stays 3-field: no @resource → no target column"
        );
    }

    #[test]
    fn reify_effect_to_tuple_zero_arg_perform_reifies_with_payload_none() {
        // A payload-FREE world-effect (a zero-arg perform, e.g. a fire-and-forget `Now.now()`) reifies with
        // payload = None — the 3-field record still builds, just with the None payload variant. Pins the
        // zero-arg arm ([] => (None)) alongside the single-Bytes-arg (Some) + multi-arg (decline) cases.
        let mut db = Db::load(crate::testkit::parse("(module m (def (f) 0) (export f))"));
        let core = reify_effect_to_tuple(&mut db, "now", &[], None, None);
        let Core::Record { fields } = core else {
            panic!("a zero-arg reify still builds a Core::Record, got {core:?}");
        };
        let names: Vec<&str> = fields.keys().map(|s| s.name.as_ref()).collect();
        assert_eq!(
            names,
            vec!["correlation", "kind", "payload"],
            "3-field record even for a zero-arg (payload-free) effect"
        );
        let payload_occ = *fields
            .iter()
            .find(|(s, _)| s.name.as_ref() == "payload")
            .unwrap()
            .1;
        assert!(
            matches!(core_of(&mut db, payload_occ), Core::SumNew { payloads, .. } if payloads.is_empty()),
            "payload = None: a nullary SumNew (the None variant) for a payload-free effect"
        );
    }
}
