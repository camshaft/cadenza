//! Mutual-recursion / tail-call ANALYSIS (multivalue_repackage_tail_call, body/sum_cont member
//! tail-call detection, tail_callees, mutual_loop_group*, sig_valtypes, tail_reaches) — pure graph
//! analysis over the def/tail-call structure, no emit. Extracted from select.rs to keep it under
//! xtask_support::MAX_SOURCE_BYTES (512 KiB). Pure code move, behavior-neutral. `use super::*` brings
//! the select module items (Db, Core, core_of, ValType, TailLoop, ...) into scope, as the sibling
//! select/* submodules do.
use super::*;

/// Recognize the MULTI-VALUE-UPGRADE tail shape and, if present, return the underlying self-call node.
///
/// When a recursive PERFORMER's out-state is OBSERVED after the recursion, the effect lowering upgrades
/// it to return `(value, out-state…)` and rewrites its tail self-call into
/// `(let ((temp (self-call …))) (tuple (. temp 0) (. temp 1) …))` — the call moves into the `let` BINDING
/// INIT and the `let` BODY re-packages `temp`'s slots into the return tuple (effects.rs `drain_and_wrap`).
/// That body is an IDENTITY repackage: `temp` already IS a `(value, out-state…)` tuple, and the body
/// rebuilds exactly `(. temp 0) … (. temp k)` in order — so the whole `let` is semantically just
/// `return self-call(…)`, a genuine tail call the upgrade obscured. Without recognizing it, the wasm loop
/// transform misses the edge and emits a real recursive call (one frame per iteration → stack exhaustion
/// at depth ~5-8k; the Rust backend survives only because rustc/LLVM TCO's the emitted identity tail).
///
/// Returns `Some(call_node)` when `id` is exactly that shape: a single-binding `let` whose init is a
/// `Core::Call`, whose body is a `Core::Tuple` of `arity` elements, and whose i-th element is
/// `Core::Proj { operand → the let binder, index: i }` for every `i` (a full, in-order identity
/// repackage). Any deviation (extra bindings, a non-projection element, a permuted/partial projection, a
/// projection of a different operand, a mismatched arity) returns `None` — so only the exact
/// return-packaging shape is treated as a tail call; genuine post-call computation is never mistaken for
/// one.
pub(super) fn multivalue_repackage_tail_call(db: &mut Db, id: StructId) -> Option<StructId> {
    let Core::Let { bindings, body } = core_of(db, id) else {
        return None;
    };
    if bindings.len() != 1 {
        return None;
    }
    let (temp_binder, init) = bindings[0];
    // The init must be a call (the candidate self-call — membership is checked by the caller).
    if !matches!(core_of(db, init), Core::Call { .. }) {
        return None;
    }
    let Core::Tuple { elems } = core_of(db, body) else {
        return None;
    };
    // Each tuple element must be `(. temp i)` in order — a full identity repackage of the bound temp.
    for (i, elem) in elems.iter().enumerate() {
        let Core::Proj { operand, index } = core_of(db, *elem) else {
            return None;
        };
        if index != i {
            return None;
        }
        // `operand` must resolve to a reference to THIS let's binder (not some other tuple).
        match core_of(db, operand) {
            Core::LocalRef { binder } if binder == temp_binder => {}
            _ => return None,
        }
    }
    Some(init)
}

