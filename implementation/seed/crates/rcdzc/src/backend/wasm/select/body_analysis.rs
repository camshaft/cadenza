//! Select-time static body/param analyses — the loop-param liveness/invalidation set
//! (`invariant_param_binders`, `invalidate_varying_params*`, the `*_borrowed_or_backedge*` /
//! `*_epilogue_droppable` / `terminal_arms_no_heapchild_escape` / `result_reaches_binder_or_heapchild`
//! / `occurs_in` family) plus LICM (`licm_*`, `collect_hoistable`, the sum-payload-prefix pass) and
//! CSE candidate detection (`collect_cse_candidate_groups`, `is_cse_shareable*`). Extracted verbatim
//! from `select.rs` to keep it under `xtask_support::MAX_SOURCE_BYTES` (512 KiB); pure code move,
//! behavior-neutral. `use super::*` brings the parent `select` module items (Db, Core, Ty, Lir, Emit,
//! Layout, HashSet, the reclaim/arith helpers, ...) into scope, as the other `select/*` submodules do.
//! The moved fns are `pub(super)` so the parent's `use body_analysis::*;` re-imports them, leaving every
//! call site in `select.rs` unchanged. All are internal to `select` (0 external references).
use super::*;

/// The set of loop-invariant PARAMETER BINDERS of a self-loop: those a member tail call NEVER reassigns
/// (every self-call passes the parameter back to its own slot — the `is_identity` shape). Starts with ALL
/// params invariant and REMOVES any that some back-edge changes; a param not threaded identically on even
/// one edge is variant. `slots` maps each param binder to its slot; `param_slots[i]` is param `i`'s slot.
pub(super) fn invariant_param_binders(
    db: &mut Db,
    body: StructId,
    params: &[(StructId, Ty)],
    slots: &HashMap<StructId, u32>,
    members: &[usize],
    self_def: usize,
) -> std::collections::HashSet<StructId> {
    // Begin optimistic: every parameter binder is invariant.
    let mut invariant: std::collections::HashSet<StructId> =
        params.iter().map(|(b, _)| *b).collect();
    let param_slots: Vec<u32> = params
        .iter()
        .map(|(b, _)| *slots.get(b).expect("param binder has a slot"))
        .collect();
    // Walk every SELF tail call (a back-edge) and demote any param its arg does not pass through unchanged.
    invalidate_varying_params(
        db,
        body,
        &param_slots,
        slots,
        members,
        self_def,
        &mut invariant,
        params,
    );
    invariant
}

/// Descend the TAIL positions (the same ones `emit_tail`/`tail_callees` thread) and, at each SELF tail
/// call, drop from `invariant` any parameter whose argument is not exactly its own identity pass-through
/// (`Core::Param{binder}` bound to the same slot). A non-self tail call (`return_call` to a peer/other
/// def) is NOT a back-edge of THIS loop for a single-member group, so it is not walked for invalidation —
/// but a single-member self-loop only has self back-edges anyway (`members == [self_def]`).
#[allow(clippy::too_many_arguments)]
pub(super) fn invalidate_varying_params(
    db: &mut Db,
    id: StructId,
    param_slots: &[u32],
    slots: &HashMap<StructId, u32>,
    members: &[usize],
    self_def: usize,
    invariant: &mut std::collections::HashSet<StructId>,
    params: &[(StructId, Ty)],
) {
    match core_of(db, id) {
        Core::Call { callee, args } if members.contains(&callee) => {
            // A back-edge: param `i` stays invariant only if arg `i` is its own identity pass-through.
            for (i, &arg) in args.iter().enumerate() {
                if i >= param_slots.len() {
                    continue;
                }
                let is_identity = matches!(core_of(db, arg), Core::Param { binder }
                    if slots.get(&binder) == Some(&param_slots[i]));
                if !is_identity {
                    invariant.remove(&params[i].0);
                }
            }
        }
        Core::Call { .. } => {}
        // MULTI-VALUE-UPGRADE back-edge: `(let ((t (member-call …))) (tuple (. t 0) …))` iterates the loop
        // via the bound self-call (see `multivalue_repackage_tail_call` + the `emit_tail` `Core::Let` arm),
        // so its args drive the SAME per-param varying analysis a plain `Core::Call` back-edge does. Without
        // this the varying counter (e.g. `n-1`) is misread as invariant and LICM wrongly hoists the loop
        // condition out — an infinite loop. Handle it BEFORE the generic `Core::Let` body recursion below.
        Core::Let { .. }
            if multivalue_repackage_tail_call(db, id)
                .map(|c| matches!(core_of(db, c), Core::Call { callee, .. } if members.contains(&callee)))
                .unwrap_or(false) =>
        {
            let call = multivalue_repackage_tail_call(db, id).unwrap();
            if let Core::Call { args, .. } = core_of(db, call) {
                for (i, &arg) in args.iter().enumerate() {
                    if i >= param_slots.len() {
                        continue;
                    }
                    let is_identity = matches!(core_of(db, arg), Core::Param { binder }
                        if slots.get(&binder) == Some(&param_slots[i]));
                    if !is_identity {
                        invariant.remove(&params[i].0);
                    }
                }
            }
        }
        Core::If { then_, else_, .. } => {
            invalidate_varying_params(
                db,
                then_,
                param_slots,
                slots,
                members,
                self_def,
                invariant,
                params,
            );
            invalidate_varying_params(
                db,
                else_,
                param_slots,
                slots,
                members,
                self_def,
                invariant,
                params,
            );
        }
        Core::Let { body, .. } => invalidate_varying_params(
            db,
            body,
            param_slots,
            slots,
            members,
            self_def,
            invariant,
            params,
        ),
        Core::Match { arms, .. } => {
            for arm in arms {
                invalidate_varying_params(
                    db,
                    arm.body,
                    param_slots,
                    slots,
                    members,
                    self_def,
                    invariant,
                    params,
                );
            }
        }
        Core::MatchList { arms, .. } => {
            for arm in arms {
                invalidate_varying_params(
                    db,
                    arm.body,
                    param_slots,
                    slots,
                    members,
                    self_def,
                    invariant,
                    params,
                );
            }
        }
        Core::MatchSum { root, .. } => invalidate_varying_params_sum(
            db,
            &root,
            param_slots,
            slots,
            members,
            self_def,
            invariant,
            params,
        ),
        _ => {}
    }
}

/// `invalidate_varying_params` over a sum decision tree — the `SumCont` analogue, descending the same
/// `Leaf`/`Guarded`/`LitTest`/`Switch` tail continuations `sum_cont_tail_callees` does.
#[allow(clippy::too_many_arguments)]
pub(super) fn invalidate_varying_params_sum(
    db: &mut Db,
    cont: &crate::core::SumCont,
    param_slots: &[u32],
    slots: &HashMap<StructId, u32>,
    members: &[usize],
    self_def: usize,
    invariant: &mut std::collections::HashSet<StructId>,
    params: &[(StructId, Ty)],
) {
    match cont {
        crate::core::SumCont::Leaf(body) => invalidate_varying_params(
            db,
            *body,
            param_slots,
            slots,
            members,
            self_def,
            invariant,
            params,
        ),
        crate::core::SumCont::Guarded { body, els, .. } => {
            invalidate_varying_params(
                db,
                *body,
                param_slots,
                slots,
                members,
                self_def,
                invariant,
                params,
            );
            invalidate_varying_params_sum(
                db,
                els,
                param_slots,
                slots,
                members,
                self_def,
                invariant,
                params,
            );
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            invalidate_varying_params_sum(
                db,
                then_,
                param_slots,
                slots,
                members,
                self_def,
                invariant,
                params,
            );
            invalidate_varying_params_sum(
                db,
                els,
                param_slots,
                slots,
                members,
                self_def,
                invariant,
                params,
            );
        }
        crate::core::SumCont::Switch { arms, .. } => {
            for arm in arms {
                invalidate_varying_params_sum(
                    db,
                    &arm.cont,
                    param_slots,
                    slots,
                    members,
                    self_def,
                    invariant,
                    params,
                );
            }
        }
    }
}

