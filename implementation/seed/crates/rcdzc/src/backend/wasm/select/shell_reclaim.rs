//! Sum-continuation shell-reclaim / extraction-consume DECISION predicates, peeled out of select.rs
//! (pure mechanical split, no logic change) to hold the module-root file under the 512 KiB cap.
//! All items are re-exported into the `select` root via `use shell_reclaim::*`; the `use super::*`
//! here inherits select.rs's imports + private helpers (child modules see ancestor-private items).

use super::*;

/// The shared soundness floor for deep-dropping an owned boxed-sum SHELL after a `MatchSum` (extracted
/// from the tail-position and non-tail `MatchSum` reclaim gates so both compute the IDENTICAL predicate).
/// Reclaim iff the match does not diverge, the scrutinee is a REAL boxed sum (not an erased enum-disc),
/// EVERY payload is scalar (the sread-UAF sound floor — a scalar copies out, holding no handle that could
/// alias the shell), the scrutinee was freshly stashed into an i32 slot, and it is a PROVEN-owned
/// temporary. The tail gate additionally ANDs its own `!arms_tail_call` (a member-tail-call arm `br`s to
/// the loop top and never reaches the post-match drop) — that stays at the call site because it needs the
/// `MatchSum` decision tree, and it is NOT the same as a `TailPos::Tail(Some(_))` self-loop test (a
/// `Tail(Some)` match whose arm is a constructor still has a valid reclaim point).
/// (FIND3, v-mem-safety fence — the SCRUTINEE-ESCAPE analog of scalar-extracted-not-escaped) Whether the
/// matched `scrutinee` is DEAD AFTER DESTRUCTURE — the WHOLE scrutinee value (and, when it is a Param/
/// LocalRef, its binder) does NOT appear in ANY arm body EXCEPT as the destructured value (a `SumPayload`/
/// `Proj`/`SumExpect` read OFF it). A DIRECT re-reference — `(f st)`, `(tuple st …)`, `return st` — makes it
/// LIVE AFTER the match, so the shell-deep-drop would free a still-live value → UAF. STRUCTURAL: any
/// non-destructuring reference counts.
///
/// NOTE: RESUME-ESCAPE (v-effects #048389): a handler-arm `(resume -1 st)` re-reference is INVISIBLE here —
/// by select.rs the resume is REDUCED/threaded, so `st` is not a syntactic arm-body ref. TWO guards cover
/// it: (1) the handler THREADED-STATE scrutinee is classified BORROWED, so the caller's `Owned` gate excludes
/// it (rrb1); (2) a `CallClosure`/`HostCall` in the arm (an opaque consumer that could capture the scrutinee
/// invisibly) is conservatively treated as not-dead. A genuinely OWNED-COMPUTED scrutinee (a fresh recursive
/// `Call` result — the FIND3 target) is not a threaded-state and cannot resume-escape. (Tighter pre-reduction
/// signal = v-effects' #4966 collect_tail_resume_values, deferred unless the conservative floor over-excludes.)
pub(crate) fn scrutinee_dead_after_destructure(
    db: &mut Db,
    scrutinee: StructId,
    root: &crate::core::SumCont,
) -> bool {
    let binder = match core_of(db, scrutinee) {
        Core::Param { binder } | Core::LocalRef { binder } => Some(binder),
        _ => None,
    };
    !sum_cont_refs_scrutinee(db, root, scrutinee, binder)
}

pub(crate) fn sum_cont_refs_scrutinee(
    db: &mut Db,
    cont: &crate::core::SumCont,
    scrutinee: StructId,
    binder: Option<StructId>,
) -> bool {
    match cont {
        crate::core::SumCont::Leaf(body) => expr_refs_scrutinee(db, *body, scrutinee, binder),
        crate::core::SumCont::Guarded { cond, body, els } => {
            expr_refs_scrutinee(db, *cond, scrutinee, binder)
                || expr_refs_scrutinee(db, *body, scrutinee, binder)
                || sum_cont_refs_scrutinee(db, els, scrutinee, binder)
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            sum_cont_refs_scrutinee(db, then_, scrutinee, binder)
                || sum_cont_refs_scrutinee(db, els, scrutinee, binder)
        }
        crate::core::SumCont::Switch { arms, .. } => arms
            .iter()
            .any(|a| sum_cont_refs_scrutinee(db, &a.cont, scrutinee, binder)),
    }
}

/// Whether the subtree at `id` references the scrutinee (its node id, or its binder via [`is_ref_to`])
/// OUTSIDE a destructuring read. A `SumPayload`/`Proj`/`SumExpect` reading OFF the scrutinee is the match's
/// own extraction (allowed) — its scrutinee/operand slot is SKIPPED; every OTHER position that references
/// the scrutinee is an escape. A `CallClosure`/`HostCall` is conservatively an escape (opaque capture).
pub(crate) fn expr_refs_scrutinee(
    db: &mut Db,
    id: StructId,
    scrutinee: StructId,
    binder: Option<StructId>,
) -> bool {
    let mut seen = HashSet::new();
    expr_refs_scrutinee_seen(db, id, scrutinee, binder, &mut seen)
}

pub(crate) fn expr_refs_scrutinee_seen(
    db: &mut Db,
    id: StructId,
    scrutinee: StructId,
    binder: Option<StructId>,
    seen: &mut HashSet<StructId>,
) -> bool {
    if !seen.insert(id) {
        return false;
    }
    // CONSERVATIVE OPAQUE-CAPTURE BACKSTOP: a `Core::CallClosure`'s `closure` env is INVISIBLE to this
    // syntactic walk and could capture the scrutinee — a reduced resume-continuation is a `CallClosure` of a
    // reified `k` (the v-effects #048389 escape vector) — so treat it as not-dead-after. A `Core::HostCall`
    // is NOT such a vector (v-effects soundness ruling): it is a TERMINAL host-delegated perform with no
    // closure, no captured env, no continuation (the host resolves the WIT import and returns by value; there
    // is no in-program handler and no `resume`), so its ONLY scrutinee refs are its EXPLICIT args, which the
    // normal walk below descends — and if an arg is itself (or reaches) a `CallClosure`, the recursion hits
    // this arm and stays conservative, so there is no hole. Keep `CallClosure` conservative; let `HostCall`
    // fall through to the arg-descent. (#048389 not weakened — the vector is CallClosure, unchanged.)
    if matches!(core_of(db, id), Core::CallClosure { .. }) {
        return true;
    }
    let is_scrut_ref =
        |db: &mut Db, x: StructId| x == scrutinee || binder.is_some_and(|b| is_ref_to(db, x, b));
    match core_of(db, id) {
        // A destructuring read OFF the scrutinee is the match's own extraction — SKIP its scrutinee/operand
        // slot (allowed), but still descend the OTHER children (an index/path could reference the scrutinee).
        Core::SumPayload { scrutinee: s, .. }
        | Core::SumExpect { scrutinee: s, .. }
        | Core::Proj { operand: s, .. }
            if is_scrut_ref(db, s) =>
        {
            core_child_ids(db, id)
                .into_iter()
                .filter(|&c| c != s)
                .any(|c| expr_refs_scrutinee_seen(db, c, scrutinee, binder, seen))
        }
        _ => {
            if is_scrut_ref(db, id) {
                return true;
            }
            core_child_ids(db, id)
                .into_iter()
                .any(|c| expr_refs_scrutinee_seen(db, c, scrutinee, binder, seen))
        }
    }
}

/// Whether ANY arm body (or guard) in a `MatchSum` decision tree materializes a heap sub-value of the
/// scrutinee OUT as a live handle — the sum analogue of the per-arm `arm_borrows_heap_subvalue` check
/// `list_shell_reclaim_slot` runs over a `MatchList`'s flat arms. Walks the `SumCont`: a `Leaf`/`Guarded`
/// body (and a `Guarded` guard) is an arm body; `Guarded.els`/`LitTest.then_`/`LitTest.els`/`Switch` arms
/// are continuations. If NONE borrows a heap sub-value out (escape-clean), the payload is destructured to
/// scalars — no live handle survives — and the shell deep-drop is safe. Because
/// `collect_consuming_payload_sites` marks a compound-child dup on exactly the SAME consuming-position
/// condition, escape-clean here implies the shell-reclaim dup pass collected NOTHING for this shell, so
/// the deep-drop needs no dup pairing and reclaims the shell + its borrowed-only children by cascade.
pub(crate) fn sum_cont_arm_borrows_heap_subvalue(db: &mut Db, cont: &crate::core::SumCont) -> bool {
    match cont {
        crate::core::SumCont::Leaf(body) => arm_borrows_heap_subvalue(db, *body),
        crate::core::SumCont::Guarded { cond, body, els } => {
            arm_borrows_heap_subvalue(db, *cond)
                || arm_borrows_heap_subvalue(db, *body)
                || sum_cont_arm_borrows_heap_subvalue(db, els)
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            sum_cont_arm_borrows_heap_subvalue(db, then_)
                || sum_cont_arm_borrows_heap_subvalue(db, els)
        }
        crate::core::SumCont::Switch { arms, .. } => arms
            .iter()
            .any(|a| sum_cont_arm_borrows_heap_subvalue(db, &a.cont)),
    }
}

/// Conservative reuse-clean for the compound-shell reclaim: whether ANY arm body CONSTRUCTS a compound
/// value. A constructor is the only node that can FBIP-reuse a payload cell in place — a
/// `(tuple (. t 0) (. t 1))` reuses the projected payload's cell, which the shell deep-drop would then
/// double-free even though escape analysis (which sees only borrowing projections) reports it safe (the
/// FBIP-rebuild-of-projection gap `arm_borrows_heap_subvalue` cannot see). Declining any arm that builds a
/// compound over-approximates (a rebuild-to-fresh arm leaks rather than reclaims — value-correct, never a
/// double-free), which honors the discipline "no reclaim widening without the FBIP-aware check"; the
/// destructure-to-scalar acceptance set constructs nothing and is unaffected.
pub(crate) fn sum_cont_arm_constructs_compound(db: &mut Db, cont: &crate::core::SumCont) -> bool {
    let mut seen = HashSet::new();
    sum_cont_arm_constructs_compound_seen(db, cont, &mut seen)
}

pub(crate) fn sum_cont_arm_constructs_compound_seen(
    db: &mut Db,
    cont: &crate::core::SumCont,
    seen: &mut HashSet<StructId>,
) -> bool {
    match cont {
        crate::core::SumCont::Leaf(body) => expr_constructs_compound_seen(db, *body, seen),
        crate::core::SumCont::Guarded { cond, body, els } => {
            expr_constructs_compound_seen(db, *cond, seen)
                || expr_constructs_compound_seen(db, *body, seen)
                || sum_cont_arm_constructs_compound_seen(db, els, seen)
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            sum_cont_arm_constructs_compound_seen(db, then_, seen)
                || sum_cont_arm_constructs_compound_seen(db, els, seen)
        }
        crate::core::SumCont::Switch { arms, .. } => arms
            .iter()
            .any(|a| sum_cont_arm_constructs_compound_seen(db, &a.cont, seen)),
    }
}