/// Whether the body at `id` makes a tail call to any def in `members` through the tail positions the
/// loop transform HANDLES — the body itself, an `if`'s two branches, a `let`'s body, or a `match`'s arm
/// bodies. NOT a non-tail position (an operand — that is a non-tail call). Mirrors `emit_tail`'s
/// propagation for exactly the `Call`/`If`/`Let`/`Match` cases so detection and emission agree. For a
/// plain self-loop `members = [self_def]`; for a mutual group it is every member (a tail call to any of
/// them iterates the shared loop).
pub(super) fn body_has_member_tail_call(db: &mut Db, id: StructId, members: &[usize]) -> bool {
    match core_of(db, id) {
        Core::Call { callee, .. } => members.contains(&callee),
        Core::If { then_, else_, .. } => {
            body_has_member_tail_call(db, then_, members)
                || body_has_member_tail_call(db, else_, members)
        }
        Core::Let { bindings, body } => {
            // MULTI-VALUE-UPGRADE tail: `(let ((t (member-call …))) (tuple (. t 0) …))` is an identity
            // repackage of a self-call — a genuine tail edge the effect upgrade obscured (see
            // `multivalue_repackage_tail_call`). Treat it as a member tail-call so an observed-out-state
            // performer still loops (else it recurses per iteration and exhausts the wasm stack).
            if let Some(call) = multivalue_repackage_tail_call(db, id)
                && let Core::Call { callee, .. } = core_of(db, call)
                && members.contains(&callee)
            {
                return true;
            }
            // Match `emit_tail`: a `let` keeps its body's tail position only when no heap drop is pending
            // (a drop after the body would fall back to non-tail `emit`). A scalar-only `let` (the loop
            // shapes) has no drop, so this simply recurses the body.
            let any_drop = bindings.iter().any(|(binder, _)| {
                is_heap_type(&type_of(db, *binder)) && !binding_escapes(db, body, *binder, false)
            });
            !any_drop && body_has_member_tail_call(db, body, members)
        }
        // A `match`'s arm bodies are tail positions (the probe chain threads the loop context into each),
        // so a member tail-call in any arm makes the function loopable. (A guard is NOT a tail position —
        // it is a predicate evaluated before the body, so it is not considered here.)
        Core::Match { arms, .. } => arms
            .iter()
            .any(|a| body_has_member_tail_call(db, a.body, members)),
        // A LIST match's arm bodies are tail positions too — `emit_tail` threads the loop context into
        // each (a tail self-call in a `(list …)` arm iterates the loop), so a member tail-call in any arm
        // makes the function loopable. This is what lets a tail list fold `(sa xs acc) = (match xs ((list)
        // acc) ((list x .. rest) (sa rest (+ acc x))))` become a constant-stack loop.
        Core::MatchList { arms, .. } => arms
            .iter()
            .any(|a| body_has_member_tail_call(db, a.body, members)),
        // A SUM match's decision tree has tail positions at its LEAF/GUARDED bodies — `emit_tail` threads
        // the loop context into each (a tail self-call in a `(Succ m) → (count m …)` arm iterates the
        // loop), so a member tail-call in any leaf makes the function loopable. This is what lets a
        // tail-recursive sum-type consumer `(count n acc) = (match n ((Zero) acc) ((Succ m) (count m (+
        // acc 1))))` become a constant-stack loop.
        Core::MatchSum { root, .. } => sum_cont_has_member_tail_call(db, &root, members),
        _ => false,
    }
}

/// The `body_has_member_tail_call` recursion over a sum decision tree ([`SumCont`]): a `Leaf`/`Guarded`
/// BODY is a tail position (a member tail-call there loops); the `Guarded.els`, `LitTest.then_`/`els`, and
/// `Switch` arm continuations are the remaining sub-matrix, all in the same tail position, so recurse
/// through them. The guard `cond` / literal `probe` are predicates evaluated BEFORE the body, not tail
/// positions, so they are not considered.
pub(super) fn sum_cont_has_member_tail_call(
    db: &mut Db,
    cont: &crate::core::SumCont,
    members: &[usize],
) -> bool {
    match cont {
        crate::core::SumCont::Leaf(body) => body_has_member_tail_call(db, *body, members),
        crate::core::SumCont::Guarded { body, els, .. } => {
            body_has_member_tail_call(db, *body, members)
                || sum_cont_has_member_tail_call(db, els, members)
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            sum_cont_has_member_tail_call(db, then_, members)
                || sum_cont_has_member_tail_call(db, els, members)
        }
        crate::core::SumCont::Switch { arms, .. } => arms
            .iter()
            .any(|a| sum_cont_has_member_tail_call(db, &a.cont, members)),
    }
}

/// The def indices called in TAIL position from the body at `id` — the recursion edges the loop
/// transform can turn into a `br`. Descends exactly the tail positions `emit_tail` propagates through
/// (`if` branches, `let` body without a pending drop, `match` arms); a call in a NON-tail position (an
/// operand) is NOT a tail edge (it must stay a real call) and is skipped. This is the tail-call analogue
/// of `body_has_member_tail_call`, collecting the callees rather than testing one set.
pub(super) fn tail_callees(db: &mut Db, id: StructId, out: &mut Vec<usize>) {
    match core_of(db, id) {
        Core::Call { callee, .. } if !out.contains(&callee) => out.push(callee),
        Core::Call { .. } => {}
        Core::If { then_, else_, .. } => {
            tail_callees(db, then_, out);
            tail_callees(db, else_, out);
        }
        Core::Let { bindings, body } => {
            // MULTI-VALUE-UPGRADE tail (see `multivalue_repackage_tail_call` + `body_has_member_tail_call`):
            // the identity-repackage `let` IS a tail call to the bound callee — collect it as a tail edge
            // so `mutual_loop_group` includes the self-recursion in the loop SCC.
            if let Some(call) = multivalue_repackage_tail_call(db, id)
                && let Core::Call { callee, .. } = core_of(db, call)
                && !out.contains(&callee)
            {
                out.push(callee);
                return;
            }
            let any_drop = bindings.iter().any(|(binder, _)| {
                is_heap_type(&type_of(db, *binder)) && !binding_escapes(db, body, *binder, false)
            });
            if !any_drop {
                tail_callees(db, body, out);
            }
        }
        Core::Match { arms, .. } => {
            for arm in arms {
                tail_callees(db, arm.body, out);
            }
        }
        Core::MatchList { arms, .. } => {
            for arm in arms {
                tail_callees(db, arm.body, out);
            }
        }
        Core::MatchSum { root, .. } => sum_cont_tail_callees(db, &root, out),
        _ => {}
    }
}