/// NARROW, provable-safety (default-DENY) gate for the owned-heap-param loop-exit drop: whether EVERY
/// occurrence of heap PARAMETER `binder` in `id` is either (1) a BORROW (a direct `Param` operand of a
/// match-dispatch / projection / len / sum-payload read — read without consuming) or (2) the loop
/// back-edge (an identity arg to a MEMBER tail-call, which `emit_loop_iteration` turns into an identity
/// slot move). Returns `false` (⇒ do NOT drop, conservatively leak) at the FIRST occurrence that is
/// anything else, and for ANY node kind this walk does not explicitly whitelist.
///
/// DEFAULT-DENY is the point. An earlier "absence-of-escape" gate (delegate uncovered nodes to
/// `binding_escapes`) OVER-FIRED across 3 self-host rounds: "not proven to escape" defaulted to droppable
/// over nodes it didn't model (a non-tail-consumed `tree`, a `MatchSum` sum-payload arm, a threaded-mutated
/// `store`), and dropping a param whose ownership was transferred out double-freed → wasm `unreachable`.
/// A whitelist that can only UNDER-drop (a missed borrow ⇒ a leak, never a double-free) is sound by
/// construction for a UAF-critical reclaim. This is deliberately NARROW: it fires for the witnessed
/// self-recursive-loop shape (a heap param used SOLELY as a base-case-match borrow + the tail-identity
/// back-edge) and declines everything else. The general owned-heap-param pass (the default-deny whitelist
/// extended to model every consuming node precisely) is a documented follow-up, not landed here.
pub(super) fn param_only_borrowed_or_backedge(
    db: &mut Db,
    id: StructId,
    binder: StructId,
    members: &[usize],
    param_slots: &[u32],
    slots: &HashMap<StructId, u32>,
) -> bool {
    param_only_borrowed_or_backedge_rec(db, id, binder, members, param_slots, slots, false, false)
}

/// VARYING-REBOUND variant (INC2 (a) (B) slice-1): like [`param_only_borrowed_or_backedge`] but a member
/// back-edge ACCEPTS a NON-identity arg that only BORROWS or back-edge-RECLAIMED-CONSUMES `binder` — a rebox
/// `(List.concat #list(head) (.. tail))` where `head` is a borrowing read and `tail` a RestFrom (the fresh-
/// tail rule classifies it a borrow, and the per-iteration `drop_old_borrowed`/#7547 reclaims the OLD value
/// on the edge). Decided by `!binding_escapes(arg, binder)`: `binder` only borrowed into the rebox ⟹ its old
/// value is reclaimed on the edge (leak-clean); a WHOLE-consume `(List.concat binder …)` / a heap-child move
/// ESCAPES ⟹ `binding_escapes` = true ⟹ DENY. Every NON-back-edge position still enforces borrow-only (F1:
/// `binder`'s SHELL never escapes into the result / a ctor / a non-member call), so the FINAL loop-param value
/// is dead-at-exit (F2). This is the Q2 half of the varying-param epilogue-drop admit; the no-escape
/// COMPLETENESS half (terminal arms must not reference `binder`) is enforced separately by the caller.
pub(super) fn param_only_borrowed_or_reclaimed_backedge(
    db: &mut Db,
    id: StructId,
    binder: StructId,
    members: &[usize],
    param_slots: &[u32],
    slots: &HashMap<StructId, u32>,
) -> bool {
    param_only_borrowed_or_backedge_rec(db, id, binder, members, param_slots, slots, false, true)
}

/// lgx1 (v-memory-safety co-design): whether a member back-edge ARG reboxes `binder` by CONSUMING it as the
/// BASE COLLECTION of a persistent-extend op (`List.push`/`List.prepend`/`Map.insert`/`Set.insert` base, or a
/// `List.concat`/`Bytes.concat` operand) — the varying-accumulator idiom `worker (List.push acc x)`. Such a
/// consume is RECLAIMED-ON-EDGE (the op consumes `binder` into the new value threaded to the next iteration:
/// FBIP-reuse-in-place at rc1, else path-copy + drop-old), so — UNLIKE a frame-escape / heap-child-MOVE — it
/// does NOT need the whole-function epilogue drop suppressed on its account; the epilogue drop reclaims only
/// the FINAL (base-case, un-consumed) value on the disjoint loop-EXIT path (no double-free — v-mem Q1). GATED
/// to `binder` being the BASE-collection operand ONLY (`is_ref_to`), with `binder` NOT occurring in the
/// element/key/value/other-concat-operand (a heap-child-MOVE like `List.push other (List.at acc 0)` stays
/// denied — `binding_escapes` catches it; v-mem Q2). This is the reclaimed-rebox relaxation's extension from
/// RestFrom-tail-borrow to whole-base-consume.
pub(super) fn arg_reclaims_binder_as_base(db: &mut Db, arg: StructId, binder: StructId) -> bool {
    match core_of(db, arg) {
        Core::ListPush { list, elem } | Core::ListPrepend { list, elem } => {
            is_ref_to(db, list, binder) && !occurs_in(db, elem, binder)
        }
        Core::SetInsert { set, elem, .. } => {
            is_ref_to(db, set, binder) && !occurs_in(db, elem, binder)
        }
        Core::MapInsert { map, key, val, .. } => {
            is_ref_to(db, map, binder) && !occurs_in(db, key, binder) && !occurs_in(db, val, binder)
        }
        Core::ListConcat { lhs, rhs } | Core::BytesConcat { lhs, rhs } => {
            (is_ref_to(db, lhs, binder) && !occurs_in(db, rhs, binder))
                || (is_ref_to(db, rhs, binder) && !occurs_in(db, lhs, binder))
        }
        _ => false,
    }
}