/// Whether EVERY arm RESULT of `cont` is a NON-HEAP (scalar) value — a decidable, conservative sufficient
/// condition that the match cannot carry ANY heap handle (in particular a `String.at` extracted view) OUT
/// as its terminal result. Used to admit the multi-consume `StrAt` view shell-drop WITHOUT the general
/// escape-reachability classifier (`view_escapes_as_arm_result`, v-core-opt's consuming-analysis lane): a
/// scalar match result STRUCTURALLY proves there is no escape-as-result, so the shell deep-drop's cascade
/// frees only the dead final payload ref (the per-consume child-`dup`s the dup pass already emitted balance
/// the consumes 1:1). A HEAP arm result — even a fresh, non-aliasing one — returns false → no reclaim →
/// leak, never a UAF (leak-over-UAF). Checks the arm-body result TYPE per leaf (not a subtree scan): a
/// `Leaf` is its body's type; a `Guarded` needs both its body and the fall-through `els`; `LitTest`/`Switch`
/// recurse into every continuation.
pub(crate) fn sum_cont_result_all_scalar(db: &mut Db, cont: &crate::core::SumCont) -> bool {
    match cont {
        crate::core::SumCont::Leaf(body) => !is_heap_type(&type_of(db, *body)),
        crate::core::SumCont::Guarded { body, els, .. } => {
            !is_heap_type(&type_of(db, *body)) && sum_cont_result_all_scalar(db, els)
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            sum_cont_result_all_scalar(db, then_) && sum_cont_result_all_scalar(db, els)
        }
        crate::core::SumCont::Switch { arms, .. } => {
            arms.iter().all(|a| sum_cont_result_all_scalar(db, &a.cont))
        }
    }
}

/// Whether the multi-consume `String.at` view (the Some-payload of `scrutinee`) can ESCAPE the match as a
/// live heap handle THROUGH some arm's TERMINAL RESULT — returned directly, RETAINED as a heap component of a
/// returned constructor, or carried out by a handle-aliasing reinterpret/normalize
/// (`StrToBytes`/`StrFromBytes`/`NfcNormalize`). This is the general escape-as-result axis (v-core-opt's
/// consuming-analysis lane) that GENERALIZES v-memory-safety's decidable [`sum_cont_result_all_scalar`]
/// subset: a HEAP arm result is still shell-reclaimable when the view provably does NOT flow out as (part of)
/// that result — `(String.concat c c)` CONSUMES the view into a FRESH-allocating builder
/// (`NfcNormalize(BytesConcat …)`) whose output aliases nothing of `c`, so the shell deep-drop frees only the
/// dead final payload ref (the per-consume child-`dup`s the dup pass emitted balance the consumes 1:1) →
/// reclaim to 0. By contrast `#tuple(c …)` RETAINS the view in the returned tuple → escapes → NOT reclaimable
/// (freeing the shell would UAF the escaped `c`). CONSERVATIVE in the SAFE direction (leak-over-UAF): a node
/// whose output could alias/retain the view but which is not PROVEN fresh returns escape; only nodes proven
/// fresh/scalar/borrow (the fresh builders, scalar ops, refs, literals — the `_ => false` floor) are
/// non-escaping. So `!view_escapes_as_arm_result` is TRUE only when the view provably cannot survive as (part
/// of) the result. v-memory-safety co-verifies the heap-result faces to 0 + no double-free (the `#tuple(c …)`
/// escape control must STAY leaking, no trap).
pub(crate) fn view_escapes_as_arm_result(
    db: &mut Db,
    scrutinee: StructId,
    root: &crate::core::SumCont,
) -> bool {
    let mut seen = HashSet::new();
    sum_cont_result_escapes_view(db, root, scrutinee, &mut seen)
}

/// Per-arm terminal-result walk for [`view_escapes_as_arm_result`]: the view escapes iff it escapes through
/// ANY arm's result continuation (Leaf body / Guarded body + fall-through / LitTest + Switch recursions),
/// mirroring [`sum_cont_result_all_scalar`]'s shape.
pub(crate) fn sum_cont_result_escapes_view(
    db: &mut Db,
    cont: &crate::core::SumCont,
    scrut: StructId,
    seen: &mut HashSet<StructId>,
) -> bool {
    match cont {
        crate::core::SumCont::Leaf(body) => expr_escapes_view(db, *body, scrut, seen),
        crate::core::SumCont::Guarded { body, els, .. } => {
            expr_escapes_view(db, *body, scrut, seen)
                || sum_cont_result_escapes_view(db, els, scrut, seen)
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            sum_cont_result_escapes_view(db, then_, scrut, seen)
                || sum_cont_result_escapes_view(db, els, scrut, seen)
        }
        crate::core::SumCont::Switch { arms, .. } => arms
            .iter()
            .any(|a| sum_cont_result_escapes_view(db, &a.cont, scrut, seen)),
    }
}

/// Whether the view (a payload-projection chain rooted at `scrut`) escapes THROUGH the result expression `id`
/// (a tail/result position). See [`view_escapes_as_arm_result`] for the soundness argument. Node-id `seen`
/// dedups the shared-`StructId` DAG re-walk (Core is acyclic; the pure escape value propagates on first visit,
/// so the OR-aggregation is unaffected — same pattern as [`expr_constructs_compound_seen`]).
pub(crate) fn expr_escapes_view(
    db: &mut Db,
    id: StructId,
    scrut: StructId,
    seen: &mut HashSet<StructId>,
) -> bool {
    if !seen.insert(id) {
        return false;
    }
    // The view itself (or a projection/sub-alias of it) RETURNED as the result = escape. A SCALAR leaf
    // (`get_op` Some — an unboxed byte/char COPIED out, no heap handle survives) does not escape.
    if payload_proj_chain_roots_at_node(db, id, scrut) {
        return !matches!(get_op(db, id), Ok(Some(_)));
    }
    match core_of(db, id) {
        // RETAINING constructors: each stores its operand refs INTO the returned value → escape iff any
        // operand (recursively) carries the view out.
        Core::Tuple { elems } | Core::ListNew { elems } | Core::SetOf { elems, .. } => {
            elems.iter().any(|&e| expr_escapes_view(db, e, scrut, seen))
        }
        Core::SumNew { payloads, .. } => payloads
            .iter()
            .any(|&e| expr_escapes_view(db, e, scrut, seen)),
        Core::Record { fields } => fields
            .values()
            .copied()
            .collect::<Vec<_>>()
            .into_iter()
            .any(|e| expr_escapes_view(db, e, scrut, seen)),
        Core::MapNew { entries, .. } => entries.iter().any(|&(k, v)| {
            expr_escapes_view(db, k, scrut, seen) || expr_escapes_view(db, v, scrut, seen)
        }),
        // HANDLE-ALIASING reinterpret/normalize: MAY return the SAME heap handle as their operand
        // (`NfcNormalize` is a no-op for already-NFC text; `StrToBytes`/`StrFromBytes` reinterpret the same
        // byte leaf) → transparent (recurse the operand). For the fresh-builder WIN `NfcNormalize(BytesConcat
        // c c)` the operand is a FRESH BytesConcat → recursion yields false (reclaim); `NfcNormalize(c)` /
        // `StrToBytes(c)` of the RAW view escapes.
        Core::NfcNormalize { string } | Core::StrToBytes { string } => {
            expr_escapes_view(db, string, scrut, seen)
        }
        Core::StrFromBytes { bytes, .. } => expr_escapes_view(db, bytes, scrut, seen),
        // TRANSPARENT control flow: the arm result is whichever tail is taken → recurse each tail.
        Core::Let { body, .. } => expr_escapes_view(db, body, scrut, seen),
        Core::If { then_, else_, .. } => {
            expr_escapes_view(db, then_, scrut, seen) || expr_escapes_view(db, else_, scrut, seen)
        }
        Core::MatchSum { root, .. } => sum_cont_result_escapes_view(db, &root, scrut, seen),
        // OPAQUE calls: the callee may RETURN or capture an argument, so a view flowing IN as an arg (or the
        // closure env) may flow OUT as the result → conservative escape if any operand carries the view out. A
        // fresh builder (BytesConcat/StrSlice/…) is a DEDICATED Core node (below), NOT a Call, so this does not
        // over-decline the WIN.
        Core::Call { args, .. } => args.iter().any(|&a| expr_escapes_view(db, a, scrut, seen)),
        Core::CallClosure { closure, args } => {
            expr_escapes_view(db, closure, scrut, seen)
                || args.iter().any(|&a| expr_escapes_view(db, a, scrut, seen))
        }
        // FRESH-allocating builders (BytesConcat/ListConcat/StrSlice/BytesSlice/BytesOf/BytesCompact/…), SCALAR
        // ops (BytesLen/StrScalarLen/arithmetic/cmp/ValueEq), refs and literals: the output aliases nothing of
        // the view (an operand view is byte-copied/absorbed/borrowed, never retained), so the view does not
        // escape via them. This `_ => false` floor is the leak-over-UAF boundary — a genuinely-unknown RETAINER
        // must be added to the constructor/aliasing arms above (else it would wrongly reclaim → the escape
        // control catches that as a debug-counters trap in v-mem's co-verify).
        _ => false,
    }
}

/// Whether the expression subtree `id` contains a compound CONSTRUCTOR node (see
/// [`sum_cont_arm_constructs_compound`]). Node-id `seen` set guards the shared-`StructId` DAG re-walk.
pub(crate) fn expr_constructs_compound_seen(
    db: &mut Db,
    id: StructId,
    seen: &mut HashSet<StructId>,
) -> bool {
    if !seen.insert(id) {
        return false;
    }
    if matches!(
        core_of(db, id),
        Core::Tuple { .. }
            | Core::SumNew { .. }
            | Core::ListNew { .. }
            | Core::MapNew { .. }
            | Core::SetOf { .. }
            | Core::Record { .. }
            | Core::BytesOf { .. }
            | Core::BinBuild { .. }
            | Core::BinBitsBuild { .. }
    ) {
        return true;
    }
    // A borrowing READ (Proj/SumPayload/SumExpect/*Len) reads its AGGREGATE operand in place without
    // transferring it — do NOT descend into that operand. It is the scrutinee/aggregate, and a constructor
    // in the aggregate's OWN definition (e.g. an inline `(if (Some (list …)) None)` scrutinee) is not an
    // arm reuse of the shell payload; descending it false-flagged every inline-constructed-scrutinee match
    // (d4/dm1/… declined despite pure-scalar arm bodies). A genuine FBIP-rebuild `(tuple (. t 0) (. t 1))`
    // is still caught: the `Tuple` is a top-level arm node, flagged above before its `Proj` operands here.
    match core_of(db, id) {
        Core::Proj { .. }
        | Core::SumPayload { .. }
        | Core::SumExpect { .. }
        | Core::ListLen { .. }
        | Core::BytesLen { .. }
        | Core::StrScalarLen { .. } => false,
        _ => core_child_ids(db, id)
            .into_iter()
            .any(|c| expr_constructs_compound_seen(db, c, seen)),
    }
}