/// The `tail_callees` recursion over a sum decision tree ([`SumCont`]): collect the callees in TAIL
/// position (the `Leaf`/`Guarded` bodies), descending the same continuations `sum_cont_has_member_tail_call`
/// tests. The tail-call analogue of that predicate.
pub(super) fn sum_cont_tail_callees(
    db: &mut Db,
    cont: &crate::core::SumCont,
    out: &mut Vec<usize>,
) {
    match cont {
        crate::core::SumCont::Leaf(body) => tail_callees(db, *body, out),
        crate::core::SumCont::Guarded { body, els, .. } => {
            tail_callees(db, *body, out);
            sum_cont_tail_callees(db, els, out);
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            sum_cont_tail_callees(db, then_, out);
            sum_cont_tail_callees(db, els, out);
        }
        crate::core::SumCont::Switch { arms, .. } => {
            for arm in arms {
                sum_cont_tail_callees(db, &arm.cont, out);
            }
        }
    }
}

/// The wasm value types of def `d`'s parameters, in order — its machine SIGNATURE. `None` if any
/// parameter type has no machine representation (that def can't be a loop member). Two defs share a
/// signature (the requirement for a shared mutual loop, which reuses one set of parameter slots) iff
/// their `sig_valtypes` are equal.
pub(super) fn sig_valtypes(db: &mut Db, d: usize) -> Option<Vec<ValType>> {
    crate::layout::def_params(db, d)
        .iter()
        .map(|(_, ty)| valtype_of(ty))
        .collect()
}

/// The TAIL-RECURSIVE LOOP GROUP that def `self_def` belongs to — the set of defs compiled into ONE
/// shared `loop`. Returns `[self_def]` for plain self-recursion (a single-member loop, no dispatch), a
/// LARGER set for a mutually-tail-recursive group of SAME-SIGNATURE functions (`even`/`odd`), or empty
/// when `self_def` is not tail-recursive at all (so it stays ordinary `return_call`s).
///
/// The group is the strongly-connected component of `self_def` in the TAIL-call graph, restricted to
/// members that (a) share `self_def`'s machine signature — the shared loop reuses one set of parameter
/// slots, so members must agree on arity and per-slot type — and (b) are reachable in a tail cycle back
/// to `self_def`. A def whose signature differs, or that only calls `self_def` NON-tail, is excluded (a
/// non-tail call must stay a real call; a differing signature can't share the frame). Deterministic:
/// members are returned with `self_def` first, the rest in ascending def order, so the emitted `which`
/// discriminants are stable across runs.
///
/// MEMOIZED across a whole GROUP: `select_function_of` calls this for EVERY def, and the body is a
/// double BFS over the tail-call graph (forward reach + a reach-back-to-self per member), so a group of
/// N mutually tail-recursive same-signature defs cost O(N²) per def → O(N³) over the group (measured:
/// 200 mutual defs = 687ms before this). Every member of one SCC produces the SAME member SET (differing
/// only in the `self_def`-first ordering), so the expensive set is computed ONCE and cached by the
/// group's canonical representative (its minimum member index) — the N members of a group then share
/// that one computation, and each derives its self-first order cheaply. Keying by `self_def` directly
/// would MISS (each def is queried once), so the cache keys on the SORTED set's min element.
pub(super) fn mutual_loop_group(db: &mut Db, self_def: usize) -> Vec<usize> {
    let sorted = mutual_loop_members_sorted(db, self_def);
    // Reorder to this member's view: `self_def` first (it enters the loop at its own discriminant), the
    // rest ascending. `sorted` is already ascending, so this is a cheap rotate of `self_def` to front.
    if sorted.len() <= 1 {
        return sorted; // a plain self-loop (or empty) needs no reorder
    }
    let mut members = Vec::with_capacity(sorted.len());
    members.push(self_def);
    members.extend(sorted.iter().copied().filter(|&d| d != self_def));
    members
}