/// The worker, with a `borrowed` flag: `true` iff THIS occurrence is reached through a BORROW position (a
/// projection / len / sum-payload read / match-dispatch scrutinee), where a direct `Param(binder)` is a
/// pure read (OK); `false` in a CONSUME/result position, where a direct `Param(binder)` is an ownership
/// transfer OUT (deny). Mirrors `binding_escapes`'s `tail_borrowed` threading. `allow_reclaimed_rebox`
/// relaxes ONLY the member back-edge arm (see `param_only_borrowed_or_reclaimed_backedge`); `false` = the
/// original coarse `!occurs_in` back-edge rule (the invariant-param path, UNCHANGED).
#[allow(clippy::too_many_arguments)]
pub(super) fn param_only_borrowed_or_backedge_rec(
    db: &mut Db,
    id: StructId,
    binder: StructId,
    members: &[usize],
    param_slots: &[u32],
    slots: &HashMap<StructId, u32>,
    borrowed: bool,
    allow_reclaimed_rebox: bool,
) -> bool {
    // Fast path: a subtree that does not reference `binder` at all is trivially fine (nothing to consume).
    if !occurs_in(db, id, binder) {
        return true;
    }
    let recur = |db: &mut Db, c: StructId, borrowed: bool| {
        param_only_borrowed_or_backedge_rec(
            db,
            c,
            binder,
            members,
            param_slots,
            slots,
            borrowed,
            allow_reclaimed_rebox,
        )
    };
    match core_of(db, id) {
        // A direct reference to the param: OK iff this occurrence is in a BORROW position (read, not
        // consumed). In a consume/result position it transfers ownership out → deny.
        Core::Param { binder: b } => b != binder || borrowed,
        // A member TAIL-call back-edge: every arg that is `binder`'s identity pass-through re-establishes
        // the slot (fine); every OTHER arg must not reference `binder` (a re-boxed `(Mk w)` / non-identity
        // `w` CONSUMES → deny).
        Core::Call { callee, args } if members.contains(&callee) => {
            args.iter().enumerate().all(|(i, &arg)| {
                let is_identity = i < param_slots.len()
                    && matches!(core_of(db, arg), Core::Param { binder: b }
                        if b == binder && slots.get(&binder) == Some(&param_slots[i]));
                if is_identity {
                    return true;
                }
                if allow_reclaimed_rebox {
                    // VARYING-rebound: accept a rebox that only BORROWS / reclaimed-consumes binder.
                    // (a) `!binding_escapes` — RestFrom tail = fresh-tail borrow, reclaimed on the edge.
                    // (b) lgx1: `binder` CONSUMED as the BASE COLLECTION of a persistent-extend rebox
                    //     (`List.push acc x` / prepend / Map/Set.insert base / a concat operand) — reclaimed
                    //     ON THE EDGE (the op consumes it into the threaded new value: FBIP-reuse rc1 / copy+
                    //     drop-old), so the epilogue drop (loop-EXIT path only) reclaims just the FINAL un-
                    //     consumed value → no double-free (v-mem Q1). A heap-child-MOVE stays denied by (a).
                    //     GATED further by the SCALAR-RETURN fence in `varying_param_epilogue_droppable` (v-mem
                    //     Q3): the epilogue drop fires only when the fn returns a SCALAR, so no heap child of
                    //     `binder` can escape the frame via an if/let terminal (the terminal_arms hole).
                    !binding_escapes(db, arg, binder, false)
                        || arg_reclaims_binder_as_base(db, arg, binder)
                } else {
                    // Invariant path: a non-identity arg must not CONSUME/escape binder. Historically the
                    // strict `!occurs_in`; RELAXED (CATALAN, v-memory-safety) to also admit an arg that only
                    // BORROWS binder — recurse it in a RESULT position (`borrowed = false`) so the leaf arms
                    // decide: a direct `Param(binder)` consume DENIES, a SCALAR built from borrows of binder
                    // (conv's `(+ acc (* (at0 c i) (at0 c …)))` — the `at0` = `Option.expect(List.at c i)`
                    // reads are borrows via the `ListAt` scalar-element arm) is a BORROW → OK. The arg's own
                    // VALUE is scalar here so binder does not escape through the recursive-call arg. NEUTRAL
                    // without the `ListAt` borrow arm (the `at0` reads would then DENY, = the old `!occurs_in`).
                    !occurs_in(db, arg, binder) || recur(db, arg, false)
                }
            })
        }
        // BORROW ops: their heap operand is read without consuming → recurse it with `borrowed = true` (a
        // direct `Param` operand is then a pure borrow; a nested `SumPayload{Param}` / borrow chain threads
        // the flag). Other fields (a `Proj` index, a slice bound) are scalars that don't hold `binder`.
        Core::Proj { operand, .. }
        | Core::ListLen { operand }
        | Core::BytesLen { operand }
        | Core::StrScalarLen { operand }
        | Core::BigIntToI64 { operand }
        | Core::CharToInt { operand }
        | Core::IntToCharChecked { operand, .. }
        | Core::RationalNum { operand }
        | Core::RationalDen { operand } => recur(db, operand, true),
        // `List.at` READS an element at a scalar index — it BORROWS the list (like `ListLen`), reclaimed by
        // its owner. SOUND only when the ELEMENT is a SCALAR (non-heap): a heap element is a live child
        // aliasing the list that, if it escapes, makes borrowing-the-list-then-dropping-it-at-the-base a UAF
        // (the #4917 view-producer class) → deny for a heap element (conservative). Index is a scalar.
        // (CATALAN v-memory-safety: conv reads `c : (List Int64)` via `(Option.expect (List.at c i))` — Int64
        // element → borrow → conv's `c` is borrow-only → the looped epilogue drops it at the base, so conv
        // reclaims its threaded `c` and grow's gP2 spare balances → the min-heap-shape leak clears.)
        Core::ListAt { list, .. } => {
            let elem_scalar = match type_of(db, list).strip_nominal() {
                Ty::List(e) => !is_heap_type(e),
                _ => false,
            };
            elem_scalar && recur(db, list, true)
        }
        Core::SumPayload { scrutinee, .. } | Core::SumExpect { scrutinee, .. } => {
            recur(db, scrutinee, true)
        }
        // Match dispatch BORROWS its scrutinee (sum-disc / list-len read) → scrutinee recursed borrowed;
        // each arm body is a RESULT position → recursed unborrowed.
        Core::Match { arms, scrutinee } => {
            recur(db, scrutinee, true) && arms.iter().all(|a| recur(db, a.body, false))
        }
        Core::MatchList { arms, scrutinee } => {
            recur(db, scrutinee, true) && arms.iter().all(|a| recur(db, a.body, false))
        }
        Core::MatchSum { scrutinee, root } => {
            // (it4 gap-a) A fallible interior-view scrutinee (`(List.at xs i)` etc.) BORROWS its container:
            // it reads the container, returning a fresh Option that ALIASES into it (the index/bounds/key are
            // scalars). So a match on such a view over `binder` is a BORROW of binder — but ONLY if the view
            // RESULT is borrow-clean in the arms: if the Option's payload is consumed-into-a-builder / Call /
            // returned-as-result, a live alias into binder ESCAPES the frame and dropping binder at the
            // epilogue would UAF. The F1 fence = `collect_consuming_payload_sites_cont(root, scrutinee)` EMPTY
            // (its "consuming site" contract counts move-into-builder/Call AND escape-as-result), reusing the
            // SAME proven classifier the view_reclaim / owned-scrutinee dup paths use (v-memory-safety
            // co-design; NOT the dead-code G5, which has the wrong polarity). Borrow-clean → recur the
            // container borrowed (binder READ); else fall through to `recur(scrutinee, true)` which denies an
            // unwhitelisted view node (leak-safe). it4: the Some arm get-int-copies the scalar element → no
            // consuming site → empty → xs recognized borrow-only → dropped at the loop epilogue.
            let scrut_ok = match fallible_view_container_of(db, scrutinee) {
                Some(container) if occurs_in(db, container, binder) => {
                    let mut sites = HashSet::new();
                    collect_consuming_payload_sites_cont(db, &root, scrutinee, &mut sites);
                    if sites.is_empty() {
                        recur(db, container, true)
                    } else {
                        recur(db, scrutinee, true)
                    }
                }
                _ => recur(db, scrutinee, true),
            };
            scrut_ok
                && cont_only_borrowed_or_backedge(
                    db,
                    &root,
                    binder,
                    members,
                    param_slots,
                    slots,
                    allow_reclaimed_rebox,
                )
        }
        // Control flow / binding: recurse each sub-position in RESULT (unborrowed) position — the fast path
        // already cleared sub-positions that don't reference `binder`. (A `let` initializer that borrows the
        // param into a scalar binding is rare + not whitelisted here; conservative = leak, never double-free.)
        Core::If { cond, then_, else_ } => {
            recur(db, cond, false) && recur(db, then_, false) && recur(db, else_, false)
        }
        Core::Let { bindings, body } => {
            bindings.iter().all(|&(_, v)| recur(db, v, false)) && recur(db, body, false)
        }
        // CONSUMING value-building ops a self-loop fold's TERMINAL arm builds its result with (INC2 (a) (B)
        // slice-2): recurse EACH value operand in a CONSUME position (`borrowed = false`). SOUND by the
        // existing leaf arms — a DIRECT `Param(binder)` operand is `binder` consumed WHOLE → the `Param` arm
        // denies (its SHELL escapes into the result); a nested `SumPayload`/`Proj` extraction of `binder` →
        // those arms recurse the scrutinee BORROWED (`binder` READ, its shell NOT moved). So `binder`'s SHELL
        // escapes iff it is a direct operand (denied); a SCALAR-child copy built into the result (swap2's
        // `#list(solo) → List.push acc solo` over `List Int64`) is a borrow (admitted). A heap-CHILD moved out
        // is NOT this gate's concern — `terminal_arms_no_heapchild_escape` denies that separately. This is the
        // documented widening of the NARROW whitelist for the self-loop-list-fold terminal-arm result shapes.
        Core::ListPush { list, elem }
        | Core::ListPrepend { list, elem }
        | Core::ListUpdate { list, elem, .. } => recur(db, list, false) && recur(db, elem, false),
        Core::ListConcat { lhs, rhs } => recur(db, lhs, false) && recur(db, rhs, false),
        Core::Tuple { elems } | Core::ListNew { elems } | Core::BytesOf { elems } => {
            elems.iter().all(|&e| recur(db, e, false))
        }
        Core::Record { fields } => fields.values().all(|&v| recur(db, v, false)),
        Core::SumNew { payloads, .. } => payloads.iter().all(|&p| recur(db, p, false)),
        Core::Arith { lhs, rhs, .. } | Core::Compare { lhs, rhs, .. } => {
            recur(db, lhs, false) && recur(db, rhs, false)
        }
        Core::Not { operand } | Core::Convert { operand, .. } => recur(db, operand, false),
        // Every OTHER node kind that references `binder` (a non-member Call, a Closure, a Seq, a mutating op,
        // …) is not whitelisted → deny. NARROW by design (an unlisted shape = a leak, never a double-free).
        _ => false,
    }
}

/// `param_only_borrowed_or_backedge` over a sum-match continuation (the `SumCont` tree): the leaf/guarded/
/// switch bodies are result positions checked the same way; the `Payload`/`Elem` path steps are borrows
/// carrying no binding, so only the continuations matter (mirrors `cont_binding_escapes`).
pub(super) fn cont_only_borrowed_or_backedge(
    db: &mut Db,
    cont: &crate::core::SumCont,
    binder: StructId,
    members: &[usize],
    param_slots: &[u32],
    slots: &HashMap<StructId, u32>,
    allow_reclaimed_rebox: bool,
) -> bool {
    let body_ok = |db: &mut Db, b: StructId| {
        param_only_borrowed_or_backedge_rec(
            db,
            b,
            binder,
            members,
            param_slots,
            slots,
            false,
            allow_reclaimed_rebox,
        )
    };
    match cont {
        crate::core::SumCont::Leaf(body) => body_ok(db, *body),
        crate::core::SumCont::Guarded { body, els, .. } => {
            body_ok(db, *body)
                && cont_only_borrowed_or_backedge(
                    db,
                    els,
                    binder,
                    members,
                    param_slots,
                    slots,
                    allow_reclaimed_rebox,
                )
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            cont_only_borrowed_or_backedge(
                db,
                then_,
                binder,
                members,
                param_slots,
                slots,
                allow_reclaimed_rebox,
            ) && cont_only_borrowed_or_backedge(
                db,
                els,
                binder,
                members,
                param_slots,
                slots,
                allow_reclaimed_rebox,
            )
        }
        crate::core::SumCont::Switch { arms, .. } => arms.iter().all(|a| {
            cont_only_borrowed_or_backedge(
                db,
                &a.cont,
                binder,
                members,
                param_slots,
                slots,
                allow_reclaimed_rebox,
            )
        }),
    }
}