/// Whether `id` is a FALLIBLE-READ extraction op (`List.at`/`Bytes.at`/`Map.lookup`/`Bytes.slice`/
/// `String.at`/`String.slice`) — each returns a runtime `Option` and `dup`-RETAINS the extracted element
/// into the `Some` (see `heap_operand_ownership`), holding the payload at rc >= 2 through the arm. inc2b
/// keys on this: the rc >= 2 makes an in-place FBIP reuse of the payload PATH-COPY (never alias), and the
/// Stage-B consume reclaim balances the extra retained ref against the shell deep-drop.
pub(crate) fn scrutinee_is_fallible_extraction(db: &mut Db, id: StructId) -> bool {
    matches!(
        core_of(db, id),
        Core::ListAt { .. }
            | Core::BytesAt { .. }
            | Core::StrAt { .. }
            | Core::StrSlice { .. }
            | Core::BytesSlice { .. }
            | Core::MapLookup { .. }
    )
}

/// The CONTAINER operand a fallible interior-view op READS (borrows) — the list/bytes/string/map whose
/// element/slice/value the view returns. `None` for a non-view node. Used by `param_only_borrowed_or_backedge`
/// to recognize `(List.at xs i)` etc. as a BORROW of the container (the it4 loop-frame invariant-param
/// reclaim): the container is read, not consumed; the index/bounds/key are scalars carrying no binder.
pub(crate) fn fallible_view_container_of(db: &mut Db, id: StructId) -> Option<StructId> {
    match core_of(db, id) {
        Core::ListAt { list, .. } => Some(list),
        Core::BytesAt { bytes, .. } => Some(bytes),
        Core::StrAt { string, .. } => Some(string),
        Core::StrSlice { string, .. } => Some(string),
        Core::BytesSlice { bytes, .. } => Some(bytes),
        Core::MapLookup { map, .. } => Some(map),
        _ => None,
    }
}

/// The allowlist of PURE PERSISTENT BUILDER ops for the inc2b Stage-B extraction-consume reclaim: each
/// takes EXACTLY ONE owned reference to its input structure(s) and returns one fresh result that
/// references the input once (structural-sharing `dup`s the shared nodes once). So when an extraction
/// `Some`'s payload is CONSUMED by one of these, a single `dup`-on-escape (the site
/// `collect_shell_reclaim_child_dups` marks) balances the shell deep-drop 1:1. An OPAQUE consumer
/// (`Call`/`CallClosure`/`HostCall` — which includes a REDUCED `resume`-thread, `resume` being invisible
/// in Core, and any multi-use/capturing consumer) is NOT provably single-reference at select.rs, so a
/// payload consumed there DECLINES (leak beats UAF). Co-verified with v-runtime (runtime domain).
pub(crate) fn is_allowlisted_builder(db: &mut Db, id: StructId) -> bool {
    matches!(
        core_of(db, id),
        Core::ListPush { .. }
            | Core::ListPrepend { .. }
            | Core::ListConcat { .. }
            | Core::ListUpdate { .. }
            | Core::BytesConcat { .. }
            | Core::MapInsert { .. }
            | Core::SetInsert { .. }
            | Core::SetAlgebra { .. }
            // Single-owned-ref-move CONVERTERS (not builders, but the SAME 1:1-balance property the
            // allowlist requires): `Core::StrToBytes` (op_bytes_compact — `Symbol.to-string` / `Bytes.compact`
            // over a String) CONSUMES exactly one owned ref to its operand and returns one flat leaf,
            // retaining NO alias to the input (it does not resume-thread — it is a prim, not a Call/Closure).
            // So when an extraction-`Some` payload is consumed here, the single dup-on-escape balances the
            // shell deep-drop 1:1, exactly like a builder child. Fixes the `(match (Map.lookup m k) ((Some sy)
            // (String.byte-len (Symbol.to-string sy))) …)` symbol-value round-trip leak (17-symbols:583): the
            // Owned Map.lookup Some shell was left unreclaimed because `Symbol.to-string`→`StrToBytes` was
            // neither borrow-clean (it consumes) nor an allowlisted builder.
            | Core::StrToBytes { .. }
            // RETAINING interior-VIEW producers (v-memory-safety, the 10-bytes view-of-view family): a
            // slice-of-a-slice `(Bytes.slice outer …)` / `(String.slice outer …)` over a scrutinee-payload
            // view CONSUMES exactly one owned ref to its operand (`op_bytes_slice`/`op_str_slice` `op_drop`
            // the operand) and returns one fresh view that references the parent/grandparent's storage EXACTLY
            // ONCE (a runtime `op_dup` of the retained parent — the retained-storage discipline; String.slice
            // compacts to an independent leaf, referencing the input zero times, which is strictly safer).
            // These are PRIMS (never a Call/Closure → cannot resume-thread), so the SAME single-owned-ref-move
            // 1:1-balance property the allowlist requires holds: the one dup-on-escape
            // (`collect_shell_reclaim_child_dups`'s `owned_compound_boxed` arm, which fires because a fresh
            // `Bytes.slice`/`String.slice` scrutinee IS `Owned`) balances the shell deep-drop 1:1. Fixes the
            // 10-bytes view-of-view leak (0022 etc.): the outer `Bytes.slice` `Some` shell was left unreclaimed
            // because the inner `Bytes.slice` that consumes `outer` was neither borrow-clean nor an allowlisted
            // builder → the child-dup fired (owned_compound_boxed) with NO matching shell-drop = a leak. A
            // NON-retaining raw-alias extraction (`Map.lookup`/`List.at`) is deliberately EXCLUDED (its result
            // aliases the source WITHOUT a retaining `op_dup`, so it is not a clean single-owned-ref move —
            // leak beats UAF); an ESCAPING view (returned as the arm result) is not a builder child, so the
            // subset check fails and the shell stays leaking (the escape control).
            | Core::BytesSlice { .. }
            | Core::StrSlice { .. }
            // Single-owned-ref-move CONVERTER (v-memory-safety): `Core::StrFromBytes` (`str-from-bytes`) is the
            // fallible twin of `StrToBytes` — it CONSUMES exactly one owned ref to its bytes operand
            // (transferring the storage out as the `String` on success, `op_drop`ping it on the ill-formed-UTF-8
            // failure path) and is a PRIM (never a Call/Closure → cannot resume-thread). So a scrutinee-payload
            // view CONSUMED by `String.from-bytes` is a clean single-owned-ref move: the one dup-on-escape
            // balances the extraction shell's deep-drop 1:1, exactly like `StrToBytes`/`Bytes.concat`. Fixes the
            // `(match (Bytes.slice …) ((Some s) (String.byte-len (String.from-bytes s)))) …)` decode-window leak
            // (10-bytes:919): the outer Bytes.slice Some shell was left unreclaimed because `String.from-bytes`
            // consuming `s` was neither borrow-clean nor an allowlisted builder.
            | Core::StrFromBytes { .. }
            // Single-owned-ref-move CONVERTER (v-memory-safety): `Core::NfcNormalize` (`str-nfc-normalize`)
            // CONSUMES exactly one owned ref to its String operand and returns exactly one — the SAME handle
            // when the input is already NFC (the common ASCII case, an identity passthrough), else a fresh
            // canonical leaf with the original `op_drop`ped. Either way it retains NO alias to the input and
            // is a PRIM (never a Call/Closure → cannot resume-thread), so a scrutinee-payload view CONSUMED by
            // it is a clean single-owned-ref move: the one dup-on-escape balances the extraction shell's
            // deep-drop 1:1, exactly like `StrToBytes`. `Symbol.of` on a runtime String lowers to
            // `StrToBytes(NfcNormalize(operand))` (lower/compute.rs), so the payload is consumed by
            // `NfcNormalize` DIRECTLY (StrToBytes only sees its result) — without this arm the outer slice
            // `Some` shell was left unreclaimed (`(match (String.slice …) ((Some s) (Symbol.of s)) …)`
            // intern-a-transient-window leak, 13-strings:2456), even though the allowlisted `StrToBytes`
            // downstream would have reclaimed it had `s` reached it unwrapped.
            | Core::NfcNormalize { .. }
    )
}

/// Collect the DIRECT child node-ids of every allowlisted-builder node reachable in the arm continuation.
/// A consuming scrutinee-payload site that is one of these children is consumed DIRECTLY by a builder
/// (`mlr2`: `inner` is the `lhs` of `List.concat`); a site NOT in this set is consumed by an opaque node
/// (a `Call`/`CallClosure`, or reached only through one) and must decline.
pub(crate) fn collect_allowlisted_builder_children_cont(
    db: &mut Db,
    cont: &crate::core::SumCont,
    seen: &mut HashSet<StructId>,
    out: &mut HashSet<StructId>,
) {
    match cont {
        crate::core::SumCont::Leaf(body) => {
            collect_allowlisted_builder_children_expr(db, *body, seen, out)
        }
        crate::core::SumCont::Guarded { cond, body, els } => {
            collect_allowlisted_builder_children_expr(db, *cond, seen, out);
            collect_allowlisted_builder_children_expr(db, *body, seen, out);
            collect_allowlisted_builder_children_cont(db, els, seen, out);
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            collect_allowlisted_builder_children_cont(db, then_, seen, out);
            collect_allowlisted_builder_children_cont(db, els, seen, out);
        }
        crate::core::SumCont::Switch { arms, .. } => {
            for a in arms {
                collect_allowlisted_builder_children_cont(db, &a.cont, seen, out);
            }
        }
    }
}

pub(crate) fn collect_allowlisted_builder_children_expr(
    db: &mut Db,
    id: StructId,
    seen: &mut HashSet<StructId>,
    out: &mut HashSet<StructId>,
) {
    if !seen.insert(id) {
        return;
    }
    let is_builder = is_allowlisted_builder(db, id);
    // FRESH element-retaining collection constructors are the const-folded twins of the allowlisted
    // collection builders (`Map.insert`/`List.push`-into-empty fold to `Core::MapNew`/`Core::ListNew` in
    // `lower`), so a payload CONSUMED as one of their VALUE positions is a single-owned-ref move into the
    // fresh collection — the same 1:1 balance the builder allowlist requires. Mark only VALUE positions:
    // `ListNew` elements and `MapNew` entry VALUES. A `MapNew` KEY (and a `Set.of` element, which IS a CHAMP
    // key) is NEVER absorbed here — the map/set-key-ownership line neither v-core-opt nor v-memory-safety
    // crosses; a key-position payload stays a consuming site NOT in this set, so the subset check fails and
    // the shell declines (leak beats a key double-free). Fixes the general Map.insert extraction-shell leak
    // (05-compound:2219): the fold to `MapNew` left the payload-as-value not a builder child, so the subset
    // check failed and the shell leaked. Imprecision only OVER-DECLINES (a value missing → subset fails).
    match core_of(db, id) {
        Core::ListNew { elems } => {
            for &e in elems.iter() {
                out.insert(e);
            }
        }
        Core::MapNew { entries, .. } => {
            for &(_key, val) in entries.iter() {
                out.insert(val);
            }
        }
        _ => {}
    }
    for c in core_child_ids(db, id) {
        if is_builder {
            out.insert(c);
        }
        collect_allowlisted_builder_children_expr(db, c, seen, out);
    }
}