/// The SORTED member set of `self_def`'s tail-recursive SCC (ascending; empty if not a loop). Cached
/// PER MEMBER: since every member of one group produces the SAME sorted set, the first member to be
/// queried computes it (the O(N²) BFS) and then caches it for EVERY member of the group at once — so
/// the other N-1 members hit the cache and never recompute. That collapses the group's total cost from
/// O(N³) to O(N²) (one compute) + O(N) lookups. A non-loop def caches its own empty set.
pub(super) fn mutual_loop_members_sorted(db: &mut Db, self_def: usize) -> Vec<usize> {
    if let Some(cached) = db.mutual_loop_cache.get(&self_def) {
        return cached.clone();
    }
    let sorted = mutual_loop_group_uncached(db, self_def);
    // Cache for EVERY member of the discovered group (they all share this set) — so a co-member queried
    // later is an O(1) hit, not another O(N²) BFS. A non-loop def (empty set) caches just itself.
    if sorted.is_empty() {
        db.mutual_loop_cache.insert(self_def, Vec::new());
    } else {
        for &m in &sorted {
            db.mutual_loop_cache.insert(m, sorted.clone());
        }
    }
    sorted
}

/// The uncached core — computes the SORTED SCC member set (ascending), see [`mutual_loop_group`] docs.
pub(super) fn mutual_loop_group_uncached(db: &mut Db, self_def: usize) -> Vec<usize> {
    let Some(self_sig) = sig_valtypes(db, self_def) else {
        return Vec::new();
    };
    // Forward tail-reachability from `self_def`, staying within same-signature defs. A def enters the
    // frontier only if it shares the signature (else the edge can't be a shared-loop iteration).
    let mut reach: Vec<usize> = vec![self_def];
    let mut i = 0;
    while i < reach.len() {
        let d = reach[i];
        i += 1;
        let Some(body) = db.defs[d].body else {
            continue;
        };
        let mut callees = Vec::new();
        tail_callees(db, body, &mut callees);
        for c in callees {
            if !reach.contains(&c) && sig_valtypes(db, c).as_ref() == Some(&self_sig) {
                reach.push(c);
            }
        }
    }
    // Keep only the members that tail-reach BACK to `self_def` (a genuine cycle) — the SCC. A def in
    // `reach` that never tail-calls back is a one-way tail callee (a helper `self_def` tail-calls but
    // which does not recurse into the group); it is not part of the loop and stays a `return_call`.
    // `self_def` is always in (it seeds the group; a lone `self_def` with a self-edge loops as before,
    // and even without one an empty group falls through to no-loop via the `loops` check upstream).
    let mut members: Vec<usize> = reach
        .iter()
        .copied()
        .filter(|&d| d == self_def || tail_reaches(db, d, self_def, &reach))
        .collect();
    // Deterministic order: `self_def` first (this function enters the loop at its own discriminant),
    // the rest ascending — so the emitted `which` discriminants are stable. (Discriminants are LOCAL to
    // each member function's own loop, so `self`-first differing per function is fine — control never
    // crosses between the two functions' loops.)
    members.sort_unstable();
    members.retain(|&d| d != self_def);
    members.insert(0, self_def);
    // A single member is a plain self-loop ONLY if it actually self-tail-calls; otherwise no loop.
    if members.len() == 1 {
        let body = match db.defs[self_def].body {
            Some(b) => b,
            None => return Vec::new(),
        };
        if body_has_member_tail_call(db, body, &members) {
            return members;
        }
        return Vec::new();
    }
    members
}

/// Whether def `from` tail-reaches `target` within the candidate set `within` (a path of tail calls,
/// each hop staying inside `within`). Used to keep only the SCC members in `mutual_loop_group`.
pub(super) fn tail_reaches(db: &mut Db, from: usize, target: usize, within: &[usize]) -> bool {
    let mut seen: Vec<usize> = vec![from];
    let mut i = 0;
    while i < seen.len() {
        let d = seen[i];
        i += 1;
        let Some(body) = db.defs[d].body else {
            continue;
        };
        let mut callees = Vec::new();
        tail_callees(db, body, &mut callees);
        for c in callees {
            if c == target {
                return true;
            }
            if within.contains(&c) && !seen.contains(&c) {
                seen.push(c);
            }
        }
    }
    false
}