/// INC2 (a) (B) — whether a VARYING-rebound heap param `binder` (one a member back-edge re-binds to a FRESH
/// value, so it is NOT in the `invariant` set) is safe to reclaim at the fn-exit epilogue: the FINAL loop
/// value's shell, since the per-iteration OLD values are already reclaimed on the back-edge by
/// `drop_old_borrowed`/#7547 / the RestFrom `vec-drop`. TWO conjuncts (both conservative — any doubt DENIES,
/// a leak not a UAF):
///  • Q2/F1/F2 — `param_only_borrowed_or_reclaimed_backedge`: `binder` is only BORROWED or back-edge-
///    reclaimed-consumed (its SHELL never escapes into the result / a ctor / a non-member call), so the final
///    value is dead-at-exit and the per-iteration reboxes are reclaimed. (slice-2 widened its whitelist to the
///    self-loop-fold terminal-arm result ops — ListPush/ctors — so a scalar-child copy in the result passes.)
///  • COMPLETENESS (no-heap-child-escape, the P0 closer): every TERMINAL (non-back-edge) arm of a match ON
///    `binder` must have NO heap-CHILD escape (`terminal_arms_no_heapchild_escape`). A heap child moved out
///    (the rc-aware epilogue shell-drop would double-free it) or a spine rest-mint (double-consume) is DENIED;
///    a SCALAR-child copy (no heap sub-value escapes) is ADMITTED — the deep-drop frees only the dead shell +
///    inline scalars, NO dup needed. PASCAL (`#list()`/`#list(_last)` → `acc`) AND PAIRWISE (`#list(solo)` →
///    `List.push acc solo` over `List Int64`, `solo` a scalar copy) both satisfy it. A genuine heap-child move
///    is still denied (that would need the (ii) escaped-child dup — a later slice).
pub(super) fn varying_param_epilogue_droppable(
    db: &mut Db,
    body: StructId,
    binder: StructId,
    members: &[usize],
    param_slots: &[u32],
    slots: &HashMap<StructId, u32>,
) -> bool {
    // HEAP-CHILD-ESCAPE FENCE (lgx1, v-memory-safety Q3 — closes a real UAF hole): the base-case-exit epilogue
    // deep-drop is UAF-safe ONLY if no live heap CHILD of `binder` can escape the frame. `terminal_arms_no_
    // heapchild_escape` runs `arm_borrows_heap_subvalue` ONLY for a MATCH-ON-`binder` terminal — for an `if`/
    // `let`/bare-op terminal its check-set is EMPTY and it returns true VACUOUSLY. So a sibling terminal
    // returning a heap child of `binder` (`(List.at acc 0)`/`(Bytes.slice acc ..)`) would pass vacuously and —
    // with the base-consume relaxation above admitting the back-edge — get the drop, deep-freeing the escaped
    // child → UAF (the sread/tr3 axis-B). ORIGINAL fence was the BLUNT `is_heap_type(body)` (fence on ANY heap
    // return); that OVER-SUPPRESSED — a FRESH construction that merely CONSUMES `binder` (`(List.push acc solo)`,
    // the PAIRWISE/PASCAL solo arm) is a fresh owned escaping value, not a view into `binder`, yet the blunt
    // fence suppressed its owner-drop → LEAK (v-mem rc-trace: PAIRWISE mode-2 leaked 4, mode-3 leaked 2, the
    // fresh (List.push acc solo) rc1 never released). NARROWED (v-core-opt + v-mem co-design): fence ONLY when
    // the body's heap result actually reaches a live heap sub-handle EXTRACTED FROM `binder` (`result_reaches_
    // binder_or_heapchild`) — then a fresh construction reclaims (fixes PAIRWISE/PASCAL) while a genuine heap-
    // child escape still fences (no UAF). SOUND-toward-fence (over-cover = leak, never a UAF). Strictly narrower
    // than the old blunt fence, so it can only reclaim MORE, never suppress a drop the old fence permitted.
    if is_heap_type(&type_of(db, body)) && result_reaches_binder_or_heapchild(db, body, binder) {
        return false;
    }
    param_only_borrowed_or_reclaimed_backedge(db, body, binder, members, param_slots, slots)
        && terminal_arms_no_heapchild_escape(db, body, binder, members)
}

/// COMPLETENESS walk for [`varying_param_epilogue_droppable`]: every TERMINAL (does-not-reach-a-member-tail-
/// call) arm of a `Match`/`MatchList` whose SCRUTINEE is `binder` must have NO heap-CHILD escape — the rc-
/// aware epilogue shell-drop of the final value would cascade-free a moved-out heap child. A terminal arm may
/// reference `binder` ONLY via NON-heap-escaping reads (a SCALAR child copy `vec-get` — swap2's `#list(solo)`
/// over `List Int64`); DENIED on:
///  • `arm_borrows_heap_subvalue`: a heap sub-value read OUT of a compound in a CONSUME/RESULT position
///    (a heap child moved out — that would need the (ii) escaped-child dup, a later slice). SOUND-toward-
///    escape (no false negative), so FALSE ⟹ no heap child moved out (a scalar copy does not).
///  • `body_rest_mints_binder`: a `(.. t)` tail-mint of `binder` in a terminal arm `vec-drop`-CONSUMES the
///    spine → the epilogue shell-drop would double-consume (arm_borrows_heap_subvalue's RestFrom blind spot).
/// (slice-1 required NO REFERENCE = PASCAL; slice-2 relaxes to no-heap-child-escape = PAIRWISE's scalar
/// `solo`, still NO dup, still not the P0.) A `MatchSum`-on-binder is conservatively DENIED (a later slice).
/// Cycle-guarded; DENIES on the first heap-child-escaping/rest-minting terminal arm or any `MatchSum`-on-binder.
pub(super) fn terminal_arms_no_heapchild_escape(
    db: &mut Db,
    body: StructId,
    binder: StructId,
    members: &[usize],
) -> bool {
    fn scrut_is(db: &mut Db, scrut: StructId, binder: StructId) -> bool {
        matches!(core_of(db, scrut),
            Core::Param { binder: b } | Core::LocalRef { binder: b } if b == binder)
    }
    fn walk(
        db: &mut Db,
        id: StructId,
        binder: StructId,
        members: &[usize],
        seen: &mut HashSet<StructId>,
    ) -> bool {
        if !seen.insert(id) {
            return true;
        }
        // Terminal arm bodies of a match ON binder that we must verify have no heap-child escape.
        let to_check: Vec<StructId> = match core_of(db, id) {
            Core::MatchSum { scrutinee, .. } if scrut_is(db, scrutinee, binder) => {
                return false; // conservative: MatchSum-on-binder not handled yet
            }
            Core::Match { scrutinee, arms } if scrut_is(db, scrutinee, binder) => {
                arms.iter().map(|a| a.body).collect()
            }
            Core::MatchList { scrutinee, arms } if scrut_is(db, scrutinee, binder) => {
                arms.iter().map(|a| a.body).collect()
            }
            _ => Vec::new(),
        };
        for b in to_check {
            // Only TERMINAL (non-back-edge) arms matter (a back-edge arm's binder-consume is reclaimed
            // per-iteration). A heap-child escape or a spine rest-mint would make the epilogue deep-drop
            // double-free / double-consume → DENY. A scalar-child borrow (no heap sub-value escapes) is fine.
            if !body_has_member_tail_call(db, b, members)
                && (arm_borrows_heap_subvalue(db, b)
                    || body_rest_mints_binder(db, b, binder, &mut HashSet::new()))
            {
                return false;
            }
        }
        for c in core_child_ids(db, id) {
            if !walk(db, c, binder, members, seen) {
                return false;
            }
        }
        true
    }
    let mut seen = HashSet::new();
    walk(db, body, binder, members, &mut seen)
}

