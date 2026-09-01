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
    lower_ast_splice_lift, lower_blake3_of, lower_gensym, lower_print, lower_read, lower_type_ast,
    reflect_module_value,
};
// Re-exported at crate scope so `crate::lower::is_ast_float_variant` (the rust backend's escape guard)
// keeps resolving to its new home.
pub(crate) use ast_reflect::expand_macros;
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
pub(crate) use compute::record_spread_desugar;
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

mod arith_fold;
// Arithmetic/boolean/bitwise lowering + constant folding & algebraic simplification lives in
// `arith_fold`; its most-visible items are `pub(crate)`, so the same `pub(crate)` glob re-export
// preserves each item's visibility across the tree.
pub(crate) use arith_fold::*;

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
        // A non-String operand is a TYPE error (`infer` already reports the authoritative CDZ0203
        // "expects an argument of type String"); defer to it via the neutral decline rather than the
        // misleading "runtime string / UTF-8 walk" wording — a constant `5` is not a string at all.
        return runtime_string_op_decline(
            db,
            operand,
            "a runtime string's scalar length needs a UTF-8 decoding walk (byte-len works)",
        );
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
        _ => Core::Poison(Reject::unsupported(
            "a runtime string's scalar length needs a UTF-8 decoding walk (byte-len works)",
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
    // earlier ctor-only gate): the native ctor-LEAF head `#tuple(a b)` (`compound_ctor_leaf`; the legacy
    // STRING-prim head `("tuple" …)` is gone post the M3 reader-flip 2026-08-31) and — crucially — the NAME
    // head `(tuple a b)` / `(record …)` that the ML/paren surface `let (a,b) = …` desugars to (`head_name`;
    // `compound_ctor_leaf` deliberately rejects a name head). An ANNOTATED binder `(: x T)` (head `:`), a bare name, a `bin`/`list`/
    // `map` binder, or a sum-ctor pattern (`(Some x)`) are NOT caught — head_name is `:`/`bin`/`Some`/… ≠
    // tuple|record — so they keep the ordinary path (annotated-binder diagnostics, bin/list/map reclaim, the
    // CDZ0210 refutable-sum reject all unchanged).
    // The gate below misfires on a DO-BLOCK VALUE-DEF, which `compute.rs` routes here as a SELF-KEYED
    // `(V, V)` binding (binder-occ == init-occ == the value node V, keyed on V for the keep-analysis) —
    // a bare-name binding `(def p0 V)`, NOT a destructure. When V is a record/tuple LITERAL, `.0` IS a
    // compound ctor, so the `compound_ctor`/`head_name` probe below wrongly reads it as a destructure
    // PATTERN and routes `(def p0 #record(…))` through a single-arm match of the record against ITSELF →
    // match_tree raises a spurious CDZ0210 "non-exhaustive sum match" (corpus-15 regression: a do-local
    // record binding + projection declined). A GENUINE destructure `let` has a pattern node DISTINCT from
    // its init (`(let (((tuple a b) init)) …)` — pattern ≠ init); the self-keyed value-def has `.0 == .1`.
    // So require `.0 != .1`: the destructure fast-path fires only for a real distinct-pattern destructure,
    // and a bare-name value-def binding keeps the ordinary path (its projection stays a runtime `Proj`).
    if bindings.len() == 1
        && bindings[0].0 != bindings[0].1
        && (matches!(
            db.ast.compound_ctor_leaf(bindings[0].0),
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
    // RUNTIME `?` SHORT-CIRCUIT (BRICK 3b, `DESIGN-try-operator-rcdzc.md` §3.2/§4 v1). A single-binding
    // `(let ((x (try e))) body)` whose operand `e` is a RUNTIME (non-constant) fallible value — its
    // Some/Ok-vs-None/Err discriminant is decided at run time (the operator's `try(Int64.checked-add(max, v))`
    // over a runtime `v`) — desugars to a `Core::MatchSum` over `e`, exactly the nested-`match` the `?`
    // collapses (§0). The SUCCESS arm continues with `body` (the payload binder `x` resolves to a
    // `Core::SumPayload` of `e` — installed as a core-override on the `try` init, mirroring how a real
    // match-arm binder resolves); the FAILURE (default) arm yields `e`'s own failure value (`None`/`Err r`,
    // already `T_B`-typed), which flows out as the boundary body's value — the abortive short-circuit. The
    // scrutinee `e` is materialized ONCE by `MatchSum`'s scrutinee slot, so the runtime operand (and any
    // host call it makes) evaluates exactly once and every payload read + the failure re-reference read the
    // slot. Emitted through the existing `MatchSum` backend — no new `block`/`br`. Like the constant-failure
    // fold above, this assumes the `let` is in BOUNDARY-TAIL position (its value IS the boundary's value);
    // a `?` under a non-fallible/non-tail wrapper is the same accepted v1 limitation the constant path has.
    // Restricted to a SINGLE-binding `let` (the ML `let x = e? in …` / `do (def x e?) …` idiom desugars to
    // nested single-binding lets); a multi-binding `let` containing a runtime `?` keeps the decline below.
    if bindings.len() == 1
        && let Resolved::Try { operand } = resolved_of(db, bindings[0].1)
        && let Some(success_disc) = success_disc_of(db, operand)
        && let Some(failure_disc) = failure_disc_of(db, operand)
        && !matches!(core_of(db, operand), Core::SumNew { .. } | Core::Poison(_))
        && let Some(failure) = runtime_try_failure_value(db, operand, node)
    {
        // A constant success/failure operand never reaches here (`compute.rs`'s `Resolved::Try` folds a
        // constant `SumNew` to the payload / a `Core::Break`, and the const-failure loop above handles the
        // break); the `SumNew`/`Poison` guard drops those + an ill-typed operand, so only a runtime fallible
        // value builds the desugar.
        let (_name_occ, init) = bindings[0];
        trace!(target: "rcdzc::lower", node = node.0, "runtime `?` → MatchSum short-circuit (success arm continues, failure arm re-wraps the failure value)");
        // The payload binder `x` (a reference resolves to the `try` init occurrence) reads the success
        // payload of the materialized scrutinee — the same `SumPayload` shape a real `(Some x)` arm binder
        // resolves to (`core.rs` `SumArm`), so the existing match reclaim governs its dup/drop. Installed as
        // a core-override so it wins over the init's memoized runtime-decline core.
        db.core_override.insert(
            init,
            Core::SumPayload {
                scrutinee: operand,
                path: std::rc::Rc::from(vec![crate::core::PathStep::Payload]),
            },
        );
        let root = crate::core::SumCont::Switch {
            path: std::rc::Rc::from(Vec::<crate::core::PathStep>::new()),
            arms: vec![
                crate::core::SumArm {
                    disc: Some(success_disc),
                    cont: crate::core::SumCont::Leaf(body),
                },
                // The FAILURE arm — the single failure variant (`None` for an Option, `Err` for a Result;
                // each is a two-variant sum, so the two explicit-disc arms are exhaustive) — yields a FRESHLY
                // RE-WRAPPED failure value (`None` / `Err r`, `r` extracted from the matched scrutinee).
                // Re-wrapping — rather than returning the scrutinee itself — keeps the matched shell from
                // ESCAPING through this arm: the scrutinee is fully consumed on BOTH paths (its shell dropped
                // after the success payload borrow, matched-then-reconstructed on the failure path), so no
                // cell leaks. The disc is EXPLICIT (not a `None` wildcard default) so the failure re-wrap's
                // `SumPayload` extraction sits under a bound match arm the RUST backend recognizes (a
                // wildcard-default `SumPayload` is "no bound match arm" there). The value is `T_B`-typed and
                // flows out as the boundary body's value.
                crate::core::SumArm {
                    disc: Some(failure_disc),
                    cont: crate::core::SumCont::Leaf(failure),
                },
            ],
        };
        return Core::MatchSum {
            scrutinee: operand,
            root: std::rc::Rc::new(root),
        };
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
    // Arity-1 `Sub` `(- e)` is no longer special: like arity-1 `Add` `(+ e)` it CURRIES to a partial
    // subtraction (prefix negation deprecated in favour of `Num.neg`/`T.neg`), so a well-formed `(- e)`
    // lowers to a closure (not a poison) and takes the `core_of` path below to `None` — no arity fault.
    // A genuine malformed `(- a b c)` / `(-)` still faults there, uniformly with `Add`.
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
            // `List.concat` (`ListConcat`) is the DIRECT ANALOG of `BytesConcat` (adv-54b) — a fresh-list-
            // building runtime op that CONSUMES/dups both operands (`select.rs`: "vec-concat consumes both
            // operands"; classified a producer, NOT a borrow). A `let`-bound concat used more than once was
            // COPY-PROPAGATED (it was not in this list), RE-BUILDING the concat at each use — and in a CHAINED
            // shape where each binding concats the PREVIOUS one twice (`x1=(concat x0 x0)`, `x2=(concat x1 x1)`,
            // …) the duplication COMPOUNDS to 2^N: an EXPONENTIAL wasm emit + compile time for a linear-size
            // program (operator seq-203; N=14 emitted 136 KB, N≥20 timed out). Keeping it under the >= 2-use
            // rule materializes the list ONCE into a `Core::Let` slot whose handle each use reads (a dup =
            // an rc bump), so a linear program emits LINEAR output. Regression-free NOW (the kept-binding
            // dup/consume EMIT is the same sound path `BytesConcat` uses post-adv-66 — ListConcat is already
            // CONSUMING in `mark_binder_dups`), exactly as adv-54b widened from the StrSlice/StrToBytes pair.
            | Core::ListConcat { .. }
            // FAMILY WIDENING (the #7416 ListConcat class — the exponential-emit boxed-rep fix, seq-203):
            // the boxed-COLLECTION PRODUCERS are all fresh-owned-handle producers that CONSUME/dup their
            // operands (v-wasm-opt verified all 8 on BOTH axes @ 48e46259fe: mark_binder_dups is_borrow=false
            // CONSUMING + heap_operand_ownership Owned; no adv-54 kept-emit caveat — the adv-54 3-case
            // regression was a since-fixed mark_binder_dups bug). So a multi-use let-bound collection producer
            // was COPY-PROPAGATED (not in this list) → rebuilt at each use → an EXPONENTIAL chain (Set.union /
            // List.push / Map.insert chained, each binding consuming the previous 2x, 2^N). Keeping them under
            // the >= 2-use rule materializes ONCE into a `Core::Let` slot (handle read per use, a dup=rc bump),
            // so a linear program emits LINEAR output. LIST: `List.push`/`List.prepend`/`List.update`; MAP:
            // `Map`-empty-build / `Map.insert`/`Map.remove`; SET: `Set.of`. (`SumNew` — the sum-payload rep —
            // verified too but deferred to a separate increment for its ubiquity; `SetAlgebra`=`Set.union` is
            // the confirmed exp witness, pending its own verify.) v-wasm-opt co-lands a per-kind emit-size
            // regression gate with each; gate-local + opt-sweep are the per-op empirical arbiter.
            | Core::ListPush { .. }
            | Core::ListPrepend { .. }
            | Core::ListUpdate { .. }
            | Core::MapNew { .. }
            | Core::MapInsert { .. }
            | Core::MapRemove { .. }
            | Core::SetOf { .. }
            // `Set.union`/`intersect`/`diff` (`Core::SetAlgebra`) is the TRUE Set exp witness (v-compiler-perf
            // confirmed a `Set.union` chain still 2^N on main — `SetOf` alone is a constructor, not the
            // binary-same-operand reproducer). A binary op that CONSUMES/dups both set operands + returns a
            // fresh owned set (v-wasm-opt verified both axes: heap_operand_ownership Owned select.rs:7473,
            // mark_binder_dups seq[(lhs,F),(rhs,F)] CONSUMING reclaim.rs:3269) — the exact `ListConcat` shape
            // for sets. Kept under the >= 2-use rule so a `Set.union x x` chain is LINEAR.
            | Core::SetAlgebra { .. }
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
        // A NOMINAL's type surface: the bare NAME for a monomorphic newtype, or the STRUCTURED application
        // `(Box Int64)` for a GENERIC instantiation — same as `Ty::Sum`. Dropping the args rendered a generic
        // `Box Int64` as bare `Box` = `Box <free>` on re-read, so a `(: xs (List Box))` param annotation
        // mismatched the `Box Int64` a `(Box.Wrap n)` produces (CDZ0203, breaker rg1).
        Ty::Nominal { decl, args, .. } => {
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
                // A floating-point three-way `compare` is a PERMANENT carve-out (float offers only the IEEE
                // PARTIAL order — a NaN is unordered — not the total order `compare` reports), so it is a
                // genuine user error, not a not-yet decline: coded CDZ0203 (the same "this operation is not
                // defined on this type" class as bare `=` on a function/type value), keeping the actionable
                // IEEE-partial-order message that points at `<`/`<=`/`>`/`>=`. (v-corpus-declines corpus-
                // deprecation BUCKET-2: assert the code, replacing the former codeless decline.)
                None if float_operand(db, &operands) => Core::Poison(Reject::coded(
                    Code::TypeMismatch,
                    "`compare` needs a total order, but a floating-point type offers only the IEEE partial order (a not-a-number is unordered), so it has no three-way comparison — use the relational operators `<`, `<=`, `>`, `>=` instead",
                )),
                // A COMPOUND (tuple/record/list/sum) whose leaves are NOT all orderable — a float leaf (only
                // the IEEE partial order, §319) or a Set/Map leaf (no blessed order). Reached only after the
                // orderable-compound `ValueCmp` arm above declined, so the compound has an UN-orderable leaf:
                // a PERMANENT carve-out (never a not-yet), the three-way `compare` twin of the boolean-ordering
                // carve-out `lower_comparison` codes (this file, §7143 reconcile). Coded CDZ0203 — the SAME
                // no-total-order `Code::TypeMismatch` family as the pure-float `compare` above and the `<`/`>`
                // compound arm — with the ALL-LEAF message (float OR set/map) + the component-wise route.
                //
                // GATE on the operands being the SAME type (`ta == tb`), NOT merely "some side is a compound":
                // unlike the `<`/`>` node (whose ill-typed form `infer` poisons upstream, so lower never runs),
                // a mismatched `(compare (tuple …) 1)` DOES reach here, and `comparison_compound_ty` reports the
                // tuple side even for that cross-type pair — coding it would DOUBLE-error alongside infer's
                // "different types" CDZ0201 (which the codeless fallback below stays droppable-as-consequent
                // for). The checker unified a well-typed compare's operands, so `ta == tb` is exactly the
                // matched-compound carve-out and excludes every mismatch shape (tuple-vs-int, tuple-vs-record,
                // 2-tuple-vs-1-tuple, …). An under-resolved empty collection (`(compare (list) (list 1.0))` —
                // one side `List ?`) fails `ta == tb` and degrades to the codeless decline below: still a clean
                // refusal, dedup-safe, and vanishingly rare.
                None if {
                    let ta = crate::infer::type_of(db, lhs);
                    ta == crate::infer::type_of(db, rhs)
                        && matches!(
                            ta,
                            crate::ty::Ty::Tuple(_)
                                | crate::ty::Ty::Record(_)
                                | crate::ty::Ty::List(_)
                                | crate::ty::Ty::Sum { .. }
                        )
                } =>
                {
                    Core::Poison(Reject::coded(
                        Code::TypeMismatch,
                        "a compound value with a float, set, or map leaf has no total order, so it has no \
                         three-way `compare` (a float offers only the IEEE partial order; a set/map carries no \
                         blessed order) — compare its orderable components individually",
                    ))
                }
                None => Core::Poison(Reject::decline(
                    "`compare` of this value has no total order the compiler can walk (a float/bytes/set/map leaf, or an un-orderable shape) — compare its orderable components individually",
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
                _ => Core::Poison(Reject::unsupported(
                    "comparison of an integer beyond the machine width is not supported",
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
                // Because this is a PERMANENT carve-out (never a not-yet), it is a coded CDZ0203 — the SAME
                // `Code::TypeMismatch` no-total-order class as the pure-float three-way `compare` carve-out
                // above (`lower_compare`, #7001), Set.to-list over a float leaf (#7076), and the sum-payload
                // float leaf — one family, one code (v-inference/v-corpus-declines reconcile of the compound
                // float-leaf ordering decline; replaces the former codeless decline).
                trace!(target: "rcdzc::lower", op = intrinsic_name(op), "reject: ordering of a compound with an un-orderable (float/set/map) leaf — permanent carve-out (CDZ0203)");
                Core::Poison(Reject::coded(
                    Code::TypeMismatch,
                    "a compound value with a float, set, or map leaf has no total order, so it cannot be \
                     ordered by `<`/`<=`/`>`/`>=` (a float offers only the IEEE partial order; a set/map \
                     carries no blessed order) — order its orderable components individually",
                ))
            } else {
                // EQUALITY (`=`) reaching here is a genuinely unbuilt canonicalization (a Set/Map leaf
                // whose structural `=` the compiler cannot walk) — the honest heap-walk decline, now the
                // umbrella CDZ0900 (`Reject::unsupported`). Uses the `COMPOUND_COMPARISON_DECLINE` marker
                // VERBATIM (it is corpus-referenced + the mismatched-type `dedup_faults` key that recognizes
                // it by `.contains`, and dropping the old "(not yet built)" deferral suffix keeps that match).
                trace!(target: "rcdzc::lower", op = intrinsic_name(op), "decline: equality of a compound value needs a heap walk (CDZ0900)");
                Core::Poison(Reject::unsupported(
                    crate::diag::COMPOUND_COMPARISON_DECLINE,
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
        return Core::Poison(Reject::unsupported(
            "a partially-applied constructor as a runtime value is not lowered to a runtime closure",
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
        // A PERMANENT correct-reject (operator-ruled A): a non-canonical float has no value form, so this
        // is a coded CDZ0201 (Malformed) — the SAME family the non-finite float LITERAL (resolve.rs
        // `FloatInf`/`FloatNan`) and the non-finite const FOLD (arith_fold.rs, #6893) carry, not a codeless
        // decline. (v-cdz-smith/v-corpus-declines seq-286 ledger.)
        return Core::Poison(Reject::coded(
            crate::diag::Code::Malformed,
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

/// The FAILURE-variant discriminant of a fallible value `id` — `None`'s disc for an Option, `Err`'s for a
/// Result (the twin of [`success_disc_of`]). This is the discriminant the runtime `?` desugar's failure
/// (short-circuit) arm matches. `None` if `id` is neither an Option nor a Result.
fn failure_disc_of(db: &mut Db, id: StructId) -> Option<u32> {
    option_discs(db, id)
        .map(|(_, none)| none)
        .or_else(|| result_discs(db, id).map(|(_, err)| err))
}

/// The FAILURE value a runtime `?`/try short-circuits to (`DESIGN-try-operator-rcdzc.md` §3.2): a freshly
/// RE-WRAPPED `None` (Option) / `Err r` (Result), typed `T_B` (the enclosing boundary type, read off the
/// desugared `let` node `boundary`). The failure value is RECONSTRUCTED rather than the matched scrutinee
/// re-read, so the scrutinee shell does not escape the failure arm (it is consumed on both paths — see
/// `lower_let`). For an Option, `None` is a `Core::SumNew` at the none discriminant with NO payloads
/// (`(None)`, the empty-payload form the `List.at` out-of-range fold produces). For a Result, `Err r`
/// re-wraps the error payload `r`, extracted from the matched scrutinee via a `Core::SumPayload` typed with
/// the Err variant's field type (so a scalar error unboxes, a heap error rides as a handle). `None` when
/// `operand` is neither a well-formed Option nor Result (the caller then keeps the decline).
fn runtime_try_failure_value(
    db: &mut Db,
    operand: StructId,
    boundary: StructId,
) -> Option<StructId> {
    let boundary_ty = crate::infer::type_of(db, boundary);
    if let Some((_some, none_disc)) = option_discs(db, operand) {
        return Some(synth_core(
            db,
            Core::SumNew {
                disc: none_disc,
                payloads: Vec::new().into(),
            },
            boundary_ty,
        ));
    }
    if let Some((_ok, err_disc)) = result_discs(db, operand) {
        // The Err variant's payload TYPE, so the extracted `r` unboxes correctly (scalar) or rides as a
        // handle (heap). Read off the operand's solved Result type by variant NAME (never positionally).
        let operand_ty = crate::infer::type_of(db, operand);
        let err_ty = crate::lower::value_form::sum_variant_payload_types(db, &operand_ty)?
            .into_iter()
            .find(|(name, _)| name == "Err")
            .and_then(|(_, tys)| tys.into_iter().next())?;
        let r = synth_core(
            db,
            Core::SumPayload {
                scrutinee: operand,
                path: std::rc::Rc::from(vec![crate::core::PathStep::Payload]),
            },
            err_ty,
        );
        return Some(synth_core(
            db,
            Core::SumNew {
                disc: err_disc,
                payloads: vec![r].into(),
            },
            boundary_ty,
        ));
    }
    None
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
        _ => Core::Poison(Reject::unsupported(
            "Set.of over a runtime list is not supported (a constant list literal only)",
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
        let mismatch = !matches!(
            crate::infer::type_of(db, set),
            crate::ty::Ty::Set(_) | crate::ty::Ty::Var(_) | crate::ty::Ty::Any
        );
        return ill_typed_operand_decline(
            mismatch,
            "Set.contains operand is not a solved set type",
        );
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
        let mismatch = !matches!(
            crate::infer::type_of(db, set),
            crate::ty::Ty::Set(_) | crate::ty::Ty::Var(_) | crate::ty::Ty::Any
        );
        return ill_typed_operand_decline(mismatch, "Set.to-list operand is not a solved set type");
    };
    // An element type with NO total order cannot be enumerated in order — a float leaf (only the IEEE
    // partial order; a not-a-number is unordered) or a set/map leaf (no blessed order). Decline it HERE,
    // in the shared front-end, so BOTH backends AND `cdz check` inherit ONE coded verdict (CDZ0203,
    // ALL-LEAF — matching the ordering #7143 + compare #7210 reconcile that unified float AND set/map to
    // one no-total-order code). Formerly the reject was PER-BACKEND (wasm: float→CDZ0203 / set-map→codeless
    // decline; rust: only float-carrying declined, a set/map-leaf element ENUMERATED via `BTreeSet`'s `Ord`
    // — a blessed order the spec does not offer) → a backend divergence + a check/compile gap. A BARE float
    // element still enumerates (canonical bytes; `float_ok`), so this fires only on an un-orderable SHAPE.
    if !orderable_leaf_or_compound(db, &elem_ty, /*float_ok=*/ true, &mut Vec::new()) {
        return Core::Poison(to_list_unorderable_reject("element"));
    }
    Core::SetToList { set, elem_ty }
}

/// The CDZ0203 no-total-order reject a `Set.to-list` / `Map.to-list` returns when its element / key type
/// cannot be enumerated in a total order — a float leaf (only the IEEE partial order, a not-a-number is
/// unordered) or a set/map leaf (no blessed order). The ENUMERATION face of the ordering/compare
/// no-total-order carve-out (§319 / 03:626); ALL-LEAF (float AND set/map → one CDZ0203), matching the
/// ordering (#7143) + compare (#7210) reconcile. Coded (a PERMANENT carve-out, never a not-yet), returned
/// from the shared front-end so both backends + `cdz check` agree. `what` = "element" (Set) / "key" (Map).
fn to_list_unorderable_reject(what: &str) -> Reject {
    Reject::coded(
        crate::diag::Code::TypeMismatch,
        format!(
            "a to-list enumerates its {what}s in a total order, but this {what} type has no total order — a \
             float leaf offers only the IEEE partial order (a not-a-number is unordered), and a set/map leaf \
             carries no blessed order — so its {what}s cannot be enumerated in order"
        ),
    )
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
                return Err(Reject::unsupported(
                    "a runtime bin match with a dynamically-offset dependent size field is not lowered",
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
                    return Err(Reject::unsupported(
                        "a runtime bin literal int segment after a non-final unsized bytes / utf8 segment is not probed",
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
                // A non-int source or unresolved target bounds can't be range-checked here. A DEFINITE
                // non-int SOURCE (`(Int64.of "x")`) is a TYPE error `infer` already reports as CDZ0203 —
                // defer to it with the neutral decline rather than the misleading "runtime checked
                // conversion out of range" wording (the source is not an integer at all). A genuine INT
                // source (even runtime) keeps the honest conversion decline (its CDZ0900 is the only report).
                let src_ty = crate::infer::type_of(db, args[0]);
                if src_ty.is_fully_solved() && !matches!(src_ty, crate::ty::Ty::Int(_)) {
                    return Core::Poison(Reject::decline(crate::lower::ILL_TYPED_OPERAND_DECLINE));
                }
                return Core::Poison(Reject::unsupported(
                    "a runtime checked integer conversion (T.of) that could be out of range is not supported (convert a constant, widen instead of narrow, or use T.wrap)",
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
/// Whether `ty` has a FLOAT leaf anywhere (transitively) — the float-no-total-order face of an
/// un-orderable value. Used to SCOPE the compound-ordering / to-list rejects: an un-orderable value
/// caused by a float leaf is a PERMANENT no-total-order carve-out (float offers only the IEEE partial
/// order) → coded CDZ0203 "use `<`/`<=`/`>`/`>=`"; one caused by a Set/Map leaf (no blessed order at all)
/// is a separate class that stays a codeless decline. Mirrors `orderable_leaf_or_compound`'s recursion,
/// closing recursive sums on `seen`. (A `Set`/`Map` whose ELEMENT/value is a float still has a float leaf —
/// it is the enclosing collection's own orderability that differs; here we only ask "is a float present".)
pub(crate) fn type_has_float_leaf(
    db: &mut Db,
    ty: &crate::ty::Ty,
    seen: &mut Vec<crate::ast::StructId>,
) -> bool {
    use crate::ty::Ty;
    match ty {
        Ty::Float(_) => true,
        Ty::Tuple(elems) => {
            let elems: Vec<Ty> = elems.iter().cloned().collect();
            elems.iter().any(|e| type_has_float_leaf(db, e, seen))
        }
        Ty::List(elem) | Ty::Set(elem) => {
            let elem = (**elem).clone();
            type_has_float_leaf(db, &elem, seen)
        }
        Ty::Map(k, v) => {
            let (k, v) = ((**k).clone(), (**v).clone());
            type_has_float_leaf(db, &k, seen) || type_has_float_leaf(db, &v, seen)
        }
        Ty::Record(fields) => {
            let vals: Vec<Ty> = fields.values().cloned().collect();
            vals.iter().any(|v| type_has_float_leaf(db, v, seen))
        }
        Ty::Sum { decl, .. } => {
            if seen.contains(decl) {
                return false; // recursive back-edge — no new float leaf via this cycle
            }
            seen.push(*decl);
            let vc = db.type_decl_by_occ(*decl).map(|t| t.variants.len());
            let mut found = false;
            if let Some(vc) = vc {
                for disc in 0..vc {
                    let ctor = db
                        .type_decl_by_occ(*decl)
                        .and_then(|t| t.variants.get(disc))
                        .and_then(|v| v.ctor);
                    if let Some(ctor) = ctor
                        && let Some(payload_ty) =
                            crate::infer::payload_ty_at_instantiation(db, ctor, ty)
                        && type_has_float_leaf(db, &payload_ty, seen)
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
        // A NOMINAL newtype / a QTY are TRANSPARENT wrappers over their inner type — a newtype (or
        // quantity) over an orderable payload IS orderable (it is the same value, wrapped for identity /
        // units). Peel to the inner and pass `float_ok` THROUGH (unlike a COMPOUND arm, which resets it to
        // false): a bare-root newtype-over-float enumerates by canonical bytes exactly like a bare float.
        // (#7234 regression fix: the to-list orderability check reached a `Nominal { inner: Int64 }`
        // element — `(type Nat (Mk Int64))` set element — and wrongly declined CDZ0203; it must honor the
        // newtype's orderable payload, the SAME peel `set_shape_descriptor`/the rust backend's
        // `strip_nominal(_and_qty)` already do. A nominal/qty over an un-orderable inner — a Set/Map leaf,
        // a float-leaf compound — still declines via the inner's own arm.)
        Ty::Nominal { inner, .. } => orderable_leaf_or_compound(db, inner, float_ok, seen),
        Ty::Qty { inner, .. } => orderable_leaf_or_compound(db, inner, float_ok, seen),
        // Anything else (a bare var, a function, Any, an unresolved nominal) — not orderable.
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
        Prim::Neg => "neg",
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
        Prim::SetNew => "set-new",
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
        Prim::Gensym => "gensym",
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
        Prim::TypeAst { instantiated } => {
            if instantiated {
                "type-ast"
            } else {
                "type-ast-generic"
            }
        }
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
        match desc_wit("(record (= z 1) (= a true))") {
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