/// inc2b Stage B — reclaim an extraction-`Some` (`List.at`/`Map.lookup`/`Bytes.slice`, which `dup`-retains
/// its payload) whose payload ESCAPES only by being CONSUMED by a pure persistent builder from the
/// allowlist ([`is_allowlisted_builder`]). Fires iff the scrutinee is a fallible extraction AND there is
/// at least one CONSUMING scrutinee-payload site AND every such site (the set
/// `collect_shell_reclaim_child_dups` will `dup`) is a DIRECT child of an allowlisted builder — so each
/// `dup` is balanced 1:1 by the builder's single-owned-ref move + the shell deep-drop. Borrow-only arms go
/// to Stage A; a payload consumed by an opaque `Call`/`CallClosure` (or reduced `resume`-thread) is not a
/// builder child → subset fails → shell declines (leak>UAF). Reuse-clean unneeded (extraction holds rc>=2 →
/// FBIP path-copies). Imprecision only OVER-DECLINES, never over-reclaims.
pub(crate) fn sum_cont_extraction_consume_allowlisted(
    db: &mut Db,
    root: &crate::core::SumCont,
    scrutinee: StructId,
) -> bool {
    if !scrutinee_is_fallible_extraction(db, scrutinee) {
        return false;
    }
    let mut consuming = HashSet::new();
    collect_consuming_payload_sites_cont(db, root, scrutinee, &mut consuming);
    if consuming.is_empty() {
        return false;
    }
    let mut seen = HashSet::new();
    let mut builder_children = HashSet::new();
    collect_allowlisted_builder_children_cont(db, root, &mut seen, &mut builder_children);
    consuming.iter().all(|s| builder_children.contains(s))
}

/// Whether `id` is FRESHLY PRODUCED here — so it can't be a pre-existing binding a `resume` continuation
/// re-reads (the resume-escape the `Core::Call` gate guarded; "inlined once, never resume-threaded" holds for
/// any fresh producer). Fresh: `Core::Call`; `Core::SumNew` iff each HEAP payload is itself fresh (recurse;
/// scalar/unit OK) — rejects `(Some <shared/borrowed local>)` whose shell-drop would free a still-held value
/// (the `neg_shared` UAF control), structural not fold-reliant; `Core::If` iff BOTH branches are (the INLINED
/// `mk` producer the bare-`Call` gate missed, 05:2163/2214); `Core::Let` iff body is. Others excluded (may
/// carry a threaded/shared binding; leak>UAF). Bounded walk.
pub(crate) fn is_fresh_owned_sum_producer(db: &mut Db, id: StructId) -> bool {
    match core_of(db, id) {
        Core::Call { .. } => true,
        // Reclaim-safe only if each heap payload is itself fresh (scalar/unit OK); a bare shared/borrowed
        // `LocalRef`/`Param` payload would be freed out from under the surrounding scope.
        Core::SumNew { payloads, .. } => payloads
            .iter()
            .all(|&p| !is_heap_type(&type_of(db, p)) || is_fresh_owned_sum_producer(db, p)),
        Core::If { then_, else_, .. } => {
            is_fresh_owned_sum_producer(db, then_) && is_fresh_owned_sum_producer(db, else_)
        }
        Core::Let { body, .. } => is_fresh_owned_sum_producer(db, body),
        _ => false,
    }
}

/// COMPUTED-`Some` companion of [`sum_cont_extraction_consume_allowlisted`] (05:#9134). A computed owned `Some`
/// (fresh `Core::Call`, or the INLINED `Core::If` of two `SumNew`s) moves its payload in at rc1, but
/// `owned_compound_boxed` dups each consuming site (keyed on scrutinee `Owned`, not `Core::Call`) → rc>=2, so
/// the 1:1 balance holds. GATED [`is_fresh_owned_sum_producer`] + DEAD-AFTER-DESTRUCTURE +
/// `!view_escapes_as_arm_result`. Widening the bare-`Call` drop gate to the fresh-producer set COMPLETES the
/// lockstep the dup side already ran (Owned-keyed) — no unbalanced drop added. Imprecision only OVER-DECLINES.
pub(crate) fn sum_cont_owned_call_consume_allowlisted(
    db: &mut Db,
    root: &crate::core::SumCont,
    scrutinee: StructId,
) -> bool {
    if !is_fresh_owned_sum_producer(db, scrutinee) {
        return false;
    }
    if !scrutinee_dead_after_destructure(db, scrutinee, root) {
        return false;
    }
    // ESCAPE AXIS (defense-in-depth): payload must not survive as an arm's terminal RESULT (a builder result
    // returned whole would carry it out, racing the deep-drop cascade). Stricter-only (OVER-DECLINE).
    if view_escapes_as_arm_result(db, scrutinee, root) {
        return false;
    }
    let mut consuming = HashSet::new();
    collect_consuming_payload_sites_cont(db, root, scrutinee, &mut consuming);
    if consuming.is_empty() {
        return false;
    }
    let mut seen = HashSet::new();
    let mut builder_children = HashSet::new();
    collect_allowlisted_builder_children_cont(db, root, &mut seen, &mut builder_children);
    consuming.iter().all(|s| builder_children.contains(s))
}

pub(crate) fn sum_shell_reclaim_ok(
    db: &mut Db,
    scrutinee: StructId,
    scrut_ty: &Ty,
    stashed_slot: Option<(u32, ValType)>,
    never_diverges: bool,
    root: &crate::core::SumCont,
) -> bool {
    // The STASHED-owned path: the payload-safety + rematch gates PLUS a freshly-stashed I32 slot holding an
    // OWNED scrutinee (a computed/materialized temporary). A reused PARAM/local slot fails the Owned gate
    // (heap_operand_ownership(Param)==Borrowed) — that case is the non-tail-spine param path, gated separately
    // via `sum_shell_reclaim_payload_ok` + the proven-owned-dead-after `nontail_match_reclaim_binders` set.
    matches!(stashed_slot, Some((_, ValType::I32)))
        && matches!(
            heap_operand_ownership(db, scrutinee),
            Ok(HandleOwnership::Owned)
        )
        && (sum_shell_reclaim_payload_ok(db, scrutinee, scrut_ty, never_diverges, root)
            // STASHED-OWNED-COMPUTED COMPOUND increment (v-core-opt + v-mem-safety co-design). The all-scalar
            // floor in `sum_shell_reclaim_payload_ok` leaves a COMPOUND-payload owned COMPUTED scrutinee's shell
            // un-dropped, yet the dup-pass ALREADY dups its consumed children (`owned_compound_boxed` arm of
            // `collect_shell_reclaim_child_dups` fires for computed + `Owned` + compound-boxed sum) — so the
            // deep-drop is the MISSING half of that lockstep (orphaned dup = leak). Completed under the compound
            // PARAM path's `nontail_param_compound_extra_ok` fences (G4 no arm returns the shell whole / no
            // payload-in-result; G5 no arm alias-outs a shell child via a fallible interior-view op — the
            // sread-UAF fence). `dup ⊇ drop` so the cascade nets, never a double-free (residual over-dup = leak).
            // RESUME-ESCAPE GUARD: restrict to a fresh `Core::Call` result DEAD-AFTER-DESTRUCTURE — inlined once,
            // CANNOT be resume-threaded (a handler threading state via If/materialize/Param could `resume`-escape
            // a payload invisibly → a husk-drop would free a live escapee). `Core::AstDecode` (op 94) and
            // `Core::StrFromBytes` (op 96) JOIN `Core::Call` here: both are PURE prims (never handlers) minting a
            // FRESH owned shell inlined once, so strictly at least as dead-after-safe as a Call; the fences hold
            // identically and the dup pass dups any consumed payload child (12-metaprogramming:0072 Ast round-
            // trip; 10-bytes:919 str-from-bytes `Some String`). Without them those husks + payloads leaked.
            || (matches!(
                core_of(db, scrutinee),
                Core::Call { .. } | Core::AstDecode { .. } | Core::StrFromBytes { .. }
            )
                && scrutinee_dead_after_destructure(db, scrutinee, root)
                && nontail_param_compound_extra_ok(db, scrutinee, scrut_ty, never_diverges, root)))
}