/// lgx1-fix (v-core-opt + v-memory-safety co-design): whether `body` contains a HEAP-typed EXTRACTION that
/// hands out a live sub-handle aliasing `binder` — `Proj`/`ListAt`/`StrAt`/`StrSlice`/`BytesSlice`/`MapLookup`/
/// `SumExpect`/`SumPayload`(non-`RestFrom`) whose source-compound operand contains `binder`. Such a handle, if
/// it escapes a terminal, would be freed by the epilogue deep-drop of `binder`'s slot → UAF, so the part-2
/// heap-child-escape fence must fire. A FRESH construction that merely CONSUMES `binder` (`List.push`/`prepend`/
/// `insert`/`concat`/record/tuple) is NOT an extraction → does not fire → its fresh owner-ref is reclaimed
/// (fixes the PAIRWISE/PASCAL over-suppress leak, v-mem rc-trace). Scalar extractions (`BytesAt`→byte,
/// `len`/`size`→int, `SetContains`→bool, `StrScalarAt`→Char) are excluded by the `is_heap_type` gate. A
/// `RestFrom` `SumPayload` tail is a FRESH owned sublist (`vec-drop` mints it), not a view into `binder` →
/// excluded. SOUND-toward-fence: scans the WHOLE body (incl. borrowed-scrutinee + back-edge-arg positions) with
/// no position filtering, so it can only OVER-fence (a tracked leak, never a UAF); position precision is a safe
/// later refinement. The set = `arm_borrows_heap_subvalue`'s `is_heap_borrow` (Proj/SumExpect/SumPayload)
/// EXTENDED with the collection/slice/view producers it omits (`ListAt`/`StrAt`/`StrSlice`/`BytesSlice`/
/// `MapLookup`) — omitting those UNDER-covers a `(List.at acc 0)`/`(Bytes.slice acc ..)` heap-child escape = the
/// criterion-#4 UAF. Strictly narrower than the old blunt `is_heap_type(body)` fence, so only reclaims more.
pub(super) fn result_reaches_binder_or_heapchild(
    db: &mut Db,
    body: StructId,
    binder: StructId,
) -> bool {
    fn walk(db: &mut Db, id: StructId, binder: StructId, seen: &mut HashSet<StructId>) -> bool {
        if !seen.insert(id) {
            return false;
        }
        if is_heap_type(&type_of(db, id)) {
            let src: Option<StructId> = match core_of(db, id) {
                Core::Proj { operand, .. } => Some(operand),
                Core::ListAt { list, .. } => Some(list),
                Core::StrAt { string, .. } => Some(string),
                Core::StrSlice { string, .. } => Some(string),
                Core::BytesSlice { bytes, .. } => Some(bytes),
                Core::MapLookup { map, .. } => Some(map),
                Core::SumExpect { scrutinee, .. } => Some(scrutinee),
                // A non-`RestFrom` payload read is a live view into the sum's owned compound; a `RestFrom`
                // tail is a fresh owned sublist (`vec-drop` mints it), so it is NOT a view into `binder`.
                Core::SumPayload { scrutinee, path } => {
                    if matches!(path.last(), Some(crate::core::PathStep::RestFrom(_))) {
                        None
                    } else {
                        Some(scrutinee)
                    }
                }
                _ => None,
            };
            if let Some(s) = src
                && occurs_in(db, s, binder)
            {
                return true;
            }
        }
        core_child_ids(db, id)
            .into_iter()
            .any(|c| walk(db, c, binder, seen))
    }
    let mut seen = HashSet::new();
    walk(db, body, binder, &mut seen)
}

/// Whether `binder` occurs anywhere in the subtree at `id` (a fresh-cache wrapper over `binder_occurs`).
pub(super) fn occurs_in(db: &mut Db, id: StructId, binder: StructId) -> bool {
    let mut cache: HashMap<StructId, bool> = HashMap::new();
    binder_occurs(db, id, binder, &mut cache)
}

/// Whether the node at `id` is LOOP-INVARIANT given the set of invariant param binders — it is built
/// ONLY from invariant params and constants through PURE, side-effect-free operators. CONSERVATIVE: only
/// the enumerated pure scalar/collection-read variants qualify (arithmetic, comparison, conversion,
/// negation, a collection COUNT, a projection / sum-payload read); every other kind — a call, control
/// flow, a heap CONSTRUCTION, a `let`/`LocalRef` (a loop-varying local), a `Captured`/closure — is
/// treated as variant (returns false), so LICM never hoists something it cannot prove invariant. A bare
/// `Param` is invariant iff in the set; a `ConstInt`/`ConstBool`/`Unit` is always invariant.
pub(super) fn licm_invariant(
    db: &mut Db,
    id: StructId,
    inv_params: &std::collections::HashSet<StructId>,
) -> bool {
    match core_of(db, id) {
        Core::ConstInt(_) | Core::ConstBool(_) | Core::Unit => true,
        Core::Param { binder } => inv_params.contains(&binder),
        // Pure scalar operators — invariant iff every operand is.
        Core::Arith { lhs, rhs, .. }
        | Core::Compare { lhs, rhs, .. }
        | Core::StrCmp { lhs, rhs, .. }
        | Core::FloatCompare { lhs, rhs, .. } => {
            licm_invariant(db, lhs, inv_params) && licm_invariant(db, rhs, inv_params)
        }
        Core::Convert { operand, .. } | Core::Not { operand } => {
            licm_invariant(db, operand, inv_params)
        }
        // A collection COUNT / a projection / a sum-payload read is a pure borrowing read — invariant iff
        // the container is. (Its trap-freedom is decided separately by `is_trap_free`.)
        Core::ListLen { operand } | Core::BytesLen { operand } | Core::StrScalarLen { operand } => {
            licm_invariant(db, operand, inv_params)
        }
        Core::MapSize { map } => licm_invariant(db, map, inv_params),
        Core::SetLen { set } => licm_invariant(db, set, inv_params),
        Core::Proj { operand, .. } => licm_invariant(db, operand, inv_params),
        Core::SumPayload { scrutinee, .. } => licm_invariant(db, scrutinee, inv_params),
        // Everything else — calls, control flow, heap builds, LocalRef (a loop-varying let), closures,
        // effects — is conservatively variant. LICM does not hoist it.
        _ => false,
    }
}

/// Whether a node is TRIVIAL to (re)materialize — a bare parameter or a constant. Such a node is already
/// a single `local.get` / immediate at each use, so hoisting it into a slot would only ADD a redundant
/// slot + move; LICM skips it and hoists only NON-trivial invariant computations.
pub(super) fn licm_trivial(db: &mut Db, id: StructId) -> bool {
    matches!(
        core_of(db, id),
        Core::Param { .. } | Core::ConstInt(_) | Core::ConstBool(_) | Core::Unit
    )
}

/// Collect the MAXIMAL hoistable subexpressions of a loop body: trap-free, loop-invariant, non-trivial
/// nodes, taking the OUTERMOST such node on each path (a maximal invariant subtree is hoisted as ONE
/// slot; its invariant sub-parts ride along inside it, needing no separate slot). Descends the body; at a
/// node that is hoistable it records the node and does NOT descend (maximal); otherwise it recurses into
/// the child positions that can CONTAIN a hoistable operand. Returns the node ids in DISCOVERY order
/// (deduplicated), so each is emitted once before the loop. Only pure/analyzable parents are descended —
/// which is sufficient because a hoistable node under an unanalyzed parent is still found when the walk
/// reaches it through the parent's enumerated child positions.
pub(super) fn collect_hoistable(
    db: &mut Db,
    id: StructId,
    inv_params: &std::collections::HashSet<StructId>,
    frontier: &std::collections::HashSet<StructId>,
    out: &mut Vec<StructId>,
) {
    // A non-trivial INVARIANT node is a maximal hoist root when hoisting it before the loop adds no trap.
    // Two ways that holds:
    //   • it is TRAP-FREE — hoisting can add no trap regardless of position; OR
    //   • it is in the loop body's DOMINATING FRONTIER — an ALWAYS-EVALUATED position (the loop condition
    //     `(< i (* n 2))` runs on entry AND on every exit check, even for a 0-iteration loop). Such a node
    //     is evaluated ≥1 time whenever the loop is reached, so pulling it before the loop is TRAP-
    //     EQUIVALENT: a trapping invariant (a checked `(* n 2)`) traps on the first condition check either
    //     way. (A trapping invariant BURIED IN A BRANCH is NOT in the frontier — it might run zero times —
    //     so it stays put, keeping the `is_trap_free` guard for those.)
    // Record it and don't descend (maximal — its invariant sub-parts ride along in the one slot).
    if !licm_trivial(db, id)
        && licm_invariant(db, id, inv_params)
        && (crate::lower::is_trap_free(db, id) || frontier.contains(&id))
        // HEAP-HANDLE HOIST GUARD (Perceus soundness): a hoisted value is materialized ONCE before the loop
        // into a persistent slot and read back each iteration via `slots.get(&id)` — with the refcounts it
        // had at hoist time. That is correct for a SCALAR result (a count/index — copying an i64 is free and
        // rc-neutral). But a heap-HANDLE hoist root emits its dup/retain ONCE in the prologue, while the body
        // may CONSUME it (a `List.push`/`Bytes.concat`/`Map.insert` of the projected handle) once PER
        // ITERATION — so a single hoisted dup covers only the first consume; the second iteration consumes a
        // shared handle at rc==1 and FBIP-mutates it in place, and the loop-carried value DRIFTS. (Repro:
        // `(loop … pr … (List.len (List.push (. pr 0) 99)))` with `pr` a threaded tuple carrying the list —
        // per-iter len drifts 3,3,4,5,… .) A heap invariant that is only BORROWED in the body is safe, but
        // its maximal hoist root is then the enclosing SCALAR read (`List.len (. pr 0)` hoists as one i64
        // slot, the projection riding inside), so refusing a heap-TYPED root loses only the dangerous
        // handle-alone hoist, never the scalar borrow-read wins. A missed hoist is a slower loop, never wrong.
        && !is_heap_type(&type_of(db, id))
    {
        if !out.contains(&id) {
            out.push(id);
        }
        return;
    }
    // Otherwise descend the child positions that can hold a hoistable operand. Enumerated conservatively:
    // exactly the pure operator operands + the control-flow / match sub-positions + call args + the common
    // heap-op operands. An unlisted variant simply is not descended (a missed hoist, never a wrong one).
    for child in licm_children(db, id) {
        collect_hoistable(db, child, inv_params, frontier, out);
    }
}

