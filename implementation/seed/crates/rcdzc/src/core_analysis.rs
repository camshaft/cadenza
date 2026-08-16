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
        Core::Call { args, .. } => args,
        Core::Tuple { elems } | Core::ListNew { elems } | Core::BytesOf { elems } => elems.to_vec(),
        Core::ListPush { list, elem } => vec![list, elem],
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
    let n = counts.entry(id).or_insert(0);
    *n += 1;
    if *n == 1 {
        order.push(id);
        for child in licm_children(db, id) {
            collect_node_refs(db, child, counts, order);
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
pub(crate) fn collect_shared_heap_binding_candidates(db: &mut Db, body: StructId) -> Vec<StructId> {
    let mut counts: HashMap<StructId, u32> = HashMap::new();
    let mut order: Vec<StructId> = Vec::new();
    collect_node_refs(db, body, &mut counts, &mut order);
    let mut out: Vec<StructId> = Vec::new();
    for id in order {
        // Shared: reached by ≥2 parent edges (the re-descent source).
        if counts.get(&id).copied().unwrap_or(0) < 2 {
            continue;
        }
        // A LocalRef is already a slot read — binding it would be redundant / cyclic.
        if matches!(core_of(db, id), Core::LocalRef { .. }) {
            continue;
        }
        // Heap handle only: a scalar share is the scalar CSE's job (no dup/drop); a Unit owns no reference.
        let ty = crate::infer::type_of(db, id);
        if !is_heap_type(&ty) || matches!(ty.strip_nominal(), Ty::Unit) {
            continue;
        }
        out.push(id);
    }
    out
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
        // A pure conversion: same op over an equal operand.
        (
            Core::Convert {
                op: ox,
                operand: px,
            },
            Core::Convert {
                op: oy,
                operand: py,
            },
        ) => ox == oy && core_eq(db, px, py),
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
}