/// The owned-single-view-producer twin of [`sum_shell_reclaim_ok`] for `MatchSum` (the `SumExpect`
/// `sumexpect_shell_reclaim` analogue): a `String.at`/`Bytes.slice` scrutinee returns a fresh `Some(one
/// view)` — owned + single-heap-payload BY CONSTRUCTION ([`is_owned_single_view_producer`]) — but is
/// deliberately NOT globally `Owned` (`heap_operand_ownership` — the Stage-B `String.concat` note at
/// select.rs's StrAt comment), so `sum_shell_reclaim_ok`'s `Owned` gate MISSES it and its `Some` shell
/// LEAKS one cell per match (the corpus-06 codec `find-at`/`fromcol` per-`String.at`-iteration leak). Treat
/// it as owned LOCALLY here (the local>global discipline the SumExpect reclaim already uses).
///
/// CRITICAL — the STRICT BORROW-CLEAN floor, NOT the full [`sum_shell_reclaim_payload_ok`]. This view path
/// emits NO compensating child-`dup` (`collect_shell_reclaim_child_dups` keys on a globally-`Owned` scrutinee,
/// which a StrAt is NOT), so it is sound ONLY when the view is purely BORROWED — never consumed/escaped and
/// never FBIP-rebuilt. `sum_shell_reclaim_payload_ok`'s consume-into-builder (disjunct 4,
/// `sum_cont_extraction_consume_allowlisted`) + extraction-borrowed (disjunct 5) branches ASSUME that
/// child-dup, so admitting them here DOUBLE-FREES a consumed view (`rev-go`'s `(String.concat acc c)` — c
/// consumed by the allowlisted `String.concat` AND freed again by the shell-drop cascade → an rc-underflow
/// trap; caught by the corpus enum). So gate on the borrow-clean disjunct-(3) conditions DIRECTLY:
/// `!arm_borrows_heap_subvalue` (the view is only READ — `value-eq`/`Bytes.at`/probe, per the relax set — never
/// materialized out in a consuming position) AND `!arm_constructs_compound` (no rebuild reusing the view cell),
/// plus the shared non-diverging / heap / non-enum / not-re-matched (Class-B) safety gates. `find-at`,
/// `balanced-paren`, the multibyte-rope `(match (String.at …) ((Some c) (if (= c …) …)))` cases qualify (value-eq
/// borrow); `rev-go`/`fromcol` (consume the view) are correctly EXCLUDED (leak beats a double-free). Stage-B /
/// value-eq StrAt consumers are UNCHANGED — a `MatchSum`-emit-LOCAL override, no global reclassification.
pub(crate) fn matchsum_view_shell_reclaim_ok(
    db: &mut Db,
    scrutinee: StructId,
    scrut_ty: &Ty,
    stashed_slot: Option<(u32, ValType)>,
    never_diverges: bool,
    root: &crate::core::SumCont,
    // The TOP function body — threaded (as `Some`) so the multi-consume disjunct below can key on the SAME
    // `strat_view_multi_consume` predicate the dup pass uses (which counts consume refs over the whole body),
    // keeping the shell-drop in EXACT lockstep with the child-`dup`s. Both callers pass the fn body (the
    // import companion `body_reclaims_view_shell` and the emit via `out.fn_body`), so import ⟺ emit for the
    // added `drop`. `None` (no fn-body context) skips the disjunct — the borrow-only path is unaffected.
    top_body: Option<StructId>,
) -> bool {
    if !is_owned_single_view_producer(db, scrutinee)
        || !matches!(stashed_slot, Some((_, ValType::I32)))
        || never_diverges
        || !is_heap_type(scrut_ty)
        || ty_is_enum_disc(db, scrut_ty)
        // Class-B: a scrutinee re-matched by a nested MatchSum is reclaimed by the inner drop already.
        || cont_rematches_scrutinee(db, scrutinee, root)
    {
        return false;
    }
    // The PRECISE soundness condition for this NO-CHILD-DUP path: the view must have ZERO CONSUMING sites —
    // it is only BORROWED (a `value-eq`/probe read), never moved into a builder/Call NOR returned as the
    // arm result. A consuming site would transfer ownership of the view to that consumer, so the shell-drop
    // cascade freeing the same view = a DOUBLE-FREE (the owned-scrutinee path compensates with a child-`dup`
    // that this local view path does NOT emit — `rev-go`'s `(String.concat acc c)` consume, `fromcol`'s
    // `find-at … c` consume, and an escape-as-result all register a consuming site → excluded, leak beats
    // UAF). `find-at`/`balanced-paren`/the multibyte-rope cases consume nothing (value-eq only) → empty set →
    // reclaimed. Uses the SAME consume/borrow classifier as the owned-scrutinee dup-site collection.
    let mut consuming = HashSet::new();
    collect_consuming_payload_sites_cont(db, root, scrutinee, &mut consuming);
    if consuming.is_empty() {
        return true;
    }
    // MULTI-CONSUME StrAt view + SCALAR match result (v-memory-safety solo subset of the muv shell-reclaim
    // co-design; the general escape-as-result gate `view_escapes_as_arm_result` is v-core-opt's
    // consuming-analysis lane, needed only for HEAP arm results). `String.at` is the ONE view producer not
    // globally `Owned`, so a view CONSUMED more than once got child-`dup`s from the dup pass
    // (`strat_view_multi_consume` — the SAME predicate, single source of truth) but NO shell-drop, leaving
    // the shell + one payload ref leaked (the 2-husk residue). When the match RESULT is a non-heap scalar,
    // the view provably does not escape as the arm terminal, so freeing the shell (its deep-drop cascades
    // ONE decrement into the payload, which the child-`dup`s left at the shell's own rc1) reclaims BOTH
    // cells with no double-free. SINGLE-consume is excluded by `strat_view_multi_consume`'s `> 1` gate (its
    // lone consume already frees the payload — a shell cascade there would double-free), matching the dup
    // pass exactly. Requires the fn-body context (`top_body`) to compute the consume count identically to
    // the dup side; without it we conservatively decline (leak). HEAP-result arms → `sum_cont_result_all_
    // scalar` false → decline (leak beats UAF; v-core-opt's classifier handles those).
    let compound_boxed = is_heap_type(scrut_ty)
        && !ty_is_enum_disc(db, scrut_ty)
        && !sum_has_only_scalar_payloads(db, scrut_ty);
    // Reclaim the StrAt `Some` shell whenever the view is consumed (≥1) AND the arm result does NOT carry the
    // view out (all-scalar result, OR the escape classifier proves non-escape). This SUBSUMES the earlier
    // muv-only `> 1` gate: `strat_view_consume_nonescaping` is `≥1 && (scalar || !view_escapes)`, which the
    // `> 1 && (scalar || !escapes)` muv condition implies, and it ALSO covers SINGLE-consume (the lone
    // consume + a child-`dup` from the dup pass balances the shell deep-drop 1:1). The escaping case (view
    // returned / stored into a returned collection) declines here — its multi-consume child-`dup`s (needed
    // for the multi-consume double-free) are still emitted UNCONDITIONALLY by the dup pass's bare
    // `strat_view_multi_consume`, leaving the shell + one ref as a residual leak (leak beats UAF). Dup-side
    // and gate share this ONE predicate → exact lockstep.
    top_body
        .is_some_and(|tb| strat_view_consume_nonescaping(db, tb, root, scrutinee, compound_boxed))
}

/// The PROJECTION-of-a-fresh-owned-aggregate twin of [`matchsum_view_shell_reclaim_ok`]: a `MatchSum`
/// scrutinee `(. <fresh-owned-aggregate> i)` (`Core::Proj`) extracting a HEAP-SUM field out of a fresh
/// OWNED product (a `#tuple`/`#record`/recursive-`Call` result — `heap_operand_ownership(operand) ==
/// Owned`). The projected `Some` shell is owned LOCALLY (like the view twin's local>global discipline) but
/// `Core::Proj` is deliberately NOT in `heap_operand_ownership` (it stays Borrowed to avoid perturbing the
/// Stage-B product path), so `sum_shell_reclaim_ok`'s global-`Owned` gate MISSES it and the extracted
/// `Some` shell LEAKS one cell per match (02-binding-and-control:7314 — `(match (. (mk …) 0) ((Some v) …))`,
/// the fresh-tuple projection; siblings 6042/6085).
///
/// SOUND by the SAME fence the view twin's borrow-clean branch relies on. A STASHED MatchSum scrutinee's
/// shell is held in its I32 slot and LEAKS unless a reclaim disjunct fires — so ADDING this disjunct drops
/// exactly the shell that currently has NO drop (rc1 → 0, balanced; never a double-free of an
/// already-balanced shell). This is a NO-CHILD-DUP path (`collect_shell_reclaim_child_dups` keys on a
/// globally-`Owned` scrutinee, which a `Core::Proj` is NOT), so — exactly as the view twin — it is sound
/// ONLY under the STRICT BORROW-CLEAN floor: the projected sum must have ZERO consuming sites (payload only
/// READ as a scalar/probe, never moved into a builder/Call NOR escaped as an arm result). A consuming site
/// would transfer ownership of the payload, so the husk deep-drop's cascade freeing that same payload =
/// a DOUBLE-FREE (leak beats UAF → excluded). 02:7314's `((Some v) (+ v …))` reads only the scalar `v`
/// (the `.1` projection is a SEPARATE fresh `mk` call, not this scrutinee) → empty set → reclaimed. The
/// fresh-owned aggregate is deep-dropped after the projection, which dup-retains the extracted child across
/// that drop (Perceus), leaving the shell at exactly the rc1 this drop balances. An aggregate that is NOT
/// globally `Owned` (a borrowed binder/param that could alias a still-live product) fails the `Owned` gate
/// → declined (leak-safe).
pub(crate) fn matchsum_proj_owned_aggregate_reclaim_ok(
    db: &mut Db,
    scrutinee: StructId,
    scrut_ty: &Ty,
    stashed_slot: Option<(u32, ValType)>,
    never_diverges: bool,
    root: &crate::core::SumCont,
) -> bool {
    // Shared safety floor (identical to the view twin): freshly-stashed I32 slot, diverging-clean, heap
    // non-enum sum, and not re-matched by a nested MatchSum (Class-B — reclaimed by the inner drop already).
    if !matches!(stashed_slot, Some((_, ValType::I32)))
        || never_diverges
        || !is_heap_type(scrut_ty)
        || ty_is_enum_disc(db, scrut_ty)
        || cont_rematches_scrutinee(db, scrutinee, root)
    {
        return false;
    }
    // The scrutinee must be a projection OUT OF a fresh globally-`Owned` aggregate. The `Owned` gate on the
    // OPERAND (not the Proj node) is the load-bearing fence: a fresh owned product is consumed by this
    // projection and deep-dropped after, which forces the projection to dup-retain the extracted child.
    let Core::Proj { operand, .. } = core_of(db, scrutinee) else {
        return false;
    };
    if !matches!(
        heap_operand_ownership(db, operand),
        Ok(HandleOwnership::Owned)
    ) {
        return false;
    }
    // STRICT BORROW-CLEAN floor (NO child-dup on this local path): the projected sum must be purely
    // BORROWED — zero consuming sites. Uses the SAME consume/borrow classifier as the owned-scrutinee dup
    // collection and the view twin.
    let mut consuming = HashSet::new();
    collect_consuming_payload_sites_cont(db, root, scrutinee, &mut consuming);
    consuming.is_empty()
}

/// The `Option.expect`-result twin of [`matchsum_proj_owned_aggregate_reclaim_ok`]: a `MatchSum` scrutinee
/// that is a `Core::SumExpect` (`(Option.expect <owned-Some> …)`) extracting a HEAP-SUM payload out of an
/// OWNED source Option. `SumExpect` is deliberately NOT in `heap_operand_ownership` (like `Core::Proj` /
/// `StrAt` — it stays Borrowed to avoid perturbing the value-eq / Stage-B extraction consumers), so
/// `sum_shell_reclaim_ok`'s global-`Owned` gate MISSES it and the extracted payload shell LEAKS one cell per
/// match (05-compound-types:2117 — `(nc (Option.expect (List.at … 0) "at"))` inlines to `(match
/// (Option.expect …) ((Ast.Int _) 1) ((Ast.List _) 9))`, a borrow-clean disc-only match whose extracted
/// `Ast` shell is never dropped).
///
/// SOUND by the SAME fence as the proj/view twins. The load-bearing gate is `heap_operand_ownership(<the
/// SumExpect's own scrutinee — the source Option>) == Owned`: `Option.expect` on an OWNED Some TRANSFERS the
/// payload out as owned (the fallible-read producers — `List.at`/`Map.lookup`/`Bytes.at` — `dup` the payload
/// INTO the `Some`, so the extracted value is an INDEPENDENT owned ref, never an alias into a still-live
/// source; the source collection is already reclaimed by the time the match runs). The `SumExpect` emit
/// already `drop`s the Some shell + leaves the extracted payload at rc1, so this reclaim drops exactly that
/// one un-dropped shell (rc1 → 0, balanced; never a double-free). NO-CHILD-DUP path → sound ONLY under the
/// STRICT BORROW-CLEAN floor (zero consuming sites — the payload only READ/disc-probed, never moved into a
/// builder/Call NOR escaped as an arm result); a consuming site → declined (leak beats UAF). An `Option.
/// expect` on a BORROWED Some (source not `Owned`) fails the gate → declined (its payload is borrowed, must
/// stay leaking — dropping it would double-free the owner's ref).
pub(crate) fn matchsum_expect_owned_reclaim_ok(
    db: &mut Db,
    scrutinee: StructId,
    scrut_ty: &Ty,
    stashed_slot: Option<(u32, ValType)>,
    never_diverges: bool,
    root: &crate::core::SumCont,
) -> bool {
    // Shared safety floor (identical to the proj/view twins).
    if !matches!(stashed_slot, Some((_, ValType::I32)))
        || never_diverges
        || !is_heap_type(scrut_ty)
        || ty_is_enum_disc(db, scrut_ty)
        || cont_rematches_scrutinee(db, scrutinee, root)
    {
        return false;
    }
    // The scrutinee must be an `Option.expect` (`Core::SumExpect`) whose SOURCE Option is globally `Owned`
    // (a fallible-read producer / constructor). That is the fence: expect on an owned Some transfers the
    // payload out as an independent owned ref.
    let Core::SumExpect {
        scrutinee: source, ..
    } = core_of(db, scrutinee)
    else {
        return false;
    };
    if !matches!(
        heap_operand_ownership(db, source),
        Ok(HandleOwnership::Owned)
    ) {
        return false;
    }
    // STRICT BORROW-CLEAN floor (NO child-dup): the extracted payload must have ZERO consuming sites.
    let mut consuming = HashSet::new();
    collect_consuming_payload_sites_cont(db, root, scrutinee, &mut consuming);
    consuming.is_empty()
}