// ── SHARED SUM-PAYLOAD-PREFIX CSE (per-arm-body) ──────────────────────────────────────────────────
//
// A match arm reading MULTIPLE elements of one payload tuple — `(Node (tuple l r))` binds `l` =
// `SumPayload{s, [Payload, Elem(0)]}` and `r` = `SumPayload{s, [Payload, Elem(1)]}` — re-walks the shared
// `sum-payload(s)` PREFIX per element (the two nodes are not `core_eq`, so the value-numbering CSE does not
// share them; only their prefix is common, and a prefix is a sub-PATH, not a `Core` node). This is the
// canonical AST-walker / linked-list-fold shape (`(Cons (tuple h t))`, `(Node (tuple l r))`).
//
// Fix: before emitting an arm body, compute each such shared prefix ONCE into a slot and record it (keyed
// by `(scrutinee-id, prefix step count)`); the `Core::SumPayload` emit then reads the slot and walks only
// the SUFFIX. SOUND: `op_sum_payload` is TOTAL (never traps — a mismatched node yields NULL, not a trap)
// and BORROWING (returns a handle from `handles.first()` with NO refcount change), so materializing it at
// the arm-body top is trap- and refcount-equivalent to the per-element re-walks, regardless of any control
// flow inside the arm body. Restricted to a prefix ending in `Payload` and shared by ≥2 `SumPayload` nodes
// that extend it with a BORROWING `Elem` step (an `arr-get`/`vec-get`, not a `RestFrom` `vec-drop`, which
// consumes) — so the materialized handle is only ever borrowed, never consumed.

/// Collect the shared SUM-PAYLOAD PREFIXES of `body` worth materializing: each returned
/// `(scrutinee, prefix)` is a path ending in `Payload` that ≥2 distinct `SumPayload` nodes in `body`
/// extend with a further `Elem` step (so both re-walk `<scrutinee>…prefix`). Walks the whole body
/// (through control flow — the arm body may nest `if`/`match`); groups by `(scrutinee, prefix)`.
pub(super) fn collect_sum_payload_prefixes(
    db: &mut Db,
    body: StructId,
) -> Vec<(StructId, Vec<crate::core::PathStep>)> {
    // Every distinct SumPayload node in the body, as (scrutinee, path).
    let mut seen: std::collections::HashSet<StructId> = std::collections::HashSet::new();
    let mut payloads: Vec<(StructId, Vec<crate::core::PathStep>)> = Vec::new();
    fn walk(
        db: &mut Db,
        id: StructId,
        seen: &mut std::collections::HashSet<StructId>,
        payloads: &mut Vec<(StructId, Vec<crate::core::PathStep>)>,
    ) {
        if !seen.insert(id) {
            return;
        }
        if let Core::SumPayload { scrutinee, path } = core_of(db, id) {
            payloads.push((scrutinee, path.to_vec()));
        }
        for child in licm_children(db, id) {
            walk(db, child, seen, payloads);
        }
    }
    walk(db, body, &mut seen, &mut payloads);
    // Tally each PREFIX (a path truncated after a `Payload` step) by how many payload nodes extend it with
    // a following `Elem`. `(scrutinee, prefix)` with a count ≥2 is a shared prefix worth hoisting.
    let mut counts: HashMap<(StructId, usize), usize> = HashMap::new();
    let mut key_path: HashMap<(StructId, usize), (StructId, Vec<crate::core::PathStep>)> =
        HashMap::new();
    for (scrutinee, path) in &payloads {
        // Consider every prefix `path[..k]` that ENDS in `Payload` and is FOLLOWED by an `Elem` (a
        // borrowing read). `RestFrom` never appears mid-path (it is a sole step), so a followed step is
        // always `Elem`/`Payload`; require `Elem` so the materialized prefix handle is only borrowed.
        for k in 1..path.len() {
            if matches!(path[k - 1], crate::core::PathStep::Payload)
                && matches!(path[k], crate::core::PathStep::Elem(_))
            {
                let key = (*scrutinee, k);
                *counts.entry(key).or_insert(0) += 1;
                key_path
                    .entry(key)
                    .or_insert_with(|| (*scrutinee, path[..k].to_vec()));
            }
        }
    }
    // The shared prefixes, unordered (the caller `materialize_payload_prefixes` sorts them shortest-first
    // so a nested prefix's walk can read a shorter already-materialized one).
    counts
        .into_iter()
        .filter(|(_, n)| *n >= 2)
        .filter_map(|(key, _)| key_path.remove(&key))
        .collect()
}