/// The MATCH-EXTRACTION owned-locally twin of [`matchsum_expect_owned_reclaim_ok`] (the 4th such producer,
/// after the proj/expect/view paths): a `MatchSum` whose SCRUTINEE is itself an INLINED `Core::MatchSum` that
/// ESCAPES an owned heap child. `top`'s `(match (dn …) (#tuple(ast pos) ast))` inlines into `main`'s `(match
/// (top …) ((AInt n) n) (_ -1))`, so `main`'s scrutinee node IS that inner `MatchSum`, and its runtime value
/// is the escaping `AInt` shell the inner escaping-proj emit (#9391) already dup'd to rc>=1. The global
/// `heap_operand_ownership` conservatively classes a match-extraction as BORROWED (as it does Proj/SumExpect/
/// StrAt), so `sum_shell_reclaim_ok`'s `Owned` gate MISSES it and the extracted shell LEAKS one cell
/// (02-binding-and-control:6042 RESIDUAL — `main`'s returned-`AInt` COMPOUND-payload shell). Treat it as owned
/// LOCALLY (the same local>global discipline the proj/expect/view twins use): the child escaped OWNED, and it
/// is dead-after in the outer match, so its shell is a dead owned temporary.
///
/// SOUNDNESS (UAF-sensitive — why the all-scalar TYPE floor forbade compound shells): a boxed-sum shell
/// deep-drop is runtime-disc-aware (frees only the entered variant's children), so it is safe UNLESS an arm
/// ALIASES a heap child out past the match (the sread UAF). `nontail_param_compound_extra_ok` (G4 no arm
/// returns a heap payload / the shell whole; G5 no arm interior-view-aliases a child; not-re-matched; not-
/// returns-scrutinee) is the PRECISE per-arm guard the coarse all-scalar TYPE floor over-approximated — gate
/// on it directly. `main`'s `(AInt n) n` arm copies out a SCALAR (no alias) → passes; a heap-aliasing arm
/// declines (leak beats UAF). The inner-match escaping-proj gate ([`matchsum_escaping_proj_node`]`.is_some`) is
/// REQUIRED: it proves the child escaped OWNED (the inner emit dup'd it). A borrow-only inner match (no dup)
/// would leave the child rc-SHARED with the inner shell, so the outer deep-drop would DOUBLE-FREE → declined.
#[allow(dead_code)] // TEMP: inert until the emit.rs:4166 `reclaim_shell` OR-term is wired (co-fix w/ v-core-opt).
pub(crate) fn matchsum_matchextract_owned_reclaim_ok(
    db: &mut Db,
    scrutinee: StructId,
    scrut_ty: &Ty,
    stashed_slot: Option<(u32, ValType)>,
    never_diverges: bool,
    root: &crate::core::SumCont,
) -> bool {
    // Shared safety floor (identical to the proj/expect/view twins) + whole-scrutinee dead-after.
    if !matches!(stashed_slot, Some((_, ValType::I32)))
        || never_diverges
        || !is_heap_type(scrut_ty)
        || ty_is_enum_disc(db, scrut_ty)
        || cont_rematches_scrutinee(db, scrutinee, root)
        || !scrutinee_dead_after_destructure(db, scrutinee, root)
    {
        return false;
    }
    // The scrutinee must be an INLINED `MatchSum` that ESCAPES an OWNED heap child — escaping-proj-recognized,
    // so the inner emit dup'd the child (it escapes rc>=1 OWNED, not borrow-shared with the inner shell).
    let Core::MatchSum {
        scrutinee: inner,
        root: inner_root,
    } = core_of(db, scrutinee)
    else {
        return false;
    };
    let inner_ty = type_of(db, inner);
    // `false` for the inner's `never_diverges`: permissive (the node fn bails on `true`); a never-diverging
    // inner escapes NO value for the outer to match, so this cannot admit an unsound case.
    if matchsum_escaping_proj_node(db, inner, &inner_ty, false, inner_root.as_ref()).is_none() {
        return false;
    }
    // The PRECISE per-arm alias fence (G4/G5) the all-scalar TYPE floor over-approximated: a scalar-extracting
    // / borrow-clean outer arm reclaims; any heap-child alias-out declines (leak-over-UAF).
    nontail_param_compound_extra_ok(db, scrutinee, scrut_ty, never_diverges, root)
}

/// Whether `id` is a child EXTRACTION — a `Core::SumPayload`/`Core::Proj`, or a chain of them — rooted at the
/// match SCRUTINEE (by node id, or, for a `Param`/`LocalRef` scrutinee, its binder — the SAME identity test
/// [`scrutinee_dead_after_destructure`] uses). A tuple pattern destructures via `Core::SumPayload` (path
/// `[Elem(i)]`), a record/tuple field via `Core::Proj` — both root here. `id` being the BARE scrutinee
/// returns `false` (a whole-shell move, not a child extraction). A `Core::Call` scrutinee has no binder, so
/// the node-id root is essential.
#[allow(dead_code)] // TEMP: used only by the inert `matchsum_escaping_proj_reclaim` until the emit is wired.
fn extraction_roots_at_scrutinee(
    db: &mut Db,
    id: StructId,
    scrutinee: StructId,
    binder: Option<StructId>,
) -> bool {
    match core_of(db, id) {
        Core::SumPayload {
            scrutinee: operand, ..
        }
        | Core::Proj { operand, .. } => {
            operand == scrutinee
                || binder.is_some_and(|b| is_ref_to(db, operand, b))
                || extraction_roots_at_scrutinee(db, operand, scrutinee, binder)
        }
        _ => false,
    }
}

/// RECOGNIZER (v-memory-safety recognition lane) for the escaping-heap-child `MatchSum` shell reclaim — the
/// co-fix half v-core-opt's emit consumes at emit.rs:4166 (02-binding-and-control:6042, the mutual-recursion
/// tuple-match; the recursive-descent-parser sibling of the landed 02:6085 Proj-of-LET fix). The sibling
/// `matchsum_*_reclaim_ok` paths all DECLINE this shape because the arm ESCAPES a heap sub-value (its result
/// IS the extracted child), so the borrow-clean floor fails — a bare shell deep-drop would cascade-free the
/// still-live returned child (UAF). The reclaim is instead the project+dup+deep-drop shape: dup the escaping
/// child (rc>=2) BEFORE it escapes, then deep-drop the shell (cascade nets the child rc 2->1, result-safe,
/// and reclaims the shell + releases its cell-ref so the consequential child leak balances).
///
/// `Some(node)` = the recognized shape — return the SOLE escaping heap child's EXTRACTION NODE (a
/// `Core::SumPayload`/`Core::Proj` off the scrutinee; the arm result IS this node), so the emit dups that
/// node's result (rc>=2) before the escape and fires the shell deep-drop. `None` = BAIL (keep the sound
/// leak, leak-over-UAF): not the shape, a CONDITIONAL / multi-arm escape (Guarded/LitTest/Switch — v1 admits
/// only a single-`Leaf` arm), a bare-scrutinee (whole-shell) move, an FBIP-rebuild arm (a compound-
/// constructing arm could reuse the projected cell), a scalar child (copies out — reclaimed by the
/// all-scalar floor), or the standard owned/dead-after gates fail. ANY unproven condition → `None`.
/// Conservative by construction — a false `None` only leaks, never a UAF.
/// DROP-SIDE recognizer (emit.rs:4166, has the stashed slot). The shell deep-drop is only valid when the
/// scrutinee was freshly stashed into an I32 reclaim slot; then delegate to the slot-INDEPENDENT
/// [`matchsum_escaping_proj_node`] the DUP-side also calls. LOCKSTEP: the drop-side is the slot-gated SUBSET
/// of the dup-side, so drop ⊆ dup ⇒ a shell deep-drop NEVER fires without the protecting child-dup (no UAF);
/// a dup fired where the drop declines (slot absent) is only an orphaned-dup leak (safe, census-caught).
#[allow(dead_code)] // TEMP: inert until v-core-opt wires the emit.rs:4166 dup-escaping-child + deep-drop path.
pub(crate) fn matchsum_escaping_proj_reclaim(
    db: &mut Db,
    scrutinee: StructId,
    scrut_ty: &Ty,
    stashed_slot: Option<(u32, ValType)>,
    never_diverges: bool,
    root: &crate::core::SumCont,
) -> Option<StructId> {
    if !matches!(stashed_slot, Some((_, ValType::I32))) {
        return None;
    }
    matchsum_escaping_proj_node(db, scrutinee, scrut_ty, never_diverges, root)
}

/// The SLOT-INDEPENDENT recognition core of [`matchsum_escaping_proj_reclaim`] — every gate EXCEPT the
/// drop-side's `stashed_slot` check. Consumed by the DUP-side (`collect_shell_reclaim_child_dups_seen`, which
/// runs PRE-EMIT and has no stashed slot) so it inserts into `dup_sites` the SAME escaping-extraction node
/// the drop-side reclaims — keeping dup⇔drop LOCKSTEP on one node. Returns `Some(node)`/`None` with the same
/// semantics as [`matchsum_escaping_proj_reclaim`] (which see for the shape + bail conditions).
#[allow(dead_code)] // TEMP: inert until v-core-opt wires the dup-side (collect_shell_reclaim_child_dups_seen).
pub(crate) fn matchsum_escaping_proj_node(
    db: &mut Db,
    scrutinee: StructId,
    scrut_ty: &Ty,
    never_diverges: bool,
    root: &crate::core::SumCont,
) -> Option<StructId> {
    // Shared safety floor (mirrors the proj/view/expect twins, minus the slot gate): returns normally (not
    // never-diverging), heap non-enum sum, not re-matched by a nested MatchSum (Class-B), and the whole
    // scrutinee is DEAD after the destructure (no non-extracting reference keeps it live past the shell drop).
    if never_diverges
        || !is_heap_type(scrut_ty)
        || ty_is_enum_disc(db, scrut_ty)
        || cont_rematches_scrutinee(db, scrutinee, root)
        || !scrutinee_dead_after_destructure(db, scrutinee, root)
    {
        return None;
    }
    // The scrutinee is an OWNED fresh producer (a Call/SumNew/inlined-If result, or globally Owned) — its
    // shell is a dead owned temporary safe to deep-drop once the escaping child is independently retained.
    // A borrowed/shared scrutinee would double-free the owner's ref → excluded.
    if !matches!(
        heap_operand_ownership(db, scrutinee),
        Ok(HandleOwnership::Owned)
    ) && !is_fresh_owned_sum_producer(db, scrutinee)
    {
        return None;
    }
    // FBIP-rebuild guard: an arm that CONSTRUCTS a compound could reuse the projected child's cell in place,
    // which the shell deep-drop would then double-free even though the escape walk (seeing only borrowing
    // projections) reports it safe. Decline any compound-constructing arm (leak, not UAF).
    if sum_cont_arm_constructs_compound(db, root) {
        return None;
    }
    // SOLE escaping heap child (v1): a SINGLE `Leaf` arm whose body IS a pure projection chain off the
    // scrutinee returning a HEAP child (the arm result IS `(. scrut i)`). A multi-arm cont (Guarded/LitTest/
    // Switch) is a CONDITIONAL escape → bail (a runtime-single-arm dup would over/under-count). A non-
    // projection or bare-scrutinee (empty path = whole-shell move) → bail. A scalar child copies out (no
    // shared ref) → not this path (the all-scalar floor reclaims it). Widened later; conservative now.
    let crate::core::SumCont::Leaf(body) = root else {
        return None;
    };
    let binder = match core_of(db, scrutinee) {
        Core::Param { binder } | Core::LocalRef { binder } => Some(binder),
        _ => None,
    };
    // The Leaf body IS the escaping child extraction: a SumPayload/Proj rooted at the (owned, dead-after)
    // scrutinee, returning a HEAP child. A scalar child copies out (no shared ref → the all-scalar floor
    // reclaims it); a non-extraction / bare-scrutinee body is a whole-shell move → bail.
    if !is_heap_type(&type_of(db, *body))
        || !extraction_roots_at_scrutinee(db, *body, scrutinee, binder)
    {
        return None;
    }
    Some(*body)
}

/// The scrutinee-shell-reclaim gates that are INDEPENDENT of how the scrutinee's handle is held (stashed
/// temp vs proven-owned param slot): heap + non-enum + non-diverging + payload-safety + not-re-matched.
/// [`sum_shell_reclaim_ok`] ANDs the stashed-Owned requirement on top; the non-tail-spine param path ANDs
/// the proven-owned-dead-after param membership on top. Splitting these lets BOTH reclaim a shell soundly
/// while the payload/rematch soundness stays in ONE place.
pub(crate) fn sum_shell_reclaim_payload_ok(
    db: &mut Db,
    scrutinee: StructId,
    scrut_ty: &Ty,
    never_diverges: bool,
    root: &crate::core::SumCont,
) -> bool {
    !never_diverges
        && is_heap_type(scrut_ty)
        && !ty_is_enum_disc(db, scrut_ty)
        // The all-scalar floor is always safe (a scalar payload copies out). A COMPOUND payload is
        // reclaimable when the arm is escape-clean (no heap sub-value read out as a live handle) AND
        // reuse-clean (no arm constructs a compound that could FBIP-reuse a payload cell).
        // inc2b (Stage A): an escape-clean + reuse-clean compound Some reclaims even when its scrutinee is
        // a fallible EXTRACTION op (List.at/Map.lookup/Bytes.slice). The extraction dup-retains the payload
        // into the Some, so the payload is at rc>=2 through the arm — any in-place FBIP reuse of its cell is
        // structurally suppressed (node_rc: rc>1 path-copies, never mutates in place), and the deep-drop's
        // cascade merely decrements the extra retained ref (the source keeps its own ref). Verified on the
        // debug runtime: the extraction family reclaims value-correct with zero traps (mts1 6->3 no-trap,
        // p.rc>=2 held through the rebuild since the shell drop fires AFTER the arm body). The earlier
        // scrutinee_is_fallible_extraction decline was over-conservative. Stage B (the third OR branch,
        // sum_cont_extraction_consume_allowlisted) additionally reclaims an escape-clean=FALSE extraction
        // Some whose payload is CONSUMED by a pure allowlisted builder (List.concat/push/insert/…): the
        // builder is a single-owned-ref move, so the dup-on-escape balances the deep-drop 1:1. A consume by
        // an opaque Call/CallClosure (incl. a reduced resume-thread — resume is invisible in Core) is NOT a
        // builder child, so it declines (leak beats UAF).
        && (sum_has_only_scalar_payloads(db, scrut_ty)
            // (FIND3, v-mem-safety-confirmed) ALL-SCALAR PRODUCT scrutinee (a Tuple/Record of all-scalar
            // fields): every field is EXTRACTED via get-int/get-bool (COPIED, not a cell alias), so the arm
            // CANNOT FBIP-reuse the old product's cells → the shell-deep-drop is safe EVEN WHEN the arm builds
            // a compound (the `!arm_constructs_compound` FBIP suppression is spurious here). The Tuple/product
            // analog of the all-scalar-payload floor + #4939. Fixes the arg-scaling group (fib fast-doubling,
            // Catalan/Pascal/look-and-say/pairwise-swap — recursive builds' intermediate scalar-tuples).
            // GATED (both load-bearing, v-mem-safety): OWNED scrutinee (a fresh recursive-`Call` result — NEVER
            // a BORROWED handler threaded-state, which can resume-escape invisibly; rrb1) AND DEAD-AFTER-
            // DESTRUCTURE (the whole scrutinee not re-referenced/escaping in any arm — rrb1's `(resume -1 st)`).
            || (ty_is_all_scalar_product(db, scrut_ty)
                // The scrutinee is a fresh recursive-`Call` result (an Owned COMPUTED value consumed by this
                // match) — NEVER a handler THREADED-STATE (an If/materialize/Param that a `resume` re-reads
                // invisibly, rrb1). A `Core::Call` result is inlined once as the match scrutinee and cannot be
                // resume-threaded, so it IS dead after destructure — the sound proxy for v-effects' "exclude
                // any resuming arm" that is decidable at select.rs (the resume-escape being pre-reduction).
                // Keeps the arg-scaling group (fib/Catalan/Pascal recursive-tuple results); excludes rrb1.
                && matches!(core_of(db, scrutinee), Core::Call { .. })
                && scrutinee_dead_after_destructure(db, scrutinee, root))
            || (!sum_cont_arm_borrows_heap_subvalue(db, root)
                && !sum_cont_arm_constructs_compound(db, root))
            || sum_cont_extraction_consume_allowlisted(db, root, scrutinee)
            // COMPUTED-Some sibling of the extraction-consume allowlist (05:#9134): an OWNED `Core::Call`
            // `Some` result, dead-after-destructure, whose payload is consumed by an allowlisted
            // single-owned-ref-move builder. `owned_compound_boxed` dups the payload (rc>=2 through the arm),
            // so the shell deep-drop balances 1:1 exactly as the extraction case. Fixes the runtime-heap
            // computed-Some StrToBytes over-retain (Some shell + payload) the 289e75fbba extraction fix missed.
            || sum_cont_owned_call_consume_allowlisted(db, root, scrutinee)
            // (5) EXTRACTION-BORROWED-PROBE (CHAMP-key, v-mem-safety-approved as a new disjoint disjunct — do
            // NOT broaden branch (3)'s !arm_constructs_compound, which is the general FBIP fence for non-
            // extraction scrutinees where rc>=2 is not guaranteed). Reclaim a compound Some from a fallible
            // EXTRACTION (List.at/Bytes.at/Str.at/Str.slice/Bytes.slice/Map.lookup) whose arm is BORROW-CLEAN,
            // EVEN WHEN the arm constructs a compound. Two hazards, two conjuncts, both load-bearing:
            //   (i) FBIP-REUSE — scrutinee_is_fallible_extraction: the extraction dup-retains the payload into
            //       the Some, holding it at rc>=2 through the arm (the inc2b property, mts1 6->3 no-trap), so
            //       any in-place FBIP reuse of its cell path-copies (node_rc: rc>1 never mutates) → the arm's
            //       compound build cannot alias/consume the payload cell.
            //   (ii) ESCAPE — !sum_cont_arm_borrows_heap_subvalue: no heap sub-value is read out as a live
            //       handle outliving the shell-deep-drop (the borrow-relax above classifies a Set.contains/
            //       Map.lookup/value-eq key/probe as a borrow, NOT an escape). A view CONSUMED/STORED into a
            //       collection (Set.of/insert, Map.insert — MIXED/STORED negative controls) is NOT in the
            //       borrow allowlist → arm_borrows stays TRUE → this disjunct does NOT fire → those stay a
            //       defined leak, never a double-free. The outer `&& !cont_rematches_scrutinee` (Class-B) still
            //       applies. This is the borrowed-probe sub-case of the Stage-A extraction-rebuild inc2b already
            //       verified — strictly SAFER (the payload is only READ, never flows into the compound).
            || (scrutinee_is_fallible_extraction(db, scrutinee)
                && !sum_cont_arm_borrows_heap_subvalue(db, root)))
        // Class-B UAF (cb3-5): a scrutinee RE-MATCHED by a NESTED `MatchSum` in an arm (`match s { Circle =>
        // match s { … } … }`, `s` an owned/inlined sum) is reclaimed by the INNER match's shell-drop already;
        // this ENCLOSING reclaim would deep-drop the SAME handle a 2nd time → double-free. Suppress the
        // enclosing reclaim when the scrutinee recurs as a nested-match scrutinee — the innermost reclaim
        // fires once, rc balances. Leak-safe if the nested match is only in SOME arms (a non-re-matching arm
        // then leaves the shell un-reclaimed = a value-correct leak, never a UAF).
        && !cont_rematches_scrutinee(db, scrutinee, root)
}