/// Materialize the shared SUM-PAYLOAD prefixes of an arm `body` into fresh slots (a per-arm-body CSE) and
/// register them in `out.payload_prefix_slots` keyed by `(scrutinee, prefix step count)`. Each prefix is
/// emitted ONCE (`<scrutinee> …prefix` — reusing a shorter already-registered prefix via the `SumPayload`
/// emit's fast path, since shorter prefixes are materialized first) and stored into its slot. Returns the
/// keys registered so the caller can REMOVE them after the arm body — fencing the slots to this arm so a
/// sibling arm never reads a payload its own scrutinee value did not produce. Slots are claimed from
/// `*high` upward (never `base`), so the arm body (which emits above `*high`) never clashes with them.
#[allow(clippy::too_many_arguments)]
pub(super) fn materialize_payload_prefixes(
    db: &mut Db,
    body: StructId,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    slots: &HashMap<StructId, u32>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<Vec<(StructId, Vec<crate::core::PathStep>)>, Reject> {
    let mut prefixes = collect_sum_payload_prefixes(db, body);
    if prefixes.is_empty() {
        return Ok(Vec::new());
    }
    // SHORTEST-first: a longer prefix's own walk then reads a shorter already-slotted prefix (the emit's
    // longest-matching-prefix fast path), so a nested payload chain materializes each level once.
    prefixes.sort_by_key(|(_, p)| p.len());
    let mut keys = Vec::new();
    for (scrutinee, prefix) in prefixes {
        let slot = *high;
        *high = slot + 1;
        scratch_ty.insert(slot, ValType::I32); // a payload handle is an i32
        // Emit the prefix as a BARE HANDLE WALK — `<start> …steps` with NO trailing unbox (`get_op`); a
        // prefix ends in `Payload`, so its value is a tuple/record HANDLE, used as-is. Start from the
        // longest ALREADY-registered shorter prefix if one exists (shortest-first order guarantees it is
        // materialized), else from the scrutinee. An `Elem` step is USUALLY a tuple/record `arr-get`, but a
        // LEADING `Elem` off a `List` scrutinee (a sum-with-tuple-payload matched as a LIST ELEMENT — prefix
        // `[Elem(0), Payload]`, whose two tuple binders share it) is a `vec-get` into the RRB vec, NOT a flat
        // `arr-get`. So TRACK the sub-value type down the walk exactly as the main `SumPayload` emit does and
        // pick the accessor per step; a bare unconditional `arr-get` mis-read the vec handle (→ garbage → an
        // `unreachable` trap, the list-element/tuple-payload miscompile). A `RestFrom` never appears in a
        // prefix (it is a sole step, never followed).
        let start = (0..prefix.len()).rev().find_map(|k| {
            out.payload_prefix_slots
                .get(&(scrutinee, prefix[..k].to_vec()))
                .map(|&s| (k, s))
        });
        // The absolute path walked so far (from the scrutinee root) and the CURRENT sub-value type — seeded
        // either from a shorter slotted prefix's recorded type (a `Payload`-ending prefix, else `Any`) or
        // from the scrutinee's own type when starting fresh.
        let mut walked_prefix: Vec<crate::core::PathStep>;
        let mut cur;
        let from = if let Some((k, s)) = start {
            out.push(Lir::LocalGet(s)); // [handle] — the shorter shared prefix
            walked_prefix = prefix[..k].to_vec();
            cur = out
                .sum_path_types
                .get(&(scrutinee, walked_prefix.clone()))
                .cloned()
                .unwrap_or(Ty::Any);
            k
        } else {
            emit(
                db,
                scrutinee,
                slots,
                slot + 1,
                high,
                scratch_ty,
                layout,
                out,
            )?; // [handle]
            walked_prefix = Vec::new();
            cur = type_of(db, scrutinee);
            0
        };
        for step in &prefix[from..] {
            walked_prefix.push(*step);
            match step {
                crate::core::PathStep::Payload => {
                    out.push(Lir::CallImport(OP_SUM_PAYLOAD));
                    cur = match cur.strip_nominal() {
                        Ty::Sum { .. } => payload_step_ty_of(
                            db,
                            scrutinee,
                            Some(scrutinee),
                            &cur,
                            &walked_prefix,
                            &out.sum_path_types,
                        ),
                        inner => inner.clone(),
                    };
                }
                crate::core::PathStep::Elem(i) => {
                    out.push(Lir::ConstI32(*i as i32));
                    // A list element reads the RRB vec (`vec-get`); a tuple/record cell reads the flat array
                    // (`arr-get`). Mirror the main `SumPayload` emit's per-step type-directed choice.
                    if matches!(cur.strip_nominal(), Ty::List(_)) {
                        out.push(Lir::CallImport(OP_VEC_GET));
                        cur = match cur.strip_nominal() {
                            Ty::List(e) => (**e).clone(),
                            _ => Ty::Any,
                        };
                    } else {
                        out.push(Lir::CallImport(OP_ARR_GET));
                        cur = Ty::Any;
                    }
                }
                crate::core::PathStep::RestFrom(_) => {
                    return Err(Reject::decline(
                        "a payload prefix cannot contain a RestFrom step",
                    ));
                }
                crate::core::PathStep::TupleRestFrom(_) => {
                    return Err(Reject::decline(
                        "a payload prefix cannot contain a TupleRestFrom step",
                    ));
                }
            }
        }
        out.push(Lir::LocalSet(slot));
        let key = (scrutinee, prefix.clone());
        out.payload_prefix_slots.insert(key.clone(), slot);
        keys.push(key);
    }
    let _ = base;
    Ok(keys)
}

// ── STRAIGHT-LINE COMMON-SUBEXPRESSION ELIMINATION (CSE) ──────────────────────────────────────────
//
// β-reduction SHARES an argument occurrence at every parameter use site (`beta_reduce` returns the SAME
// `StructId`), so an inlined helper `(def (g s) (+ (+ s s) s))` applied to a non-trivial argument leaves
// the ONE argument node referenced multiple times in the reduced body. `emit` is then called once PER
// reference and re-emits the whole computation each time — `g (* a b)` emits `(* a b)` twice; a heap-
// building argument (`(len xs)` twice over `xs = (build …)`) rebuilds the list at each use. The intra-op
// arith-CSE (`core_eq` in `emit_checked_arith`) only shares the two operands of ONE op, so a node used
// across DIFFERENT ops (or ≥3 times) still duplicates.
//
// This pass computes such a shared node ONCE into a slot and reads the slot at each use (via `emit`'s
// node-keyed `slots.get(&id)` fast path — the same mechanism LICM / the match-scrutinee materialization
// use). It is deliberately SCOPED to the provably-sound subset:
//  • STRAIGHT-LINE body only (no `if`/`match` anywhere) — so every use of a shared node is unconditionally
//    executed; computing it up-front never speculates past a branch (no added trap, no branch-only heap
//    build hoisted, no refcount imbalance from a value live on only one path).
//  • TRAP-FREE shared node (`is_trap_free`) — computing it before the rest can add no trap.
//  • SCALAR result (a non-heap machine value) — a scalar has no refcount, so compute-once-read-N is
//    unconditionally sound; a heap handle would need dup/drop accounting per use (deferred).
//  • NON-TRIVIAL (`!licm_trivial`) — a bare param/const is already a free `local.get`/immediate.
// Emitted INNER-FIRST (smaller subtrees first) so a nested shared node's slot is registered before an
// enclosing shared node reads it.

/// Collect the CSE candidate GROUPS of the body `id`: each returned `Vec<StructId>` is a VALUE-EQUIVALENCE
/// CLASS (all members pairwise `core_eq` — the SAME computation) of shareable, non-trivial, SCALAR nodes
/// whose TOTAL reference count across the class is ≥2 AND that has ≥1 member in the DOMINATING FRONTIER
/// (an always-evaluated position). The dominance requirement is what makes hoisting sound across control
/// flow: the class is computed anyway on entry (its dominating occurrence), so pulling it to a slot up-
/// front adds no work on any path and moves no trap — the other occurrences (in branches / anywhere) then
/// read the slot. `(if (> (* a b) 0) (* a b) (- 0 (* a b)))`: the `(* a b)` in the cond dominates, so the
/// two branch copies collapse to slot reads (3 muls → 1). A class shared ONLY across branches (no
/// dominating member) is NOT hoisted — that would speculate work / a trap onto a path that skips it.
/// Two sources of ≥2 refs both qualify (a single β-shared node ref'd twice, or distinct `core_eq`
/// occurrences), value-numbering unifies them. Groups INNER-FIRST (ascending representative subtree size)
/// so a nested class's slot is registered before an enclosing class's representative reads it.
pub(super) fn collect_cse_candidate_groups(db: &mut Db, body: StructId) -> Vec<Vec<StructId>> {
    let mut counts: HashMap<StructId, u32> = HashMap::new();
    let mut order: Vec<StructId> = Vec::new();
    collect_node_refs(db, body, &mut counts, &mut order);
    let mut dominating: std::collections::HashSet<StructId> = std::collections::HashSet::new();
    collect_dominating_frontier(db, body, &mut dominating);
    // Keep only the shareable / non-trivial / scalar distinct nodes (in first-seen order for determinism).
    let mut cands: Vec<StructId> = Vec::new();
    for id in order {
        if licm_trivial(db, id) || !is_cse_shareable(db, id) {
            continue;
        }
        let ty = type_of(db, id);
        if is_heap_type(&ty) || valtype_of(&ty).is_none() {
            continue;
        }
        cands.push(id);
    }
    // Partition into value-equivalence classes by `core_eq`. A distinct node joins the first class it is
    // `core_eq` to. To avoid an all-pairs O(cands²) `core_eq` scan (each `core_eq` a subtree-cloning
    // walk — the emit path's dominant cost on a WIDE arithmetic body where the "few CSE candidates"
    // assumption fails: N distinct scalar subterms → a singleton-heavy partition → N²/2 `core_eq` calls),
    // BUCKET candidates by a cheap shallow `core_hash_key` FIRST. `core_eq(a,b) ⇒ equal key`, so equal
    // candidates always land in the same bucket; `core_eq` then runs only WITHIN a bucket (near-always a
    // singleton or a genuine equal group), so unequal candidates never pairwise-compare. Behaviour-
    // identical to the old scan (the exact `core_eq` still decides membership within a bucket); the only
    // change is which pairs it is asked about. Distinct hashes never merge, so class identity is stable.
    let mut classes: Vec<Vec<StructId>> = Vec::new();
    let mut by_key: crate::fxhash::FxHashMap<u64, Vec<usize>> = crate::fxhash::FxHashMap::default();
    // Per-partition memo for the full-depth structural hash — each core node hashed once, so keying all
    // candidates is O(total core nodes), not O(candidates · depth).
    let mut hash_memo: crate::fxhash::FxHashMap<StructId, u64> =
        crate::fxhash::FxHashMap::default();
    for id in cands {
        let key = core_hash_key(db, id, &mut hash_memo);
        let bucket = by_key.entry(key).or_default();
        let mut placed = false;
        for &ci in bucket.iter() {
            // Count each within-bucket `core_eq` — the partition's comparison work. With hash-bucketing
            // this is O(#candidates) (each candidate compares only against same-hash predecessors, near
            // always none); the old all-pairs scan made it O(#candidates²). This is the noise-free
            // regression signal (`a_wide_arithmetic_body_partitions_cse_candidates_in_bounded_time`). A
            // per-`Db` counter (not a process-global atomic) so the parallel test harness's other
            // concurrent compiles can't pollute the reading — see `Db::cse_partition_core_eq_calls`.
            #[cfg(test)]
            {
                db.cse_partition_core_eq_calls += 1;
            }
            if core_eq(db, classes[ci][0], id) {
                classes[ci].push(id);
                placed = true;
                break;
            }
        }
        if !placed {
            bucket.push(classes.len());
            classes.push(vec![id]);
        }
    }
    // Keep a class iff (a) its TOTAL reference count (summing each distinct member's multiplicity) is ≥2 —
    // an actual repeat worth naming — AND (b) ≥1 member is in the DOMINATING FRONTIER (always evaluated),
    // so hoisting it to the top is sound on every path. INNER-FIRST by representative size so emitting a
    // class's representative reads any already-slotted nested class instead of recomputing.
    let mut groups: Vec<Vec<StructId>> = classes
        .into_iter()
        .filter(|c| {
            c.iter().map(|m| counts[m]).sum::<u32>() >= 2
                && c.iter().any(|m| dominating.contains(m))
        })
        .collect();
    groups.sort_by_key(|c| subtree_size(db, c[0]));
    groups
}

/// Whether the node at `id` is a PURE, DETERMINISTIC SCALAR computation whose sharing is observably
/// identical to recomputing it — the UNARY analogue of the pairwise [`core_eq`] pure set (arith incl.
/// CHECKED `+`/`-`/`*`, compare, convert, not, proj, sum-payload, a nested pure `if`, or a leaf). A CALL,
/// a heap CONSTRUCT, control flow with an impure sub-part, an effect — anything else — is NOT shareable
/// (returns false). Used by straight-line CSE: sharing such a node computes it ONCE at the point that
/// dominates all its uses (the body is straight-line, so the first use dominates the rest), which
/// preserves its value AND its trap behavior — a trapping subexpression traps at the same first-occurrence
/// point whether shared or duplicated (the exact `core_eq` rationale). Distinct from `is_trap_free` (which
/// EXCLUDES a checked op because hoisting it past a BRANCH could add a trap): here there is no branch, so a
/// checked op is shareable too. NOTE: not restricted to scalar HERE — the caller applies the scalar filter
/// (`is_heap_type`); this predicate is purely about determinism/effect-freedom.
pub(super) fn is_cse_shareable(db: &mut Db, id: StructId) -> bool {
    // Memoize on id ALONE: the verdict is a pure function of the immutable Core subtree (see
    // `Db::is_cse_shareable_memo`). The worker's recursion calls THIS wrapper, so nested queries memoize —
    // turning the per-candidate subtree re-walk (O(N²)+ on deeply-nested expressions) into O(N).
    if let Some(&v) = db.is_cse_shareable_memo.get(&id) {
        return v;
    }
    let v = is_cse_shareable_inner(db, id);
    db.is_cse_shareable_memo.insert(id, v);
    v
}

pub(super) fn is_cse_shareable_inner(db: &mut Db, id: StructId) -> bool {
    // Per-`Db` compile-cost counter (a memo MISS) — surfaced via `CompileOutput::is_cse_shareable_uncached_calls`
    // for the regression guard; scoped to one `Db` so the parallel test harness can't pollute it.
    #[cfg(test)]
    {
        db.is_cse_shareable_uncached_calls += 1;
    }
    match core_of(db, id) {
        Core::ConstInt(_) | Core::ConstBool(_) | Core::Unit | Core::Param { .. } => true,
        // A `let`-LOCAL reference is NOT shareable by this pass: its slot is established only when the
        // `let` binding is emitted INSIDE the body, but CSE hoists a candidate to BEFORE the body — so a
        // hoisted `(* k k)` over a let-local `k` would read an unbound slot ("let-binding reference has no
        // local slot"). Params (slots `0..n`, live up front) are fine; a let-local is excluded so its
        // enclosing subexpression is never hoisted. (The `let`-binding-level CSE — `should_keep_binding`
        // — already names a multiply-used let value; a computation OVER a let-local stays in place.)
        Core::LocalRef { .. } => false,
        Core::Arith { lhs, rhs, .. }
        | Core::Compare { lhs, rhs, .. }
        | Core::StrCmp { lhs, rhs, .. }
        | Core::FloatCompare { lhs, rhs, .. } => {
            is_cse_shareable(db, lhs) && is_cse_shareable(db, rhs)
        }
        Core::Convert { operand, .. } | Core::Not { operand } | Core::Proj { operand, .. } => {
            is_cse_shareable(db, operand)
        }
        // A COLLECTION COUNT (`List.len`/`Bytes.len`/`Map.size`/`Set.len`) is a TOTAL O(1) BORROWING read
        // returning a SCALAR (a `vec-len`/`bytes-len`/`champ-size` runtime import — no refcount change, no
        // effect, deterministic). Sharing two identical counts of the same collection is observably
        // identical to reading twice (same value, no trap), and the RESULT is a scalar so the caller's
        // `is_heap_type` filter admits it (we CSE the count, not the collection handle). The operand must
        // itself be shareable (a param handle / another shareable read) so the read is well-formed at the
        // hoist point. Mirrors `is_trap_free`'s treatment of these counts.
        Core::ListLen { operand } | Core::BytesLen { operand } | Core::StrScalarLen { operand } => {
            is_cse_shareable(db, operand)
        }
        Core::MapSize { map } => is_cse_shareable(db, map),
        Core::SetLen { set } => is_cse_shareable(db, set),
        Core::SumPayload { scrutinee, .. } => is_cse_shareable(db, scrutinee),
        // A `List.at`/`Bytes.at` indexed read (`vec-get`/`bytes-get` after a bounds check) BORROWS the
        // sequence and is DETERMINISTIC — the same (list, index) yields the same element, no rc change on
        // the sequence, no effect. It produces an `Option` (a heap sum), so `ListAt`/`BytesAt` never
        // qualify as a CSE candidate THEMSELVES (the caller's `is_heap_type` filter drops them); they are
        // shareable only as the SCRUTINEE of a scalar-unwrapping `SumExpect` below. Both operands must be
        // shareable so the read is well-formed at the hoist point.
        Core::ListAt { list, index, .. } => {
            is_cse_shareable(db, list) && is_cse_shareable(db, index)
        }
        Core::BytesAt { bytes, index, .. } => {
            is_cse_shareable(db, bytes) && is_cse_shareable(db, index)
        }
        // `Map.lookup` (`map-lookup`) BORROWS the map and is DETERMINISTIC — the same (map, key) yields the
        // same result, no rc change on the map, no effect. It returns an `Option` (a heap sum), so like
        // `ListAt` it never qualifies as a CSE candidate itself (the caller's `is_heap_type` filter drops
        // it); it is shareable only as the SCRUTINEE of a scalar-unwrapping `SumExpect` — so a repeated
        // `(Option.expect (Map.lookup m k))` reading a scalar value shares ONE `map-lookup` (an O(log n)
        // CHAMP walk) instead of two. Both operands must be shareable so the read is well-formed at the
        // hoist point (the key is consumed into an owned temporary; a constant/param key qualifies).
        // (`Set.contains` returns a bare Bool but boxes its element into a fixed scratch slot the CSE hoist
        // can't relocate, so it does not share today — not admitted here to avoid a dead arm.)
        Core::MapLookup { map, key, .. } => is_cse_shareable(db, map) && is_cse_shareable(db, key),
        // `Option.expect`/`Result.expect` on a runtime sum (`SumExpect`) BORROWS its scrutinee and is a
        // deterministic unwrap-or-trap: the same present sum yields the same payload, and an absent one
        // traps — sharing preserves both (the CSE driver only hoists a class with a DOMINATING-frontier
        // member, so the trap fires at the same first-occurrence point whether shared or duplicated, the
        // standard checked-op CSE rationale). When the unwrapped payload is SCALAR (the common
        // `(Option.expect (List.at xs i))` reading an `Int64` element) the whole `SumExpect(ListAt …)` is a
        // scalar-valued borrowing read the caller's `is_heap_type` filter admits — so two identical such
        // reads share one bounds-check + `vec-get` + unbox instead of duplicating the ~20-instr sequence.
        // A heap-payload `SumExpect` is filtered out by the scalar gate, so this arm needs no type guard.
        Core::SumExpect { scrutinee, .. } => is_cse_shareable(db, scrutinee),
        Core::If {
            cond, then_, else_, ..
        } => {
            is_cse_shareable(db, cond) && is_cse_shareable(db, then_) && is_cse_shareable(db, else_)
        }
        _ => false,
    }
}