/// Whether the outer MatchSum's owned `scrutinee` is RE-MATCHED — appears as the scrutinee of a NESTED
/// `MatchSum` within `root`'s arm bodies (Class-B UAF cb3-5). Keyed on the scrutinee NODE (a CSE-shared
/// re-match) and, when the scrutinee is a `Param`/`LocalRef`, its BINDER (a distinct-node same-binder
/// re-match). Used by [`sum_shell_reclaim_ok`] to SUPPRESS the enclosing shell-reclaim so the innermost
/// match's reclaim is the sole drop of the shared owned scrutinee (no double-free).
pub(crate) fn cont_rematches_scrutinee(
    db: &mut Db,
    scrutinee: StructId,
    root: &crate::core::SumCont,
) -> bool {
    let tgt_binder = match core_of(db, scrutinee) {
        Core::Param { binder } | Core::LocalRef { binder } => Some(binder),
        _ => None,
    };
    let mut seen = HashSet::new();
    cont_rematches_scrutinee_cont(db, scrutinee, tgt_binder, root, &mut seen)
}

pub(crate) fn cont_rematches_scrutinee_cont(
    db: &mut Db,
    scrutinee: StructId,
    tgt_binder: Option<StructId>,
    cont: &crate::core::SumCont,
    seen: &mut HashSet<StructId>,
) -> bool {
    match cont {
        crate::core::SumCont::Leaf(body) => {
            expr_rematches_scrutinee(db, scrutinee, tgt_binder, *body, seen)
        }
        crate::core::SumCont::Guarded { cond, body, els } => {
            expr_rematches_scrutinee(db, scrutinee, tgt_binder, *cond, seen)
                || expr_rematches_scrutinee(db, scrutinee, tgt_binder, *body, seen)
                || cont_rematches_scrutinee_cont(db, scrutinee, tgt_binder, els, seen)
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            cont_rematches_scrutinee_cont(db, scrutinee, tgt_binder, then_, seen)
                || cont_rematches_scrutinee_cont(db, scrutinee, tgt_binder, els, seen)
        }
        crate::core::SumCont::Switch { arms, .. } => arms
            .iter()
            .any(|a| cont_rematches_scrutinee_cont(db, scrutinee, tgt_binder, &a.cont, seen)),
    }
}

pub(crate) fn expr_rematches_scrutinee(
    db: &mut Db,
    scrutinee: StructId,
    tgt_binder: Option<StructId>,
    id: StructId,
    seen: &mut HashSet<StructId>,
) -> bool {
    if !seen.insert(id) {
        return false;
    }
    // A nested match — Sum, List, OR scalar — RE-MATCHING the same scrutinee (by node id, or by binder when
    // the scrutinee is a `Param`/`LocalRef`). Covers Class-B for both `MatchSum` (cb3-5) and `MatchList`
    // (breaker's runtime-list re-match UAF): the inner match reclaims/re-reads the shared owned scrutinee, so
    // the enclosing reclaim must be suppressed.
    let nested_scrut = match core_of(db, id) {
        Core::MatchSum { scrutinee: s2, .. }
        | Core::MatchList { scrutinee: s2, .. }
        | Core::Match { scrutinee: s2, .. } => Some(s2),
        _ => None,
    };
    if let Some(s2) = nested_scrut {
        if s2 == scrutinee {
            return true;
        }
        if let Some(b) = tgt_binder
            && matches!(core_of(db, s2), Core::Param { binder } | Core::LocalRef { binder } if binder == b)
        {
            return true;
        }
    }
    core_child_ids(db, id)
        .into_iter()
        .any(|c| expr_rematches_scrutinee(db, scrutinee, tgt_binder, c, seen))
}

/// Whether the outer `MatchList`'s `scrutinee` is RE-MATCHED (appears as the scrutinee of a NESTED match)
/// within any arm body/guard — the list analogue of [`cont_rematches_scrutinee`]. Used by
/// [`list_shell_reclaim_slot`] to SUPPRESS the enclosing list shell-reclaim so the shared owned list is not
/// deep-dropped while a nested `match xs` still reads it (breaker's Class-B runtime-list re-match UAF).
pub(crate) fn list_arms_rematch_scrutinee(
    db: &mut Db,
    scrutinee: StructId,
    arms: &[crate::core::ListArm],
) -> bool {
    let tgt_binder = match core_of(db, scrutinee) {
        Core::Param { binder } | Core::LocalRef { binder } => Some(binder),
        _ => None,
    };
    let mut seen = HashSet::new();
    arms.iter().any(|a| {
        expr_rematches_scrutinee(db, scrutinee, tgt_binder, a.body, &mut seen)
            || a.guard
                .is_some_and(|g| expr_rematches_scrutinee(db, scrutinee, tgt_binder, g, &mut seen))
    })
}

/// Whether EVERY variant of the sum type `sum` carries either NO payload (nullary) or a SCALAR payload
/// (Int/Bool/Float — copied off, never a heap handle). Used to gate the `MatchSum` owned-shell reclaim: it
/// is only sound to drop the scrutinee shell after the match when NO arm can BORROW a heap payload handle
/// out of the shell (a borrowed compound/list/string payload is threaded into the arm body — often a
/// recursive walk — and OUTLIVES the match block, so freeing the shell would free it mid-use → a UAF, the
/// HOL-kernel `term-eq (Comb x y)` regression v-patterns caught). If every payload is a scalar or absent,
/// no handle aliases the shell and the drop is safe. Conservative: an `(Option Int64)` / all-scalar-enum
/// qualifies (the reported List.at/Map.lookup leak), a compound-payload sum does NOT (left un-dropped — a
/// residual leak there, never a double-free). This mirrors the SumExpect gate's scalar-payload arm applied
/// to EVERY variant. Returns false for a non-sum or an unresolvable payload (reject-don't-miscompile).
/// (FIND3) Whether `ty` is a PRODUCT (Tuple/Record) whose fields are ALL SCALAR (Int/Bool/Float). Such a
/// product is destructured field-by-field into COPIED immediates (get-int/get-bool) — no field is a heap
/// handle that could alias into an arm's rebuilt compound (the FBIP-reuse hazard `sum_cont_arm_constructs_
/// compound` guards). So its shell is safely deep-droppable after a scalar-extracting match EVEN WHEN the arm
/// builds a compound. The product analog of [`sum_has_only_scalar_payloads`] (which bails on non-`Sum` types,
/// so a bare Tuple/Record scrutinee never matched it — fib fast-doubling's `(Tuple Int Int)`). Conservative:
/// ANY heap field → false (a heap field could alias the arm's compound = v-mem-safety's heap-extracted
/// must-hold). ONE level (a nested-product field is heap → false). Caller ANDs Owned + dead-after-destructure.
pub(crate) fn ty_is_all_scalar_product(db: &mut Db, ty: &Ty) -> bool {
    fn is_scalar(t: &Ty) -> bool {
        matches!(t.strip_nominal(), Ty::Int(_) | Ty::Bool | Ty::Float(_))
    }
    match ty.strip_nominal() {
        Ty::Tuple(elems) => !elems.is_empty() && elems.iter().all(is_scalar),
        Ty::Record(fields) => !fields.is_empty() && fields.values().all(is_scalar),
        _ => {
            let _ = db;
            false
        }
    }
}

pub(crate) fn sum_has_only_scalar_payloads(db: &mut Db, sum: &Ty) -> bool {
    let stripped = sum.strip_nominal().clone();
    let Ty::Sum { decl, .. } = &stripped else {
        return false;
    };
    let Some(n) = db.type_decl_by_occ(*decl).map(|td| td.variants.len()) else {
        return false;
    };
    (0..n as u32).all(|disc| match variant_payload_ty_at(db, &stripped, disc) {
        // No payload (nullary variant) — nothing to borrow.
        None => true,
        // A scalar payload is copied off (get-int/get-bool), never a handle aliasing the shell.
        Some(ty) => matches!(ty.strip_nominal(), Ty::Int(_) | Ty::Bool | Ty::Float(_)),
    })
}

/// The type reached by a `Payload` step whose FULL path (from the root, INCLUDING this `Payload`) is
/// `prefix`, given the current sub-value type `cur`. Prefer the RECORDED entered-variant payload type in
/// `recorded` (keyed by the absolute path — written as an enclosing switch descended into a specific
/// variant); this is authoritative because it carries WHICH variant was entered, which the flat path alone
/// cannot. Fall back to variant 0 (`sum_single_payload_ty`) only when nothing is recorded (the root switch,
/// whose `cur` IS the scrutinee's type). A NOMINAL newtype `Payload` is a static unwrap to its inner type.
pub(crate) fn payload_step_ty(
    db: &mut Db,
    scrutinee: StructId,
    cur: &Ty,
    prefix: &[crate::core::PathStep],
    recorded: &HashMap<(StructId, Vec<crate::core::PathStep>), Ty>,
) -> Ty {
    payload_step_ty_of(db, scrutinee, None, cur, prefix, recorded)
}

/// [`payload_step_ty`] with an optional SCRUTINEE node, so a `Payload` step whose entered variant was NOT
/// recorded (an enclosing `Switch` was FOLDED AWAY by the `known_disc` optimization — its emit never ran
/// `record_entered_payload_ty`) can recover the ACTUAL entered variant's payload type from the scrutinee's
/// CONSTANT value at this path, instead of falling back to VARIANT 0. When a switch is folded, the sub-value
/// at `prefix[..len-1]` is a compile-time `SumNew{disc}` (that is exactly what `const_at_path`/`known_disc`
/// proved to fold it), so its discriminant is known — and its payload type is `variant_payload_ty_at(sum,
/// disc)`, not variant 0's. Falling back to variant 0 read a nested self-recursive-sum payload at the wrong
/// depth (a `(W (I 7))` over `(type T (I …) (W T))` with a known outer `W` disc: the inner `I` payload was
/// resolved as `I`'s `Int64` from variant 0, erasing the second `Payload` step → a silent MISCOMPILE). Only
/// used where the scrutinee node is in scope (the emit walks); the type-only `payload_step_ty` keeps the
/// variant-0 fallback (its callers already thread `recorded` from an emitted switch, so a miss there is the
/// genuine root/variant-0 case).
pub(crate) fn payload_step_ty_of(
    db: &mut Db,
    root_scrutinee: StructId,
    scrutinee: Option<StructId>,
    cur: &Ty,
    prefix: &[crate::core::PathStep],
    recorded: &HashMap<(StructId, Vec<crate::core::PathStep>), Ty>,
) -> Ty {
    if let Some(t) = recorded.get(&(root_scrutinee, prefix.to_vec())) {
        return t.clone();
    }
    match cur.strip_nominal() {
        Ty::Sum { .. } => {
            // Recover the entered variant from the scrutinee's CONSTANT value at the parent path (the box
            // this `Payload` unwraps). `prefix` ends in `Payload`; its parent is `prefix[..len-1]`.
            if let Some(s) = scrutinee
                && let Some(parent) = prefix.split_last().map(|(_, rest)| rest)
                && let Some(disc) = const_disc_at(db, s, parent)
                && let Some(pt) = variant_payload_ty_at(db, cur, disc)
            {
                return pt;
            }
            sum_single_payload_ty(db, cur).unwrap_or(Ty::Any)
        }
        inner => inner.clone(),
    }
}
