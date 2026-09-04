//! `lower::match_desugar` — match & pattern lowering/desugaring, split out of `lower.rs`. Covers the
//! match-into-if / match-into-match fold rewrites, `lower_match` and the list/map/ctor match lowerings
//! (`lower_match_list`/`lower_match_map`), the SROA tuple-scrutinee destructure, and the family of
//! `desugar_*_list_elements` / `desugar_*_map_*` refutable/saturating pattern desugarings plus their
//! clone/predicate helpers. Behaviour-preserving move: items are reached across the `lower` module tree
//! via `use super::*` (the parent glob-re-exports every sibling), so the call graph is unchanged.

use super::*;

/// MATCH-INTO-IF: `(match (if c A B) arms)` where BOTH branches `A`/`B` reduce to a compile-time-visible
/// COMPOUND (a `Core::SumNew` constructor or a `Core::Tuple`) → `(if c (match A arms) (match B arms))`,
/// pushing the match into each branch so each inner match FOLDS against its now-constant scrutinee (the
/// existing direct-scrutinee `SumNew`/`Tuple` collapse in `lower_match_sum`/`build_tree`). The match
/// analogue of the `PROJECTION-INTO-IF` / `MEMBER-INTO-IF` folds (a tuple/record built through an `if`,
/// projected/read back, never reaches the heap): a SUM (or tuple) built through an `if` and immediately
/// deconstructed by a `match` builds and tears down a THROWAWAY heap value PER branch (`arr-alloc` + box
/// each payload + the deconstruct's `arr-get`/unbox), purely to read the payload back out. Pushing the
/// match in lets each branch's constant constructor fold to the arm body directly — `(match (if c (Some x)
/// (None)) ((Some v) v) ((None) 0))` → `(if c x 0)`, zero heap.
///
/// SOUND unconditionally (the fold is a PROFITABILITY gate, not a correctness one): `(if c A B)` evaluates
/// `c` then the SELECTED branch to a value then matches it; `(if c (match A arms) (match B arms))` evaluates
/// `c` then the selected inner match, which evaluates that SAME branch then matches — identical value, same
/// only-the-selected-branch evaluation, and `c`'s trap is preserved either way (it is evaluated first in
/// both). The `arms` appear in BOTH branches, so each branch gets its OWN deep COPY (`clone_subtree_db` — a
/// single node cannot have two parents; `push_list` reparents), each re-resolved so its payload binders
/// re-bind against THAT branch's scrutinee (a reused arm's `SumPayload` still points at the OLD `if`
/// scrutinee). `reduce_to_if` stops at a kept multi-use `if`-binding, so a shared scrutinee is left as a
/// runtime match. Returns `None` (fall through to the ordinary match lowering) when the scrutinee is not
/// such an `if`, or a branch does not reduce to a visible compound (then the rewrite would not fold — just
/// duplicate the match and grow code).
pub(super) fn fuse_match_into_if(
    db: &mut Db,
    scrutinee: StructId,
    arms: &[(StructId, StructId)],
) -> Option<Core> {
    let (cond, then_, else_) = crate::eval::reduce_to_if(db, scrutinee)?;
    // PROFITABILITY: fire only when each branch reduces to a compile-time-visible compound the inner match
    // FOLDS (a `Core::SumNew` ctor or a `Core::Tuple`). Otherwise the pushed-in matches stay runtime — the
    // rewrite would just duplicate the match over two branches, growing code for no fold. (A poison branch
    // falls through here too — the ordinary path reports it once.)
    let branch_folds = |db: &mut Db, b: StructId| {
        matches!(
            core_of(db, b),
            crate::core::Core::SumNew { .. } | crate::core::Core::Tuple { .. }
        )
    };
    if !branch_folds(db, then_) || !branch_folds(db, else_) {
        return None;
    }
    // PROFITABILITY CAP (compile-time DoS guard): the fusion deep-CLONES every arm (pattern + body) into
    // BOTH branches, and `core_of` on each pushed-in match re-enters `fuse_match_into_if` on the clone — so
    // when an arm body is itself a large/nested match, each level duplicates the deeper nest into two
    // branches, giving O(N²)+ synthesized nodes on a deeply-nested same-scrutinee match (v-inference-profiled
    // ~O(N^3.7): a `(match (if c (A n) (B 0)) ((A v) <N-deep same match>) …)` explodes). The fold's win is
    // eliminating ONE throwaway heap build per branch — bounded and small — so once the material to clone is
    // large the duplication cost dwarfs it. Bail to the ordinary runtime match when the total arm subtree
    // size exceeds a generous cap (the common small fold — `(Some x)`/`(None)` arms — is far under it). The
    // cap-walk is O(cap), so this guard never itself contributes to the blowup.
    const FUSE_ARM_SIZE_CAP: usize = 256;
    let mut arm_size = 0usize;
    for &(pat, body) in arms {
        arm_size += ast_subtree_size_capped(db, pat, FUSE_ARM_SIZE_CAP - arm_size);
        if arm_size >= FUSE_ARM_SIZE_CAP {
            break;
        }
        arm_size += ast_subtree_size_capped(db, body, FUSE_ARM_SIZE_CAP - arm_size);
        if arm_size >= FUSE_ARM_SIZE_CAP {
            break;
        }
    }
    if arm_size >= FUSE_ARM_SIZE_CAP {
        trace!(target: "rcdzc::fold", scrutinee = scrutinee.0, arm_size, "match-into-if fusion declined: arm material exceeds the clone-duplication profitability cap (keeping the runtime match)");
        return None;
    }
    // The ORIGINAL match form (scrutinee's parent) — the rewritten `if` grafts UNDER it so a free name
    // inside a cloned arm body (an enclosing param/`let`) ascends through it to the enclosing scope.
    let orig_match = db.parent_of(scrutinee);
    // Build `(match <branch> <cloned-arms>)` for one branch: each arm is a fresh DEEP copy (the arms appear
    // in both branches, and a reused arm's binders resolve to the OLD if-scrutinee). The branch occurrence
    // itself is REUSED (it appears once, in its own branch) — its `core_of` already folds to the visible
    // ctor, and the following `forget`+`resolve` re-resolves it in place under the new match.
    let build_branch_match = |db: &mut Db, branch: StructId| -> StructId {
        let match_head = db.push_name("match");
        let mut items = vec![match_head, branch];
        for &(pat, body) in arms {
            // The arm binders of the match BEING FUSED (their `SumPayload` reads `scrutinee`) must COPY so
            // they re-resolve against this branch; an ENCLOSING match's binder captured in the arm shares.
            let pat_c = clone_subtree_db_for_fused(db, pat, Some(scrutinee));
            let body_c = clone_subtree_db_for_fused(db, body, Some(scrutinee));
            items.push(db.push_list(vec![pat_c, body_c]));
        }
        db.push_list(items)
    };
    let then_match = build_branch_match(db, then_);
    let else_match = build_branch_match(db, else_);
    let if_head = db.push_name("if");
    let rewritten = db.push_list(vec![if_head, cond, then_match, else_match]);
    // GRAFT the rewritten `if` UNDER the ORIGINAL match node (its parent POINTER = the original match, at the
    // scrutinee's former child slot) — the same discipline the `handle`-reduce fold (E1c) uses. Ascent from a
    // free name in a cloned arm body then climbs `rewritten` → original-match → the enclosing binder (`def`/
    // `fn`/`let`), which recognizes the original match as the child it recorded (its `body_occ`) — so the
    // enclosing param resolves. Reparenting to the match's PARENT instead would present `rewritten` as the
    // body child, which the `def`/`fn` body-identity check (Case 4/3, identity-only — unlike `let` Case 1) would
    // reject → the CDZ0101 an enclosing-param arm reference hit.
    if let Some(orig) = orig_match {
        db.reparent(rewritten, Some(orig), db.child_ix_of(scrutinee) as u32);
    }
    // Lower each inner match via `core_of`, which resolves LAZILY on demand — like the `PROJECTION-INTO-IF`
    // mirror fold, which builds a `Core::If` over reused occurrences and NEVER calls `resolve_subtree`.
    // Crucially we do NOT re-resolve the tree: the REUSED `cond`/`then_`/`else_` occurrences come from
    // `reduce_to_if`, which may β-reduce a CALL scrutinee (`(bump n)` → `(if … (N.I n) …)`) — their free names
    // were bound by that β-SUBSTITUTION, not lexically, and are already resolve-PINNED, so an eager re-resolve
    // would re-derive them lexically and report the substituted name unbound (CDZ0101). The FRESH synthesized
    // nodes (the inner matches + their CLONED arms) are unmemoized, so `core_of`→`resolved_of` resolves each
    // on demand; each cloned arm's payload binder resolves against THAT branch's constant scrutinee.
    let then_core = core_of(db, then_match);
    let else_core = core_of(db, else_match);
    // COMMIT the fusion ONLY when BOTH inner matches FULLY FOLD to a non-match Core — i.e. the constant
    // scrutinee statically selected an arm and the sum was truly ELIMINATED. That is at once the
    // PROFITABILITY condition (a branch that stays a residual `Core::Match*` still builds + tears down the
    // sum on the heap — no win, just a duplicated match) AND the SAFETY condition: the constant-scrutinee
    // decision path (`fold_sum_path`) has coverage GAPS the runtime decision-tree does not — a deeply-NESTED
    // constant pattern with a DEAD arm of unconstrained payload type (`(Some (Ok (C.R n)))` + a dead `Err e`)
    // folds to the live body but leaves a residual switch whose dead sub-path declines at EMIT ("dispatches
    // on a non-sum sub-value") — a decline the original runtime match never hits. A residual match (or a
    // poison) → BACK OUT (`None`) and let the caller lower the ORIGINAL runtime match unchanged.
    let fully_folded = |c: &Core| {
        !matches!(
            c,
            crate::core::Core::Poison(_)
                | crate::core::Core::MatchSum { .. }
                | crate::core::Core::MatchList { .. }
                | crate::core::Core::Match { .. }
        )
    };
    if !fully_folded(&then_core) || !fully_folded(&else_core) {
        trace!(target: "rcdzc::fold", scrutinee = scrutinee.0, "match-into-if fusion backed out (a branch did not fully fold); keeping the runtime match");
        return None;
    }
    trace!(target: "rcdzc::fold", scrutinee = scrutinee.0, "match pushed into an if of constant constructors (no heap build)");
    Some(core_of(db, rewritten))
}

/// CASE-OF-MATCH — the direct twin of [`fuse_match_into_if`] over a MATCH scrutinee instead of an `if`:
/// `(match (match s (p0 A) (p1 B)…) outer_arms)` where each INNER arm body `A`/`B` builds a compile-time-
/// visible constructor pushes the OUTER match INTO each inner arm body →
/// `(match s (p0 (match A outer_arms)) (p1 (match B outer_arms))…)`. Each pushed-in `(match A outer_arms)`
/// then folds against `A`'s constant constructor (the same construct-then-match collapse `fuse_match_into_if`
/// relies on), so the THROWAWAY sum the inner match built per arm — purely to be deconstructed by the outer
/// match — never reaches the heap: `(match (match (> n 0) (true (Some n)) (false (None))) ((Some v) v)
/// ((None) 0))` → `(match (> n 0) (true n) (false 0))` → `(if (> n 0) n 0)`, zero heap.
///
/// SOUND (the fold is a PROFITABILITY gate, not a correctness one): the inner match evaluates `s`, selects
/// ONE arm, evaluates its body `A` to a value, then the outer match deconstructs it; the rewritten form
/// evaluates `s`, selects the SAME arm, then its body is `(match A outer_arms)` — evaluates `A` then matches
/// it — identical value, identical only-the-selected-arm evaluation, `s`'s trap preserved (evaluated first
/// either way). The INNER arm PATTERNS are reused verbatim (their binders stay in scope for the rewritten
/// body — the same discipline the case-of-match-in-APPLY-head rewrite uses), and `outer_arms` appear in
/// every inner arm body so each inner arm gets its OWN deep COPY (`clone_subtree_db` — a node has one
/// parent). Commit ONLY when EVERY pushed-in inner match FULLY FOLDS to a non-match Core (sum truly
/// eliminated = the win, and it sidesteps the constant-scrutinee decision path's coverage gaps exactly as
/// the `if` twin does); any residual/poison → BACK OUT (`None`) to the ordinary runtime match. Returns
/// `None` when the scrutinee is not a reducible match, or an inner arm is malformed, or a fold check fails.
pub(super) fn fuse_match_into_match(
    db: &mut Db,
    scrutinee: StructId,
    arms: &[(StructId, StructId)],
) -> Option<Core> {
    // Only fire on a DIRECTLY-VISIBLE inner match — the scrutinee is literally a `(match …)` form, or a
    // ref/annotation chain to one — NOT a match reached by β-reducing an APPLICATION. `reduce_to_match`
    // would β-reduce a call scrutinee (`(step s)`), but for a CONST-CLOSURE / specialization-sensitive
    // callee in a still-generic driver body that β-reduce disturbs the const-specialization path (a
    // partially-applied `step` mis-resolves → a spurious CDZ0201 "`Option` is a type, not a function").
    // The if-twin never hits this (a call scrutinee is not an `if`); the win case here is always a
    // syntactically-present inner match, so the narrower `reduce_to_match_direct` (Ref/Annot only, no Apply)
    // covers the gap without touching a call scrutinee.
    let inner_form = crate::eval::reduce_to_match_direct(db, scrutinee)?;
    let mtail = db.ast.as_form(inner_form, "match").map(<[_]>::to_vec)?;
    let [inner_scrut, inner_arm_occs @ ..] = mtail.as_slice() else {
        return None;
    };
    let inner_scrut = *inner_scrut;
    if inner_arm_occs.is_empty() {
        return None;
    }
    // PROFITABILITY pre-check: every inner arm's body must fold to a compile-time-visible compound the
    // pushed-in outer match can collapse (a `Core::SumNew` ctor or a `Core::Tuple`). A single runtime-valued
    // inner arm body means the rewrite would just duplicate the outer match with no fold — bail before any
    // synthesis. (A poison body bails here too; the ordinary path reports it once.)
    let mut inner_arms: Vec<(StructId, StructId)> = Vec::with_capacity(inner_arm_occs.len());
    for &arm in inner_arm_occs {
        let (pat, body) = match db.ast.get(arm) {
            crate::ast::Struct::List(kv) if kv.len() == 2 => (kv[0], kv[1]),
            _ => return None,
        };
        if !matches!(
            core_of(db, body),
            crate::core::Core::SumNew { .. } | crate::core::Core::Tuple { .. }
        ) {
            return None;
        }
        inner_arms.push((pat, body));
    }
    // The ORIGINAL outer match form (scrutinee's parent) — the rewritten inner match grafts UNDER it so a
    // free name in a cloned outer-arm body ascends through it to the enclosing scope (same rationale as the
    // `if` twin's graft-under-orig).
    let orig_match = db.parent_of(scrutinee);
    // Build the rewritten inner match: reuse each inner arm's PATTERN verbatim (its binders stay in scope for
    // the new body), and wrap its body as `(match <body> <cloned-outer-arms>)`. `outer_arms` appear in every
    // inner arm body, so each gets its OWN deep copy.
    let inner_head = db.push_name("match");
    let mut items = vec![inner_head, inner_scrut];
    for (pat, body) in inner_arms {
        let match_head = db.push_name("match");
        let mut inner_match = vec![match_head, body];
        for &(opat, obody) in arms {
            // Outer-match arm binders (their `SumPayload` reads the OUTER `scrutinee`) COPY to re-resolve
            // against the pushed-in branch; an enclosing match's captured binder shares.
            let opat_c = clone_subtree_db_for_fused(db, opat, Some(scrutinee));
            let obody_c = clone_subtree_db_for_fused(db, obody, Some(scrutinee));
            inner_match.push(db.push_list(vec![opat_c, obody_c]));
        }
        let pushed_body = db.push_list(inner_match);
        items.push(db.push_list(vec![pat, pushed_body]));
    }
    let rewritten = db.push_list(items);
    if let Some(orig) = orig_match {
        db.reparent(rewritten, Some(orig), db.child_ix_of(scrutinee) as u32);
    }
    // Lower LAZILY via `core_of` — do NOT `resolve_subtree` (the same discipline as the `if` twin). The
    // reused inner arm BODIES come from `reduce_to_match`, which may β-reduce a CALL/closure scrutinee
    // (`(step s)` for a const-closure driver → `(match s ((list) (None)) …)`) — their free names were bound
    // by β-SUBSTITUTION and are resolve-PINNED, so an eager `resolve_subtree` would re-derive them lexically
    // and mis-resolve (a specialized `Option.None`/`Option.Some` occurrence re-resolving standalone hit
    // "`Option` is a type, not a function"). The FRESH nodes (the outer `match` head, the cloned outer arms)
    // are unmemoized, so `core_of`→`resolved_of` resolves each on demand against its grafted parent; a cloned
    // outer-arm payload binder resolves against its inner-arm-body constant scrutinee. (The APPLY-head
    // case-of-match DOES `resolve_subtree` because its rewritten bodies `(f args…)` splice reused arg
    // subtrees needing re-resolution; here the reused parts are whole arm bodies kept intact.)
    // COMMIT only when EVERY inner arm's pushed-in outer match FULLY FOLDS (the sum eliminated). Re-lower each
    // inner arm's wrapped body and check; a residual `Core::Match*` / poison in any → BACK OUT to the runtime
    // match (strictly an opt — same fully-folds gate as the `if` twin).
    let rewritten_arms: Vec<StructId> = match db.ast.get(rewritten) {
        crate::ast::Struct::List(kv) => kv.iter().skip(2).copied().collect(),
        _ => return None,
    };
    for arm in rewritten_arms {
        // Each rewritten inner arm is `(pat (match body outer_arms…))`; re-lower its wrapped body and require
        // it fully folds (no residual `Core::Match*`, no poison) — else back out to the runtime match.
        let body = match db.ast.get(arm) {
            crate::ast::Struct::List(kv) if kv.len() == 2 => kv[1],
            _ => return None,
        };
        if matches!(
            core_of(db, body),
            crate::core::Core::Poison(_)
                | crate::core::Core::MatchSum { .. }
                | crate::core::Core::MatchList { .. }
                | crate::core::Core::Match { .. }
        ) {
            trace!(target: "rcdzc::fold", scrutinee = scrutinee.0, "case-of-match fusion backed out (an inner arm did not fully fold); keeping the runtime match");
            return None;
        }
    }
    trace!(target: "rcdzc::fold", scrutinee = scrutinee.0, "match pushed into a match of constant constructors (no heap build)");
    Some(core_of(db, rewritten))
}

pub(super) fn lower_match(db: &mut Db, scrutinee: StructId, arms: &[(StructId, StructId)]) -> Core {
    // A ZERO-ARM match is the DEGENERATE base case of exhaustiveness: it is well-formed ONLY when the
    // scrutinee is UNINHABITED (`Never` — a diverging expression), for which no arm is needed to cover
    // every variant, there being none (`type-system.md §Never Is The Empty Sum`, 4th sentence). The
    // scrutinee still evaluates (its divergence IS the match's outcome), so lower it: a scrutinee that
    // provably DIVERGES lowers to `Core::Trap` / a poison — return that (the match diverges through the
    // scrutinee, exactly as `(match (never-returns))` traps). A zero-arm match on an INHABITED scrutinee
    // is genuinely non-exhaustive (`Code::NonExhaustive`), NOT the malformed "no arms" it was before.
    if arms.is_empty() {
        // The scrutinee's TYPE is the EMPTY SUM (zero variants) — an uninhabited `Void`/`Never`, e.g. a
        // parameter `(: v Void)`. `type-system.md §Never Is The Empty Sum` (4th sentence): "A match on a
        // scrutinee of the empty sum type MUST be exhaustive with zero arms." No value of that type can
        // exist, so the match is unreachable: emit `Core::Trap` (the scrutinee still evaluates — reading
        // it is itself the divergence — but no arm is needed). Distinct from the diverging-EXPRESSION case
        // below (a scrutinee that folds to a trap): here the scrutinee's static TYPE proves uninhabited.
        //= spec/capabilities/type-system.md#never-is-the-empty-sum
        //# A match on a scrutinee of the empty sum type MUST be exhaustive with zero arms, because there is no variant left to cover — the degenerate base case of the exhaustiveness rule, not an exception to it.
        let scrut_ty = crate::infer::type_of(db, scrutinee);
        if is_empty_sum_ty(db, &scrut_ty) {
            return Core::Trap;
        }
        return match core_of(db, scrutinee) {
            c @ (Core::Trap | Core::Poison(_)) => c,
            // An inhabited scrutinee with NO arms is the degenerate non-exhaustive case: EVERY variant is
            // uncovered. When the scrutinee is a SUM or a nominal (its decl carries the variant set), reuse
            // `non_exhaustive_sum_reject` with an empty `tested` set — so a zero-arm match gets the SAME
            // "add the missing arms" fix a partially-covered one does (the full variant set as
            // `(V (trap "TODO: V"))` arms), not just a dead-end message. A non-sum inhabited scrutinee (a
            // bare scalar with zero arms) is covered by a single wildcard `_` arm, so carry that "add a `_`
            // (trap TODO) arm" fix (the open-sum analogue) — both for the actionable route to a fix and,
            // critically, so a CALLED def's zero-arm scalar match dedups: the def-body copy's fix edits the
            // real (user) match form while the inlined call-site copy's fix targets the SYNTHESIZED reduced
            // match, and `dedup_faults`' `user_fix_keys` rule drops the non-user copy (a fix-LESS reject
            // gave that rule no discriminator, so both leaked as two `error:` lines for one defect). Falls
            // back to the fix-less message when the match form has no parent node (defensive; unreachable
            // for a real `(match x)`).
            _ => match &scrut_ty {
                crate::ty::Ty::Sum { decl, .. } | crate::ty::Ty::Nominal { decl, .. } => {
                    Core::Poison(non_exhaustive_sum_reject(db, *decl, &[], scrutinee))
                }
                _ => {
                    let reject = Reject::coded(
                        Code::NonExhaustive,
                        "a zero-arm match is exhaustive only on an uninhabited (Never) scrutinee; this scrutinee has values a case must cover",
                    );
                    Core::Poison(match db.parent_of(scrutinee) {
                        Some(match_form) => reject.with_fix(Fix::insert_arms_heuristic(
                            match_form,
                            vec!["(_ (trap \"TODO\"))".to_string()],
                        )),
                        None => reject,
                    })
                }
            },
        };
    }
    // A SUBJECT that fails to RESOLVE — an unbound name (CDZ0101) or an unresolved module member
    // (CDZ0201) — is a real error that must report AS ITSELF, ahead of any arm-pattern-support check.
    // Detect it at the RESOLVE stage (`resolved_of` → `Resolved::Poison` carries the coded reject), NOT by
    // LOWERING (`core_of`). This is the narrowing over #8391/#8399: firing on `core_of(scrutinee)` being a
    // `Core::Poison` was too broad — merely CALLING `core_of` on a VALID scrutinee here lowers it EARLY, in
    // a bare context/order, and that side-effect (a memoized wrong-context core) corrupted a runtime `?`
    // subject (a value MISCOMPILE — corpus-23 `tdd1`, got 0 not -1) and a Map-accumulator subject
    // (corpus-28, regressed to a decline), even though the guard did not even fire on them. `resolved_of` is
    // the resolution query (runs anyway, no lowering), so a VALID subject (a `?`, a Map-accum, any runtime
    // value) is NOT `Resolved::Poison` and is left ENTIRELY to the normal path below — no early lowering, no
    // corruption. An unbound/unresolved subject IS `Resolved::Poison`, so its coded CDZ0101/CDZ0201 still
    // surfaces here (the masking fix — and the C1 exact-code guards — preserved). Without this, a poison
    // subject's TYPE is `Any`, matching none of the kind-dispatches below, so a STRUCTURAL/ctor arm falls to
    // the scalar-probe path and MASKS the subject error with the uncoded "not a scalar literal or `_`"
    // decline. The erroring-SUBJECT sibling of the wrong-KIND CDZ0203 scrutinee check below (breaker-found,
    // concierge-routed; #8391 → clobbered by #8393 → re-applied #8399 → narrowed here after #8399 over-fired).
    if let crate::resolved::Resolved::Poison(r) = resolved_of(db, scrutinee) {
        return Core::Poison(r);
    }
    // A FUNCTION-VALUED scrutinee — `(match g …)` where `g` is a closure/def — is not matchable: a match
    // deconstructs a DATA value by its cases (`core-semantics.md §Patterns Compose` — literals, tuples,
    // constructors), and a function is not data (it has no cases, no discriminant, no machine value to
    // probe). Without this it fell through to the scalar-probe path, which tried to lower `g` as a runtime
    // value and hit the closure-boundary DECLINE ("a closure's parameter type has no machine
    // representation") — an internal-sounding, uncoded message about a "parameter type" the author never
    // asked about. Reject it up front with the real cause, coded CDZ0203 (an ill-typed match: the scrutinee
    // is not a matchable value), so the diagnostic names what is wrong.
    if let crate::ty::Ty::Fn(_, _) = crate::infer::type_of(db, scrutinee) {
        return Core::Poison(Reject::coded(
            Code::TypeMismatch,
            "a function value cannot be matched — `match` deconstructs a data value by its cases \
             (a literal, a tuple, or a constructor), and a function has none; call it, or match on \
             the value it returns",
        ));
    }
    // MATCH-INTO-IF: `(match (if c A B) arms)` where both branches build a compile-time-visible compound →
    // `(if c (match A arms) (match B arms))`, so each inner match folds against its now-constant scrutinee
    // (the sum/tuple built through the `if` never reaches the heap). Tried BEFORE the compound-dispatch
    // below — the fused form fully replaces this match. A non-`if`/non-const-branch scrutinee returns `None`
    // and falls through unchanged.
    if let Some(fused) = fuse_match_into_if(db, scrutinee, arms) {
        return fused;
    }
    // CASE-OF-MATCH (the twin over a `match` scrutinee): `(match (match s (p0 A) (p1 B)…) arms)` where each
    // inner arm builds a visible constructor → push the outer match into each inner arm body so each folds
    // against its constant ctor (the throwaway sum the inner match built per arm never reaches the heap).
    // Same fully-folds back-out as the `if` twin. A non-match/non-const-arm scrutinee returns `None`.
    if let Some(fused) = fuse_match_into_match(db, scrutinee, arms) {
        return fused;
    }
    // A COMPOUND scrutinee — a SUM, a TUPLE, or a RECORD — is matched by the DECISION TREE, not the
    // scalar-probe path. A sum dispatches on the discriminant; a tuple has no discriminant, so its match
    // is a chain of `Elem`-path binders / literal tests; a RECORD has neither a discriminant NOR a
    // sanctioned destructuring pattern (a record is read by `(. r field)` projection, not pattern-matched
    // field-by-field — `core-semantics.md §Patterns Compose` lists tuple + constructor patterns, NOT
    // record patterns), so a record match's only patterns are a bare BINDER (binds the whole record) or a
    // WILDCARD — a degenerate match the tree folds to the first covering arm. All go through
    // `lower_match_sum` (the shared decision-tree builder); a scalar scrutinee falls through to the
    // scalar-probe path below.
    if let crate::ty::Ty::Sum { .. }
    | crate::ty::Ty::Nominal { .. }
    | crate::ty::Ty::Tuple(_)
    | crate::ty::Ty::Record(_) = crate::infer::type_of(db, scrutinee)
    {
        return lower_match_sum(db, scrutinee, arms);
    }
    // A STRUCTURAL pattern head — `(bin …)` / `(list …)` / `(map …)` / `(tuple …)` — decodes a value of a
    // SPECIFIC kind (Bytes / List / Map / Tuple respectively), so it is well-formed only over a scrutinee of
    // that kind. Over a DEFINITE scrutinee of a DIFFERENT kind — a `(bin …)` on an Int64, a `(list …)` on a
    // String, a `(map …)` on a List — it is a type error, but it was neither type-checked (each kind's
    // lowering branch below is gated on the matching scrutinee type, so a mismatched one falls through) nor
    // cleanly rejected (it reached the scalar/list path and declined with the misleading generic "a match
    // pattern that is not a scalar literal or `_` is not supported"). Name the real fault (CDZ0203), the
    // structural twin of the map-key / list-element pattern-type checks. (A Sum/Tuple/Record SCRUTINEE has
    // already returned to `lower_match_sum` above, which type-checks a `(list …)`/`(map …)` pattern against
    // it — so this only sees a SCALAR / Bytes / List / Map / Set scrutinee here, where the well-formed
    // pattern kind is determined by the scrutinee type below.) `Any`/`Var` (unsolved) is not a definite
    // mismatch — skip it (a runtime value of the right kind still flows in). Anchored at the offending arm.
    let scrut_ty = crate::infer::type_of(db, scrutinee);
    let scrutinee_kind_word = |t: &crate::ty::Ty| match t {
        crate::ty::Ty::Bytes => Some("bin"),
        crate::ty::Ty::List(_) => Some("list"),
        crate::ty::Ty::Map(..) => Some("map"),
        crate::ty::Ty::Tuple(_) => Some("tuple"),
        _ => None,
    };
    // The pattern's structural KIND word — recognized via `compound_form_of` for the compound ctors
    // (native `#list`/`#tuple`/`#map` ctor-leaf head + the name/string alias, M2/M3) so a NATIVE pattern is
    // checked here just like the name-head alias; `bin` is the binary-matching head (not a compound ctor),
    // read by name. Was `head_name(pat)` (name-alias only) — so a native `#tuple(a b)` pattern over a
    // non-Tuple scrutinee slipped this CDZ0203 kind-check and fell through to a misleading generic CDZ0201.
    let scrut_kind = scrutinee_kind_word(&scrut_ty);
    let bad = {
        let ast = &db.ast;
        arms.iter().find_map(|&(pat, _)| {
            let k = if ast
                .compound_form_of(pat, crate::ast::CompoundCtor::List)
                .is_some()
            {
                Some("list")
            } else if ast
                .compound_form_of(pat, crate::ast::CompoundCtor::Tuple)
                .is_some()
            {
                Some("tuple")
            } else if ast
                .compound_form_of(pat, crate::ast::CompoundCtor::Map)
                .is_some()
            {
                Some("map")
            } else if ast.head_name(pat) == Some("bin") {
                Some("bin")
            } else {
                None
            };
            k.filter(|w| Some(*w) != scrut_kind).map(|w| (pat, w))
        })
    };
    if !matches!(scrut_ty, crate::ty::Ty::Any | crate::ty::Ty::Var(_))
        && let Some((bad_pat, pat_head)) = bad
    {
        let expects = match pat_head {
            "bin" => "a Bytes value (binary matching)",
            "list" => "a List value",
            "map" => "a Map value",
            "tuple" => "a Tuple value",
            _ => "a value of a matching kind",
        };
        return Core::Poison(
            Reject::coded(
                Code::TypeMismatch,
                format!(
                    "a `({pat_head} …)` pattern matches {expects}, but this scrutinee is {}",
                    scrut_ty.render_name(&db.name_ctx())
                ),
            )
            .at(bad_pat),
        );
    }
    // A BYTES scrutinee whose arms use `(bin …)` binary patterns → the binary matcher (BN3, const
    // scrutinee). A scalar-only match over Bytes (only bare-binder/`_` arms) is NOT here — it has no
    // `(bin …)` arm, so it falls through to the scalar path (a whole-value binder / wildcard).
    if matches!(scrut_ty, crate::ty::Ty::Bytes)
        && arms.iter().any(|&(pat, _)| {
            // Peel a `(guard <inner> <cond>)` wrapper before reading the pattern head, so a GUARDED bin arm
            // `(guard (bin …) cond)` routes to the binary matcher too (§4b). A bare `(bin …)` reads `pat`
            // directly. (The list/map routes above key on the scrutinee TYPE, not the pattern head, so they
            // already admit a guarded arm; only the bin route reads the pattern head.)
            let inner = match db.ast.as_form(pat, "guard") {
                Some(g) if g.len() == 2 => g[0],
                _ => pat,
            };
            db.ast.head_name(inner) == Some("bin")
        })
    {
        return lower_match_bin(db, scrutinee, arms);
    }
    // A LIST scrutinee is deconstructed by ELEMENT patterns (`core-semantics.md` §A List Is Deconstructed
    // By Element Patterns With An Optional Rest). This increment folds a CONSTANT list (`Core::ListNew`)
    // against both FIXED-ARITY patterns `(list a b)` / `(list)` AND REST patterns `(list a .. rest)`: the
    // known length selects the matching arm (a fixed pattern of arity k matches length k, a rest pattern
    // of lead k matches length ≥ k); leading binders read the constant elements via `SumPayload` `Elem`
    // folds and the rest binder folds to the tail sublist. A RUNTIME list scrutinee declines (later
    // increment), as does a NESTED leading sub-pattern (each leading position must be a bare binder / `_`).
    if matches!(crate::infer::type_of(db, scrutinee), crate::ty::Ty::List(_)) {
        return lower_match_list(db, scrutinee, arms);
    }
    // A MAP scrutinee whose arms use `(map …)` key-directed patterns → the map matcher (ask-61). A map
    // pattern `(map (k p) … .. rest)` matches when the map HAS key `k` (bound to a value matching `p`),
    // binding `rest` to the remaining map — a QUERY, not a structural shape. `lower_match_map` handles BOTH
    // a CONSTANT `Core::MapNew` scrutinee (compile-time arm selection) AND a RUNTIME map (desugared to a
    // `Map.lookup` presence-test `if`-chain). A scalar-only match over a map (only bare-binder/`_` arms — no
    // `(map …)` pattern) falls through to the scalar path (a whole-value binder).
    if matches!(
        crate::infer::type_of(db, scrutinee),
        crate::ty::Ty::Map(_, _)
    ) && arms.iter().any(|&(pat, _)| {
        // Peel a `(guard (map …) cond)` wrapper so a GUARDED map arm routes to the map matcher too (the map
        // twin of the §4b bin routing peel). `lower_match_map` already peels the guard internally + the
        // runtime desugar threads the cond; the guard cond's value/rest binders resolve via Case 6mg.
        let inner = match db.ast.as_form(pat, "guard") {
            Some(g) if g.len() == 2 => g[0],
            _ => pat,
        };
        db.ast.compound_form_of(inner, CompoundCtor::Map).is_some()
    }) {
        return lower_match_map(db, scrutinee, arms);
    }
    // A SET scrutinee whose arms use `(set …)` element-membership patterns → the set matcher
    // (`core-semantics.md` §A Set Is Matched By Element-Membership Patterns). A set pattern `(set e… .. rest)`
    // matches when the set CONTAINS every named element (each an ordinary value expression, compared by the
    // set's own value equality), binding `rest` to the set MINUS the named elements — a membership QUERY, not
    // a structural shape (the KEYS-ONLY twin of the map matcher). `lower_match_set` desugars to a nested
    // `Set.contains` presence-test `if`-chain + a `Set.remove` residual `let`, working over BOTH a constant
    // `Core::SetOf` and a runtime set (`Set.contains`/`Set.remove` evaluate either way). A scalar-only match
    // over a set (only bare-binder/`_` arms — no `(set …)` pattern) falls through to the scalar path.
    if matches!(crate::infer::type_of(db, scrutinee), crate::ty::Ty::Set(_))
        && arms.iter().any(|&(pat, _)| {
            // Peel a `(guard (set …) cond)` wrapper so a GUARDED set arm routes here too (the set twin of the
            // map routing peel). `lower_match_set` peels the guard internally + threads the cond.
            let inner = match db.ast.as_form(pat, "guard") {
                Some(g) if g.len() == 2 => g[0],
                _ => pat,
            };
            db.ast.compound_form_of(inner, CompoundCtor::Set).is_some()
        })
    {
        return lower_match_set(db, scrutinee, arms);
    }
    // Classify each arm into a probe + optional GUARD + body. An arm's pattern may be a GUARDED pattern
    // `(guard <inner-pat> <cond>)` — the inner pattern gives the probe, `<cond>` the guard (a boolean the
    // arm's binder is in scope for, resolve Case 5). A pattern that is not a scalar literal, binder,
    // wildcard, or such a guarded pattern declines the whole match (a compound needs a heap walk).
    let mut probes: Vec<(crate::core::Probe, Option<StructId>, StructId)> = Vec::new();
    // The inner-pattern node for each probe, index-aligned with `probes` — kept alongside (rather than
    // widening the probe tuple, which several destructures below rely on) so the pattern-type check can
    // anchor its CDZ0201 at the offending PATTERN, not the enclosing match.
    let mut probe_pats: Vec<StructId> = Vec::new();
    for &(pat, body) in arms {
        let (inner_pat, guard) = match db.ast.as_form(pat, "guard") {
            // `(guard <inner-pat> <cond>)` — a guarded pattern.
            Some(g) if g.len() == 2 => (g[0], Some(g[1])),
            Some(g) => {
                // A wrong-arity `(guard …)` — anchored at the offending arm PATTERN, with a delete fix on
                // the FIRST surplus element when there are too many (the fixed-arity family's shared
                // surplus-delete; too few has nothing to delete → message only).
                return Core::Poison(crate::resolve::fixed_arity_reject(
                    pat,
                    g,
                    2,
                    "a guarded pattern must be (guard <pattern> <cond>)",
                ));
            }
            None => (pat, None),
        };
        match classify_probe(db, inner_pat) {
            Some(p) => {
                probes.push((p, guard, body));
                probe_pats.push(inner_pat);
            }
            None => {
                // A compound arm over a scrutinee that routed to the SCALAR path (a scalar scrutinee, or a
                // List/Map scrutinee with NO structural `(list …)`/`(map …)` arm so it fell through here).
                // The COMMON reach is a ctor-shaped head that is an UNBOUND name / non-member (`(Zorp x)`,
                // `(List.Cons h t)`) — propagate its CODED poison (CDZ0101/CDZ0201) so `cdz check` surfaces
                // it via `match_pattern_fault` on EVERY body, closing the same check≡compile hole the list
                // (Inc 39) and map matchers had (the emit-path lowering walk that produces this decline
                // runs on nullary-exported bodies alone). A genuinely not-yet-lowerable compound whose head
                // resolves cleanly (or is not a name) yields no coded poison → the honest uncoded decline
                // stands; no false alarm. `inner_pat` is already peeled of any `(guard …)` wrapper above.
                // A COMPOUND pattern (`(list …)`/`(tuple …)`/`(record …)`/`(map …)`/`(set …)`, in ANY
                // spelling — `compound_form_of` recognizes the native ctor-leaf head + the name/string alias)
                // is a VALID pattern head, not an unbound-ctor error — its head must NOT be lowered as a value
                // (a native ctor-LEAF head would spuriously poison CDZ0201 "compound-constructor head leaf is
                // not a value", the misdiagnosis that made a native `#tuple`/`#record` match over a non-definite
                // (Any) scrutinee reject while the name-alias declined cleanly). Only a NON-compound ctor-shaped
                // head (`(Zorp x)`, `(List.Cons h t)`) gets its coded poison propagated.
                let is_compound_pattern = [
                    crate::ast::CompoundCtor::List,
                    crate::ast::CompoundCtor::Tuple,
                    crate::ast::CompoundCtor::Record,
                    crate::ast::CompoundCtor::Map,
                    crate::ast::CompoundCtor::Set,
                ]
                .iter()
                .any(|&c| db.ast.compound_form_of(inner_pat, c).is_some());
                let ctor_head = match db.ast.get(inner_pat) {
                    crate::ast::Struct::List(children) => children.first().copied(),
                    _ => None,
                };
                if !is_compound_pattern
                    && let Some(head) = ctor_head
                    && let Core::Poison(reject) = core_of(db, head)
                    && reject.code.is_some()
                {
                    return Core::Poison(reject);
                }
                // A `#set(…)` element-membership pattern is now LOWERED by `lower_match_set` (routed above on
                // a `Ty::Set` scrutinee) — so reaching HERE with a set pattern means the scrutinee is NOT a
                // set (a set-typed scrutinee would have routed away). That is a shape mismatch, not an
                // unsupported construct: emit the CODED `CDZ0201` (`Code::Malformed`) anchored at the pattern
                // so `match_pattern_fault` surfaces it in `cdz check` on EVERY body (`check` ≡ `compile`, the
                // same property the list/map matchers keep) rather than a check-silent uncoded decline.
                if db
                    .ast
                    .compound_form_of(inner_pat, crate::ast::CompoundCtor::Set)
                    .is_some()
                {
                    return Core::Poison(
                        Reject::coded(
                            Code::Malformed,
                            "a `(set …)` pattern matches only a set scrutinee",
                        )
                        .at(inner_pat),
                    );
                }
                return Core::Poison(Reject::decline(
                    "a match pattern that is not a scalar literal or `_` is not supported",
                ));
            }
        }
    }
    // WELL-FORMEDNESS (checked STRUCTURALLY, before any fold — a constant scrutinee does not excuse a
    // type-mismatched pattern or a non-exhaustive match; a match is well-formed or not regardless of
    // what the scrutinee happens to be):
    let scrut_ty = crate::infer::type_of(db, scrutinee);
    //  (1) each LITERAL pattern's type must agree with the scrutinee's — a bool pattern against an
    //      integer scrutinee (or vice-versa) is a shape/type error (CDZ0201), not a never-matching arm.
    for (i, (probe, _, _)) in probes.iter().enumerate() {
        let pat_ty = match probe {
            crate::core::Probe::Int(_) => Some(crate::ty::Ty::int()),
            crate::core::Probe::Bool(_) => Some(crate::ty::Ty::Bool),
            // A `Str` probe comes from a String-literal `"…"` pattern OR a SYMBOL-literal `#"…"` pattern —
            // both lower to a `Core::ConstStr` (a symbol's identity is its text, shared rep), so ONE probe
            // serves both. But `String` and `Symbol` are a distinct NOMINAL boundary that `=` (CDZ0202) and
            // fn-args reject, so the expected type must come from the PATTERN's own kind, NOT the scrutinee:
            // resolve `probe_pats[i]` — a `SymbolConst` pattern expects `Symbol`, a `Str` pattern expects
            // `String`. Then `agrees_with` accepts the same-kind cases (`#"add"` over `Symbol`, `"add"` over
            // `String`) and FAULTS the cross-boundary mix (a `"add"` pattern over a `Symbol` scrutinee, or a
            // `#"add"` over a `String`) exactly as `=` does. Keying on the scrutinee instead made the check
            // trivially true for any text scrutinee — the pattern path was more permissive than `=` (the
            // Inc-45 regression, reviewer-found; `pattern_constraints` already tags origin this way).
            crate::core::Probe::Str(_) => match crate::resolve::resolved_of(db, probe_pats[i]) {
                crate::resolved::Resolved::SymbolConst(_) => Some(crate::ty::Ty::Symbol),
                _ => Some(crate::ty::Ty::String),
            },
            // A `Char` probe comes from a char-literal `#\a` pattern; it agrees only with a `Char`
            // scrutinee (a char pattern over an Int/String scrutinee is a shape error, CDZ0201).
            crate::core::Probe::Char(_) => Some(crate::ty::Ty::Char),
            // A `Bytes` probe comes from a byte-string-literal `b"…"` pattern; it agrees only with a `Bytes`
            // scrutinee (a bytes pattern over an Int/String scrutinee is a shape error, CDZ0201).
            crate::core::Probe::Bytes(_) => Some(crate::ty::Ty::Bytes),
            // A `ListLen`/`MapHasKeys` probe never arises in the SCALAR match path (each comes from a
            // list/map PAYLOAD sub-pattern in the sum decision tree, not `classify_probe`); no scalar-type
            // check applies.
            crate::core::Probe::ListLen { .. } => None,
            crate::core::Probe::MapHasKeys { .. } => None,
            crate::core::Probe::Wild => None,
        };
        if let Some(pt) = pat_ty
            && !pt.agrees_with(&scrut_ty)
        {
            // Anchor at the offending literal PATTERN (`probe_pats[i]`), not the enclosing match.
            return Core::Poison(
                Reject::coded(
                    Code::Malformed,
                    format!(
                        "match pattern type {} does not match scrutinee type {}",
                        pt.render_name(&db.name_ctx()),
                        scrut_ty.render_name(&db.name_ctx())
                    ),
                )
                .at(probe_pats[i]),
            );
        }
    }
    //  (2) exhaustiveness: a scalar match must cover every value of the scrutinee's type. A wildcard
    //      tail covers the rest, and for an OPEN type (Int64) that is the ONLY way — no finite literal
    //      set exhausts the integers. But a FINITE type is exhausted by its literals: a Bool scrutinee
    //      covered by BOTH a `true` arm and a `false` arm needs no wildcard (the two values are the
    //      whole type — `core-semantics.md` §Matching Is Exhaustive Or Rejected). This holds EVEN when
    //      the scrutinee is a constant that hits an arm — well-formedness is independent of the value.
    //      A GUARDED arm does NOT count toward exhaustiveness — its guard may be false, so it covers no
    //      value unconditionally (`core-semantics.md` §Matching Is Exhaustive Or Rejected: "A guard does
    //      NOT count toward exhaustiveness"). So only UNGUARDED arms contribute coverage below.
    let has_wild = probes
        .iter()
        .any(|(p, g, _)| g.is_none() && matches!(p, crate::core::Probe::Wild));
    // A Bool scrutinee's two literals exhaust it. (A definitely-Bool or still-open `Any` scrutinee whose
    // arms are Bool literals — a bare parameter matched with `true`/`false` — is matching over Bool; a
    // definitely-Int scrutinee with a Bool probe already faulted in step (1) and never reaches here.)
    let bool_true = probes
        .iter()
        .any(|(p, g, _)| g.is_none() && matches!(p, crate::core::Probe::Bool(true)));
    let bool_false = probes
        .iter()
        .any(|(p, g, _)| g.is_none() && matches!(p, crate::core::Probe::Bool(false)));
    let bool_exhaustive = scrut_ty.agrees_with(&crate::ty::Ty::Bool) && bool_true && bool_false;
    if !has_wild && !bool_exhaustive {
        // Name what is uncovered + carry an "add the covering arm" fix (the missing bool literal, or a
        // wildcard for an open scalar) — the scalar twin of the sum add-arms fix.
        return Core::Poison(non_exhaustive_scalar_reject(
            db, scrutinee, &scrut_ty, bool_true, bool_false,
        ));
    }

    // Well-formed. FOLD if the scrutinee is a compile-time constant: select the first arm whose probe
    // it satisfies AND whose guard (if any) folds to `true` (no runtime match, like the const `if` fold).
    // A guard is folded via `core_of` — the arm's binder resolves to the constant scrutinee (Case 5), so
    // `(< x 0)` over a constant `x` folds to a `ConstBool`. If a matched arm's guard does NOT fold to a
    // constant bool (its guard reads a runtime value), the fold ABORTS to the runtime probe chain (we
    // cannot decide the arm at compile time). A guard that folds `false` skips the arm to the next.
    let scrut_core = core_of(db, scrutinee);
    if let Core::Poison(r) = scrut_core {
        return Core::Poison(r);
    }
    let const_scrut = match &scrut_core {
        Core::ConstInt(v) => Some(GuardFoldScrut::Int(v.clone())),
        Core::ConstBool(b) => Some(GuardFoldScrut::Bool(*b)),
        Core::ConstStr(s) => Some(GuardFoldScrut::Str(s.to_string())),
        Core::ConstChar(c) => Some(GuardFoldScrut::Char(*c)),
        Core::ConstBytes(b) => Some(GuardFoldScrut::Bytes(b.to_vec())),
        _ => None,
    };
    if let Some(sc) = const_scrut {
        let mut foldable = true;
        for (probe, guard, body) in &probes {
            let probe_hit = match &sc {
                GuardFoldScrut::Int(v) => probe_matches_int(probe, v),
                GuardFoldScrut::Bool(b) => probe_matches_bool(probe, *b),
                GuardFoldScrut::Str(s) => probe_matches_str(probe, s),
                GuardFoldScrut::Char(c) => probe_matches_char(probe, *c),
                GuardFoldScrut::Bytes(b) => probe_matches_bytes(probe, b),
            };
            if !probe_hit {
                continue; // this arm's pattern doesn't match the constant — try the next
            }
            match guard {
                None => {
                    trace!(target: "rcdzc::fold", "match folds to a selected arm (constant scrutinee)");
                    return core_of(db, *body);
                }
                Some(g) => match core_of(db, *g) {
                    Core::ConstBool(true) => {
                        trace!(target: "rcdzc::fold", "match folds to a guarded arm (guard holds over a constant)");
                        return core_of(db, *body);
                    }
                    Core::ConstBool(false) => continue, // guard fails → fall through to the next arm
                    _ => {
                        // The guard did not fold to a constant bool (it reads a runtime value). We cannot
                        // decide this arm at compile time even though the scrutinee is constant → abort
                        // the fold and emit the runtime probe chain below.
                        foldable = false;
                        break;
                    }
                },
            }
        }
        if foldable {
            // Every matched arm's guard folded false and no unguarded arm covered — unreachable, because
            // exhaustiveness requires an unguarded wildcard/literal cover (checked above).
            return Core::Poison(Reject::decline(
                "match: no arm matched a constant (unreachable)",
            ));
        }
    }
    // A SINGLE unguarded CATCH-ALL BINDING arm `(match s (z <body>))` over a NON-SCALAR scrutinee binds
    // `z` to the scrutinee and yields the body — it inspects NO structure, so it needs no probe chain and
    // no heap walk. Lower it to the body directly (the binder resolves to the scrutinee via the Case-5
    // binder→scrutinee rule, so `(match (BigInt.of n) (z (* z z)))` becomes the `bigint-mul`). This must
    // come BEFORE the non-scalar decline below, which would otherwise reject a `BigInt`/compound scrutinee
    // even though a bare-binder match never looks at it. Restricted to a NON-scalar scrutinee: a SCALAR
    // single-binder match keeps its existing `Core::Match` path below (it carries the binder's
    // lexical-block DWARF — a scratch-slot variable fenced to the match — which a body-collapse would
    // drop; the debug-info tests pin that). The scrutinee is still evaluated for effects/traps: the body
    // reads it through the binder, or a trap-free unused scrutinee simply is not mentioned by `core_of`.
    if !is_scalar(db, scrutinee)
        && let [(crate::core::Probe::Wild, None, body)] = probes.as_slice()
    {
        trace!(target: "rcdzc::lower", scrutinee = scrutinee.0, "non-scalar match with a single catch-all binder lowers to its body");
        return core_of(db, *body);
    }
    // A RUNTIME STRING / SYMBOL / BYTES scrutinee matched against text/byte LITERALS — the compiler's
    // keyword/opcode dispatch (`(match head ("if" …) ("let" …) (_ …))`), the symbol twin (`(match tag
    // (#"add" …) …)`), and the byte-string twin (`(match b (b"AB" …) …)`). Each is a heap value (not
    // `is_scalar`), so the scalar probe-chain below cannot drive it; but runtime EQUALITY over each already
    // lowers to `value-eq` (a `(= s "add")` / `(= tag #"add")` / `(= b b"AB")` emits it — a direct-Bytes
    // operand is compacted first, so a rope compares by content), so such a match is exactly a chain of
    // `value-eq` tests. Rather than thread a heap handle through the Int/Bool-shaped `Core::Match` emit,
    // DESUGAR to a nested `if`-chain built from synthesized AST — `(if (= scrutinee "add") body0 (if (=
    // scrutinee "sub") body1 <else>))` — and lower THAT (the ordinary `Resolved::If` + `=`-over-heap-value
    // paths handle it, including tail position). A guarded arm nests its guard: `(if (= scrutinee lit) (if
    // guard body <else>) <else>)`. The final catch-all (a `_`/binder wildcard, guaranteed by the
    // exhaustiveness check above) is the innermost `else`. Only fires for a definitely-`String`/`Symbol`/
    // `Bytes` scrutinee whose arms are all text-literal (`Str` probe, shared by String + Symbol) / byte-
    // literal (`Bytes` probe) / wildcard probes (the type check above already rejected a mismatched probe —
    // a `Str` over Bytes, a `Bytes` over String — so the surviving probes are kind-consistent with the
    // scrutinee); any other compound still declines below.
    if matches!(
        scrut_ty,
        crate::ty::Ty::String | crate::ty::Ty::Symbol | crate::ty::Ty::Bytes
    ) && probes.iter().all(|(p, _, _)| {
        matches!(
            p,
            crate::core::Probe::Str(_) | crate::core::Probe::Bytes(_) | crate::core::Probe::Wild
        )
    }) {
        // Split the arms into the leading STRING-LITERAL / guarded probes (each becomes an `if` level) and
        // the final unconditional WILDCARD tail (the innermost `else`). Build from `arms` (the AST pattern
        // nodes) so the `=` RHS is the arm's own literal node and each body is reused verbatim.
        // Find the FIRST unguarded wildcard arm — it is the unconditional tail; arms after it are dead
        // (unreachable), so the chain ends there. Exhaustiveness guarantees one exists.
        let tail_ix = arms.iter().position(|&(pat, _)| {
            let inner = match db.ast.as_form(pat, "guard") {
                Some(g) if g.len() == 2 => g[0],
                _ => pat,
            };
            // An unguarded bare-name / `_` arm (a `Wild` probe with no guard).
            db.ast.as_form(pat, "guard").is_none() && db.ast.as_name(inner).is_some()
        });
        let Some(tail_ix) = tail_ix else {
            // No unguarded wildcard — exhaustiveness should have rejected this; decline defensively.
            return Core::Poison(Reject::decline(
                "a runtime string match needs an unguarded wildcard tail",
            ));
        };
        // The innermost else = the wildcard tail arm's body (reused in place). A binder wildcard's body
        // reads the scrutinee via the Case-5 binder→scrutinee rule; a `_` binds nothing.
        let mut else_node = arms[tail_ix].1;
        // Fold the leading arms (indices 0..tail_ix) from the LAST backward into nested `if`s.
        for &(pat, body) in arms[..tail_ix].iter().rev() {
            let (inner_pat, guard) = match db.ast.as_form(pat, "guard") {
                Some(g) if g.len() == 2 => (g[0], Some(g[1])),
                _ => (pat, None),
            };
            // A guard nests INSIDE the matched branch: `(if guard body <else>)`, so a false guard falls
            // through to the same `else` as a non-matching literal.
            let then_branch = match guard {
                Some(g) => {
                    let if_head = db.push_name("if");
                    db.push_list(vec![if_head, g, body, else_node])
                }
                None => body,
            };
            // A NAMED-wildcard inner pattern (`(guard t cond)`, `t` a bare name ≠ `_`) binds the WHOLE
            // scrutinee to `t` — it is NOT a string literal to compare, so there is NO `(= scrutinee t)`
            // test (a wildcard matches every string; the guard alone gates it). Bind `t` with a wrapping
            // `(let ((t scrutinee)) (if guard body <else>))` — WITHOUT this, the extracted test severed `t`
            // from its `(guard …)` ancestor and a `t` reference resolved UNBOUND (false CDZ0101) on the
            // heap/string face of Finding #46 (breaker adv-53); the #46 fix only covered the guarded-SCALAR
            // desugar, not this sibling runtime-STRING path. `forget_subtree` + binding-aware scope-skip so
            // `t` re-resolves to the new `let` binder (and any outer name the guard reads re-resolves up the
            // live chain), the exact discipline the scalar path uses.
            match db.ast.as_name(inner_pat).map(str::to_string) {
                Some(nm) if nm != "_" => {
                    let let_head = db.push_name("let");
                    let binder = db.push_name(&nm);
                    let pair = db.push_list(vec![binder, scrutinee]);
                    let bindings = db.push_list(vec![pair]);
                    let wrapped = db.push_list(vec![let_head, bindings, then_branch]);
                    crate::resolve::forget_subtree(db, wrapped);
                    db.extend_scope_skip_into_subtree(wrapped);
                    else_node = wrapped;
                }
                _ => {
                    // A string-LITERAL inner pattern: `(= scrutinee <literal>)` (value-eq) gates the arm.
                    let eq_head = db.push_name("=");
                    let eq = db.push_list(vec![eq_head, scrutinee, inner_pat]);
                    let if_head = db.push_name("if");
                    else_node = db.push_list(vec![if_head, eq, then_branch, else_node]);
                }
            }
        }
        // Give the synthesized `value-eq` if-chain scope-skip entries BEFORE resolving it: the chain nests
        // O(arms) deep and every node is a non-binding `if`/`=`/application form (the arm bodies + guards
        // are REUSED load-time occurrences, which keep their own final skip), so without this a prelude
        // name (`=`) — or a `check_unknown_units`/`collect_faults` resolution of any inner synth node — walks
        // O(depth) parents to conclude "not lexically bound" → O(N²) over N arms. The pass-through skip makes
        // each such resolution O(1). Same fix as the runtime-map-match desugar below (`bf5a1a1c`).
        db.extend_scope_skip_pass_through(else_node);
        trace!(target: "rcdzc::lower", scrutinee = scrutinee.0, "runtime string match → value-eq if-chain");
        return core_of(db, else_node);
    }
    // Runtime scalar scrutinee — it must BE a scalar (a compound needs a heap walk, later).
    if !is_scalar(db, scrutinee) {
        return Core::Poison(Reject::unsupported(
            "matching a compound value needs a heap walk",
        ));
    }
    // GUARDED SCALAR MATCH → nested `if`-chain (the scalar analogue of the string desugar above). A match
    // with ANY guard cannot use the `br_table` jump (both `try_emit_*_br_table` bail on a guard) and its
    // `Core::Match` emit lowers a guarded arm to `if guard body else <rest>` — which does NOT get the
    // `if`→select / bool-materialization the plain `Core::If` does. So `(match x ((guard n (> n 100)) 1)
    // (_ 0))` emitted a structured `if/else` block where the equivalent `(if (> x 100) 1 0)` branchlessly
    // materializes to `gt_s ; extend`. Desugar to `(if (= scrutinee lit) (if guard body <else>) <else>)`
    // (a literal-probe arm) / `(if guard body <else>)` (a guarded WILDCARD, no `=` test) and re-lower —
    // the ordinary `Resolved::If` path then applies every branchless conversion. Built from the AST
    // pattern/guard/body nodes exactly as the string path, so a binder resolves through its arm as before.
    // Only for a match that HAS a guard (an unguarded scalar match keeps its `Core::Match` → `br_table`/
    // probe-chain, unchanged); the arms must be Int/Bool literal or wildcard probes (the type check above
    // rejected a mismatch, and a guarded compound-pattern arm went to the sum path).
    if probes.iter().any(|(_, guard, _)| guard.is_some())
        && probes.iter().all(|(p, _, _)| {
            matches!(
                p,
                crate::core::Probe::Int(_) | crate::core::Probe::Bool(_) | crate::core::Probe::Wild
            )
        })
    {
        // The unconditional tail = the first UNGUARDED wildcard arm's body (exhaustiveness guarantees one;
        // a guarded arm never covers unconditionally). Arms after it are unreachable and dropped.
        let tail_ix = arms.iter().position(|&(pat, _)| {
            let inner = match db.ast.as_form(pat, "guard") {
                Some(g) if g.len() == 2 => g[0],
                _ => pat,
            };
            db.ast.as_form(pat, "guard").is_none() && db.ast.as_name(inner).is_some()
        });
        let Some(tail_ix) = tail_ix else {
            return Core::Poison(Reject::decline(
                "a guarded scalar match needs an unguarded wildcard tail",
            ));
        };
        // ag5 (breaker): PIN the reused arm bodies' + guard-conds' resolution NOW, while they STILL have
        // their intact lexical ancestry — BEFORE the fold below `push_list`-reparents them (which detaches
        // them from any enclosing binder). This establishes the invariant the desugar's skip coverage
        // assumes ("the reused arm bodies resolved + memoized BEFORE the desugar"). A load-time user body
        // already satisfies it; but the effects fold mints FRESH synth `#seed`/`#cv` references at LOWER
        // time whose binder is a fold-synth `(let ((#seed n)) …)` ABOVE the match — never memoized, so once
        // reparented their `#seed` ref can't reach that let → false CDZ0101. Pinning here (ancestry intact)
        // resolves each to a `Ref` that survives the reparent via the resolve memo (the `apply_lambda`
        // idiom, resolve.rs:206). Verified at entry: `#seed`'s chain reaches the fold `#seed` let and
        // resolves to `Ref`. The `(let ((w scrutinee)) …)` wrap below binds `w` to the SAME scrutinee that
        // its guard-cond resolution already names, so the pinned `w` resolution stays valid across the wrap.
        for &(pat, body) in arms[..tail_ix].iter() {
            let cond = db.ast.as_form(pat, "guard").and_then(|g| g.get(1).copied());
            crate::resolve::resolve_subtree(db, body);
            if let Some(g) = cond {
                crate::resolve::resolve_subtree(db, g);
            }
        }
        let mut else_node = arms[tail_ix].1;
        crate::resolve::resolve_subtree(db, else_node);
        // Fold the leading arms (0..tail_ix) from LAST backward into nested `if`s.
        for &(pat, body) in arms[..tail_ix].iter().rev() {
            let (inner_pat, guard) = match db.ast.as_form(pat, "guard") {
                Some(g) if g.len() == 2 => (g[0], Some(g[1])),
                _ => (pat, None),
            };
            // A LITERAL probe adds a `(= scrutinee <literal>)` test (scalar `=` → `i*.eq`); a WILDCARD
            // inner pattern (a bare name / `_`) has NO literal test — the guard alone gates it.
            let is_wild = db.ast.as_name(inner_pat).is_some();
            // A guarded arm nests its guard INSIDE the matched branch: `(if guard body <else>)`, so a false
            // guard falls through to the same `else` as a non-matching literal.
            let then_branch = match guard {
                Some(g) => {
                    let if_head = db.push_name("if");
                    db.push_list(vec![if_head, g, body, else_node])
                }
                None => body,
            };
            else_node = if is_wild {
                // A guarded wildcard: the guard IS the whole test — `(if guard body <else>)` (already built
                // as `then_branch`; an UNguarded wildcard before the tail is unreachable but harmless).
                // A NAMED wildcard binder `(guard w cond)` binds the WHOLE scrutinee to `w` — the guard cond
                // + body read `w`. This desugar extracts them into a bare `(if g body <else>)`, severing `w`
                // from its `(guard …)` ancestor; once the extracted test is REDUCED (a computed scrutinee
                // folds `w`'s value, copying the subtree into an orphan `if`), a `w` reference can no longer
                // recover the scrutinee via `guard_cond_binds` → false CDZ0101 `unbound w` (Finding #46).
                // Bind `w` explicitly with a wrapping `(let ((w scrutinee)) (if g body <else>))` — `w` then
                // resolves to a real `let` binder that survives the copy. The reused (un-cloned) g+body keep
                // their intact scope-skip for any OTHER (outer) names they read (`n`, an enclosing param), so
                // only `w`'s resolution must be refreshed to point at the new `let` (below, before core_of).
                match db.ast.as_name(inner_pat).map(str::to_string) {
                    Some(nm) if nm != "_" => {
                        let let_head = db.push_name("let");
                        let binder = db.push_name(&nm);
                        let pair = db.push_list(vec![binder, scrutinee]);
                        let bindings = db.push_list(vec![pair]);
                        let wrapped = db.push_list(vec![let_head, bindings, then_branch]);
                        // `w` was PINNED at desugar entry (above) via `guard_cond_binds` to the SAME
                        // scrutinee occurrence this new `(let ((w scrutinee)) …)` binds — so its memoized
                        // `Ref` is already correct and survives the reparent; likewise the fold-synth
                        // `#seed`/`#cv` refs were pinned to their fold-let binders. So we must NOT
                        // `forget_subtree(wrapped)` here: a blanket forget would clear those correct pins,
                        // and the detached `wrapped` (parent `None`) can no longer re-resolve a `#seed` whose
                        // binder is an enclosing fold let → the ag5 false CDZ0101. Keep the pins; just cover
                        // the fresh synth nodes with binding-aware skip (the `(let …)`/`if` chain) so a name
                        // NOT already pinned still hops O(1) to the nearest candidate.
                        db.extend_scope_skip_into_subtree(wrapped);
                        wrapped
                    }
                    _ => then_branch,
                }
            } else {
                let eq_head = db.push_name("=");
                let eq = db.push_list(vec![eq_head, scrutinee, inner_pat]);
                let if_head = db.push_name("if");
                db.push_list(vec![if_head, eq, then_branch, else_node])
            };
        }
        // Cover the synthesized guarded-scalar if-chain with scope-skip entries (the same O(N²) the string
        // desugar above avoids): a wide guarded scalar match nests O(arms) deep, and each inner `=`/`if`
        // synth node's name resolution would else ascend the whole spine. The chain is all non-binding
        // (bodies/guards are reused load-time occurrences), so the pass-through skip is sound + O(1).
        db.extend_scope_skip_pass_through(else_node);
        trace!(target: "rcdzc::lower", scrutinee = scrutinee.0, "guarded scalar match → if-chain (unlocks branchless if→select)");
        return core_of(db, else_node);
    }
    // ALL-SAME-BODY COLLAPSE: if every arm is UNGUARDED and all their bodies lower to the SAME core, the
    // match computes that value for every scrutinee — so it collapses to the body, dropping the probe
    // chain (the match analogue of `(if c x x)` → `x`). Guarded arms are excluded: a guard may fail, so
    // its arm does not unconditionally yield its body — the choice is then observable and the chain must
    // stay. Sound ONLY when the scrutinee is TRAP-FREE: the discriminant is otherwise unused after the
    // collapse, but the scrutinee was evaluated to drive the (now-gone) probes, so a scrutinee that could
    // trap must still be evaluated (keep the chain). `core_equiv` is the same conservative pure-core
    // equality the `if`-identical-branches fold uses; a binder arm's body `core_of` reads the scrutinee
    // (Case 5), so `(match a (n n))`'s arm equals `core_of(a)` and only collapses when every arm agrees.
    if probes.iter().all(|(_, guard, _)| guard.is_none())
        && let Some((_, _, first_body)) = probes.first()
        && probes[1..]
            .iter()
            .all(|(_, _, body)| core_equiv(db, *body, *first_body))
        && is_trap_free(db, scrutinee)
    {
        trace!(target: "rcdzc::fold", scrutinee = scrutinee.0, "match with all arms yielding the same value collapses to the body (trap-free scrutinee)");
        return core_of(db, *first_body);
    }
    // COMMON-CONSTRUCTOR SINK (the match analogue of `hoist_common_ctor` for `if`): if EVERY arm is
    // UNGUARDED and builds the SAME constructor `K` (same `SumNew` disc+arity / same-arity `Tuple` /
    // same-length `List` / same-key `Record`), build `K` ONCE and sink each field position into its own
    // per-position `match` over the same scrutinee — so `(match n (0 (Some 10)) (1 (Some 20)) (_ (Some
    // 30)))` becomes `(Some (match n (0 10) (1 20) (_ 30)))`: ONE `sum-new`/alloc, the scalar decision
    // tree chooses only the payload. See `sink_ctor_through_match_arms` for soundness (each arm's payload
    // still materializes only on its own matching path, so Perceus is unchanged; the scrutinee is
    // evaluated once by the sunk match exactly as by the original).
    if probes.iter().all(|(_, guard, _)| guard.is_none())
        && let Some(core) = sink_ctor_through_match_arms(db, scrutinee, &probes)
    {
        trace!(target: "rcdzc::fold", scrutinee = scrutinee.0, "match with all arms building the same constructor sinks the constructor out (build-once)");
        return core;
    }
    trace!(target: "rcdzc::lower", scrutinee = scrutinee.0, arms = probes.len(), "match stays runtime (scalar scrutinee → probe chain)");
    Core::Match {
        scrutinee,
        arms: probes
            .into_iter()
            .map(|(probe, guard, body)| crate::core::MatchArm {
                probe,
                guard,
                // An arm body is CONDITIONALLY reached (only when its probe matches), so a provable ConstTrap
                // there demotes to a runtime `Core::Trap` — traps when the arm is taken, not at compile time
                // (cn02 / operator ruling), the match twin of the runtime-`if` branch demote.
                body: demote_conditional_trap(db, body),
            })
            .collect(),
    }
}

/// SCALAR-REPLACEMENT-OF-AGGREGATES (SROA) escape-gate for a match-scrutinee tuple (operator directive,
/// tick ~353): decide whether `(match <scrutinee> arms)` is a candidate to have its scrutinee tuple
/// SCALAR-REPLACED — the tuple exploded into its element values instead of heap-allocated — and if so
/// return the element `StructId`s (in tuple order) for the dispatch to read directly.
///
/// A candidate iff BOTH hold:
///  1. `scrutinee`'s core is a FRESH RUNTIME `Core::Tuple { elems }` — built from runtime operands, NOT a
///     compile-time-visible constant (a const tuple's `Elem`-reads already fold via `const_at_path`, so it
///     never allocates — nothing to replace). `is_const_value` screens the const case out.
///  2. EVERY arm's top-pattern is a TUPLE-DESTRUCTURE (`(tuple …)`, peeling a `(guard …)` wrapper) — so the
///     tuple is consumed ONLY positionally (each element read by index), NEVER bound WHOLE. A bare-name or
///     `_` arm at the root binds the aggregate itself (it ESCAPES), so ANY such arm disqualifies the whole
///     match — FAIL-CLOSED: on any non-tuple-destructure arm (or a scrutinee that isn't a fresh runtime
///     tuple), return `None` and leave the materialized decision-tree path untouched.
///
/// This mirrors v-patterns' `pattern_constraints` classifier (a `(tuple …)` pattern descends `Elem(i)` =
/// positional = SAFE; a bare name / `_` returns the whole-value binder = ESCAPE) via the same syntactic
/// head-check they specified, without re-running the full constraint resolution.
///
/// The returned element `StructId`s are the tuple's own operand occurrences (already evaluated in tuple
/// order when the tuple is built), which v-patterns' `lower_tuple_arms_over_locals` dispatch reads directly
/// instead of an `Elem`-path over a materialized heap slot — so the `arr-alloc`/box for the tuple is
/// eliminated. The single-arm pure-binder case is ALREADY alloc-free (`const_at_path` reads a runtime
/// `Core::Tuple`'s `Elem(i)` directly); this gate is what lets the REFUTABLE (lit-test) multi-arm case join
/// it. Inert until that dispatch consumes the result.
#[allow(dead_code)]
pub(crate) fn sroa_tuple_scrutinee_candidate(
    db: &mut Db,
    scrutinee: StructId,
    arms: &[(StructId, StructId)],
) -> Option<Vec<StructId>> {
    // (1) The scrutinee must be a FRESH RUNTIME tuple — a `Core::Tuple` that is NOT a compile-time constant
    // (a const tuple already folds its element reads, never allocates). Screen the const case out.
    if is_const_value(db, scrutinee) {
        return None;
    }
    let Core::Tuple { elems } = core_of(db, scrutinee) else {
        return None;
    };
    // PROFITABILITY cap by aggregate FIELD-COUNT (v-wasm-opt guidance): there is NO practical wasm
    // locals ceiling (a u32; engines handle tens of thousands), so we do NOT cap on a locals budget — a
    // normal match-scrutinee tuple is 2–8 fields. But exploding a HUGE/deeply-nested aggregate spends
    // locals + copies for no win (and there is no backend coalescing — slots are bump-allocated), so gate
    // on a small field-count. `SROA_MAX_TUPLE_FIELDS` is a generous ceiling far above any real
    // match-scrutinee tuple; a wider one keeps the (already alloc-free-for-pure-binders) materialized path.
    const SROA_MAX_TUPLE_FIELDS: usize = 16;
    if elems.len() > SROA_MAX_TUPLE_FIELDS {
        return None;
    }
    // (2) Every arm's top-pattern must be a tuple-destructure `(tuple …)` (peeling a `(guard …)`), so the
    // tuple is only ever read positionally. A bare-name/`_`/non-tuple arm binds or escapes the whole
    // aggregate — disqualify the whole match (fail-closed).
    for &(pat, _body) in arms {
        // Peel a `(guard <inner> <cond>)` wrapper to its inner pattern (the guard may reference the bound
        // elements, which is fine post-scalar-replacement — it reads the same element values).
        let inner = match db.ast.as_form(pat, "guard") {
            Some(g) if g.len() == 2 => g[0],
            _ => pat,
        };
        // A non-`(tuple …)` head at the root — a bare name / `_` / other — binds the aggregate whole =
        // ESCAPE; `?` returns `None` (fail-closed), disqualifying the whole match. `compound_form_of`
        // recognizes the native `#tuple(…)` ctor-leaf head too (not only the name/string alias).
        db.ast
            .compound_form_of(inner, crate::ast::CompoundCtor::Tuple)?;
    }
    // PRE-BIND each element to a KEPT let (v-patterns' interface ask): eval-once/trap-once belongs at THIS
    // bind site, not the dispatch — an element read by ≥2 arms (or a lit-tested position also bound in
    // another arm) must evaluate EXACTLY ONCE (a re-emitted effectful/trapping elem would double-evaluate =
    // a miscompile). Marking each elem a kept binding makes every reference to it lower to a
    // `Core::LocalRef { binder: elem }` (see `core_of`'s `kept_bindings` arm) — the elem evaluates once into
    // a slot, all reads are `local.get`. Bind EVERY elem unconditionally (even a single-use one: a kept let
    // of a pure single-use value is free — copy-prop erases it downstream — and it keeps the eval-once
    // invariant uniform + trivially sound). The caller wraps the dispatch body in the matching `Core::Let`
    // via `sroa_let_wrap` below, so the kept bindings are actually emitted. Self-keyed (`(elem, elem)`): no
    // synthetic binder needed — the element's own occurrence is the binding key (the bin-match
    // materialize-once idiom at `lower_match_sum`).
    for &elem in elems.iter() {
        db.kept_bindings.insert(elem);
    }
    Some(elems.to_vec())
}

/// Wrap a SROA dispatch `body` in the `Core::Let` that binds the tuple's elements (the `elems` returned by
/// [`sroa_tuple_scrutinee_candidate`], already marked kept there). Self-keyed bindings `(elem, elem)` — a
/// reference to a kept binding lowers to `Core::LocalRef { binder: elem }`, so the dispatch reads each
/// element via its slot exactly once. This is the caller-side assembly for the SROA rewrite: the
/// escape-gate, the kept-marking, and this `Let`-wrap are all the v-core-opt binding side, while the `body`
/// is v-patterns' `lower_tuple_arms_over_locals` dispatch over the same `elems`. Inert until that wiring
/// calls both.
#[allow(dead_code)]
pub(crate) fn sroa_let_wrap(elems: &[StructId], body: StructId) -> Core {
    Core::Let {
        bindings: elems.iter().map(|&e| (e, e)).collect(),
        body,
    }
}

/// Sink a COMMON CONSTRUCTOR out of every arm of a scalar `match` — the multi-arm analogue of
/// `hoist_common_ctor`. When all arms build the SAME constructor `K` (same `SumNew` disc + payload
/// arity, a same-arity `Tuple`, a same-length `List`, or a `Record` with the SAME KEY SET), the heap
/// build is DUPLICATED once per arm, differing only in the field occurrences. Build `K` ONCE and put
/// each field position under its OWN `match` over the same scrutinee + probes, selecting that position's
/// per-arm value; a position that is `core_equiv` across ALL arms is shared directly (one occurrence, no
/// sub-match). `(match n (0 (Some a)) (_ (Some b)))` → `(Some (match n (0 a) (_ b)))`.
///
/// Caller guarantees every arm is UNGUARDED (a guard makes an arm's yield conditional on more than the
/// probe, so the per-position sub-match — which replays only the probes — would not observe the guard).
///
/// SOUND: the sunk per-position `match` replays the IDENTICAL scrutinee + probe sequence, so it selects
/// the SAME arm the original match did for every scrutinee value; the chosen arm's field value is thus
/// exactly what that arm's `K` build used. Only ONE arm's fields ever materialize (the matched arm's),
/// exactly as before — a differing field stays under a `match` arm, so Perceus consume-once is unchanged
/// (a heap field is dup'd/dropped on its own path as the original arm's build did). A SHARED field is a
/// value that is `core_equiv` across every arm (pure scalar — const/param/local/arith/compare/convert/
/// proj), so emitting it once reproduces the matched arm's single evaluation. The scrutinee is evaluated
/// once by the (outer, if any) sunk match exactly as the original match evaluated it once. Returns
/// `None` when the arms are not all the same constructor, disagree in shape/key-set, or there are no
/// field positions (a nullary variant / empty tuple/record — no build to share).
pub(super) fn sink_ctor_through_match_arms(
    db: &mut Db,
    scrutinee: StructId,
    probes: &[(crate::core::Probe, Option<StructId>, StructId)],
) -> Option<Core> {
    enum Shape {
        Sum(u32),
        Tuple,
        List,
        Record(Vec<crate::resolved::Symbol>),
    }
    if probes.is_empty() {
        return None;
    }
    // Read each arm body's constructor shape + its ordered field occurrences. All arms must agree on the
    // shape (same `Shape` AND the same arity/key-set), or there is no common constructor to sink.
    let arm_fields = |db: &mut Db, body: StructId| -> Option<(u8, Vec<StructId>)> {
        // A discriminating tag so arms of DIFFERENT constructor kinds don't get merged: 0=Sum(disc via
        // separate check), 1=Tuple, 2=List, 3=Record. The disc/keys are compared separately below.
        match core_of(db, body) {
            Core::SumNew { disc: _, payloads } => Some((0, payloads.to_vec())),
            Core::Tuple { elems } => Some((1, elems.to_vec())),
            Core::ListNew { elems } => Some((2, elems.to_vec())),
            Core::Record { fields } => Some((3, fields.values().copied().collect())),
            _ => None,
        }
    };
    // Establish the reference shape from the FIRST arm, then require every other arm to match it.
    let first_body = probes[0].2;
    let (shape, first_vals): (Shape, Vec<StructId>) = match core_of(db, first_body) {
        Core::SumNew { disc, payloads } => (Shape::Sum(disc), payloads.to_vec()),
        Core::Tuple { elems } => (Shape::Tuple, elems.to_vec()),
        Core::ListNew { elems } => (Shape::List, elems.to_vec()),
        Core::Record { fields } => {
            let keys: Vec<crate::resolved::Symbol> = fields.keys().cloned().collect();
            let vals: Vec<StructId> = keys.iter().map(|k| fields[k]).collect();
            (Shape::Record(keys), vals)
        }
        _ => return None,
    };
    let arity = first_vals.len();
    if arity == 0 {
        return None;
    }
    // Collect, per FIELD POSITION, the list of per-arm value occurrences (arm-order). Bail if any arm
    // disagrees in constructor kind, disc, arity, or (for a record) key set.
    let mut per_field: Vec<Vec<StructId>> = vec![Vec::with_capacity(probes.len()); arity];
    for (_, _, body) in probes {
        // Same-shape check, position-aligned.
        let ok = match &shape {
            Shape::Sum(d0) => {
                matches!(core_of(db, *body), Core::SumNew { disc, ref payloads } if disc == *d0 && payloads.len() == arity)
            }
            Shape::Tuple => {
                matches!(core_of(db, *body), Core::Tuple { ref elems } if elems.len() == arity)
            }
            Shape::List => {
                matches!(core_of(db, *body), Core::ListNew { ref elems } if elems.len() == arity)
            }
            Shape::Record(keys) => {
                matches!(core_of(db, *body), Core::Record { ref fields } if fields.len() == arity && fields.keys().zip(keys.iter()).all(|(a, b)| a == b))
            }
        };
        if !ok {
            return None;
        }
        let (_, vals) = arm_fields(db, *body)?;
        for (i, v) in vals.into_iter().enumerate() {
            per_field[i].push(v);
        }
    }
    // Build each field position: shared (all arms `core_equiv`) → the first arm's occurrence; otherwise a
    // fresh `Core::Match` over the same scrutinee + probes selecting that position's per-arm value.
    let mut out_vals: Vec<StructId> = Vec::with_capacity(arity);
    for field_vals in &per_field {
        let all_same = field_vals[1..]
            .iter()
            .all(|&v| core_equiv(db, v, field_vals[0]));
        if all_same {
            out_vals.push(field_vals[0]);
        } else {
            // Type the sunk per-position match from the JOIN of ALL arms' field types, not `field_vals[0]`
            // alone: a bare literal arm carries a DEFERRED width that inference leaves un-ground when a
            // SIBLING arm pins the concrete width (`(match n (9 (Some 2.0)) (_ (Some v0:Float32)))` — the
            // `2.0` stays `Float(Deferred)`, it does NOT conflict, unlike a two-CONCRETE-width mix which is
            // a CDZ0201 fault caught before lowering). Typing from the first arm alone stamps that Deferred
            // onto the sunk node → the emit branchless-select then defaults a deferred float to `f64` and
            // pushes it beside the concrete `f32` sibling → an INVALID `select` (validation: "select
            // operands have different types"). `Ty::join` adopts the fixed width from whichever arm pinned
            // it — the SAME join the if/match branch-type uses — so the sunk match types to the resolved
            // width and both arms ground to it.
            let field_ty = {
                let mut acc = crate::infer::type_of(db, field_vals[0]);
                for &v in &field_vals[1..] {
                    let t = crate::infer::type_of(db, v);
                    acc = acc.join(&t);
                }
                acc
            };
            let arms: Vec<crate::core::MatchArm> = probes
                .iter()
                .zip(field_vals.iter())
                .map(|((probe, _guard, _body), &v)| crate::core::MatchArm {
                    probe: probe.clone(),
                    guard: None,
                    body: v,
                })
                .collect();
            out_vals.push(synth_core(db, Core::Match { scrutinee, arms }, field_ty));
        }
    }
    Some(match shape {
        Shape::Sum(disc) => Core::SumNew {
            disc,
            payloads: out_vals.into(),
        },
        Shape::Tuple => Core::Tuple {
            elems: out_vals.into(),
        },
        Shape::List => Core::ListNew {
            elems: out_vals.into(),
        },
        Shape::Record(keys) => Core::Record {
            fields: std::rc::Rc::new(keys.into_iter().zip(out_vals).collect()),
        },
    })
}

/// Lower a `(match scrutinee (list-pattern body)…)` over a LIST scrutinee — folds a COMPILE-TIME-CONSTANT
/// scrutinee against element patterns (FIXED-ARITY and REST) and emits `Core::MatchList` (a `vec-len`
/// dispatch) for a runtime one. Its length gives the dispatch; select the FIRST arm whose pattern matches
/// — a `(list p0 … p_{k-1})` of arity k matches length exactly k, a rest pattern `(list p0 … p_{k-1} ..
/// rest)` matches length ≥ k, a bare binder / `_` matches any — and lower that arm's body. The body's
/// leading-element binders resolve to `SumPayload` `Elem(i)` reads and the rest binder to the tail sublist
/// (`RestFrom(k)`), each FOLDing against the constant list (resolve Case 6l/6r).
///
/// A leading element position COMPOSES (`core-semantics.md §145`): it MAY itself be any IRREFUTABLE nested
/// pattern — a wildcard, a name, a tuple pattern `(tuple a b)`, or a single-variant constructor `(Mk n)` —
/// matched recursively to any depth, binding the union of its sub-binders (which resolve down the extended
/// `Elem`/`Payload` path). Since dispatch is by LENGTH, an irrefutable element always matches once the
/// length holds, so the length-coverage exhaustiveness reasoning is unchanged. A REFUTABLE element (a
/// literal, a multi-variant constructor) would need element-VALUE refinement `Core::MatchList` does not yet
/// express, so it DECLINES honestly (a later increment); the whole arm pattern must be LINEAR (CDZ0102).
///
/// A list observed ONLY through its length and its elements in order — never any cell/node structure of
/// the representation (the fold reads `vec-len`/`vec-get`/`vec-split`, representation-agnostic). A
/// well-formed match must JOINTLY cover every length n ≥ 0 (the empty list AND every non-empty list) —
/// a `_`/whole-list binder or a `(list .. rest)` arm covers the infinite tail — else CDZ0210.
//= spec/capabilities/core-semantics.md#a-list-is-deconstructed-by-element-patterns-with-an-optional-rest
//# A list MUST be matchable by an element pattern that names some number of leading elements positionally and MAY end in a rest binder for the remaining elements.
// Length dispatch: a fixed-arity arm matches exactly its arity, a rest arm matches length ≥ its lead, a
// bare binder / `_` matches any length; the empty pattern `(list)` matches only the empty list, and a
// single-leading-element-plus-rest `(list x .. r)` matches any non-empty list.
//= spec/capabilities/core-semantics.md#a-list-is-deconstructed-by-element-patterns-with-an-optional-rest
//# An element pattern naming exactly `n` elements with no rest binder MUST match a list of length exactly `n`, binding each named element position to the corresponding element; a list of any other length MUST NOT match that pattern.
//= spec/capabilities/core-semantics.md#a-list-is-deconstructed-by-element-patterns-with-an-optional-rest
//# An element pattern naming `n` leading elements followed by a rest binder MUST match any list of length at least `n`, binding the leading positions to the first `n` elements and the rest binder to a list of the remaining elements in order.
//= spec/capabilities/core-semantics.md#a-list-is-deconstructed-by-element-patterns-with-an-optional-rest
//# In particular, the empty element pattern MUST match exactly the empty list, and a single-leading-element-plus-rest pattern MUST match any non-empty list, binding its first element and the rest of the list.
//= spec/capabilities/core-semantics.md#a-list-is-deconstructed-by-element-patterns-with-an-optional-rest
//# A set of list-element arms MUST be treated as exhaustive when it covers both the empty list and every non-empty list — for example an empty-list arm together with a leading-element-plus-rest arm, or an arm ending in a rest binder that names no leading elements — and a set of arms that leaves some length uncovered MUST be a compile-time error under *Matching Is Exhaustive Or Rejected* unless a later arm (a name or wildcard pattern) covers the remainder.
//= spec/capabilities/core-semantics.md#a-list-is-deconstructed-by-element-patterns-with-an-optional-rest
//# An element pattern MUST observe a list only through its length and its elements in order; it MUST NOT expose or depend on any internal cell or node structure of the list's representation, so that the same pattern matches a list regardless of how the list is represented.
// A leading element position COMPOSES — it MAY itself be any irrefutable nested pattern (a wildcard, a
// name, a tuple pattern, a single-variant constructor) matched recursively to any depth
// (`list_element_irrefutable_or_decline`), and the whole list pattern is checked linear (CDZ0102 via
// `check_pattern_linear`). The rest binder binds the tail SUBLIST, whose type is the SAME `List T` as the
// scrutinee (infer.rs `RestFrom`), so a recursive function may match it again.
//= spec/capabilities/core-semantics.md#a-list-is-deconstructed-by-element-patterns-with-an-optional-rest
//# Each element position and the rest binder MUST be a binder position in the sense of *Patterns Compose*, so an element MAY itself be any pattern (a wildcard, a name, a tuple pattern, a constructor pattern, or a nested element pattern) matched recursively, and the whole pattern MUST remain linear (`CDZ0102`). The rest binder MUST bind a value of the same list type as the scrutinee, so a recursive function MAY match it again.
/// If `elem_pat` is a REFUTABLE SCALAR/STRING LITERAL leading list-element sub-pattern (an `Int`/`Bool`/
/// `Str`/`Float`/`Bytes` — matches one value, not every value of its type), `true`. Such an element cannot
/// be handled by the length-dispatch matcher directly (it dispatches only on length), but it desugars to a
/// fresh binder at that position plus a `(= binder <lit>)` value test conjoined into the arm's guard — see
/// `desugar_refutable_literal_list_elements`. A multi-variant CONSTRUCTOR element is ALSO refutable but
/// needs discriminant refinement (a different mechanism), so it is NOT rewritten here and still declines.
pub(super) fn is_refutable_literal_element(db: &mut Db, elem_pat: StructId) -> bool {
    matches!(
        crate::resolve::resolved_of(db, elem_pat),
        crate::resolved::Resolved::Int(_)
            | crate::resolved::Resolved::Bool(_)
            | crate::resolved::Resolved::Str(_)
            | crate::resolved::Resolved::Float(_)
            | crate::resolved::Resolved::Bytes(_)
            // A CHAR (`#\a`) or SYMBOL (`#"go"`) literal is also a refutable scalar element — each matches
            // one value, and its `=` test lowers exactly as the others (char → codepoint compare, symbol →
            // shared-`ConstStr` content compare; Inc-45/46). `clone_literal_atom` already copies a `Char`/
            // `Sym` leaf, so the `(= elem <lit>)` guard the desugar synthesizes just works. Without these,
            // a `(list #\a .. r)` / `(list #"go" .. r)` element was not recognized as a literal and fell to
            // `check_binding_pattern` → a spurious CDZ0201 "not a tuple/record/constructor".
            | crate::resolved::Resolved::Char(_)
            | crate::resolved::Resolved::SymbolConst(_)
    )
}

/// A FRESH copy of the literal-atom node `e` (an `Int`/`Bool`/`Str`/`Float`/`Bytes`/`Sym`/`Char` leaf) — a
/// new `StructId` carrying an identical `Leaf`, with its OWN empty parent/resolve/type memos. Used to move
/// a pattern-position literal into an expression position (the `=` RHS of a synthesized value test): the
/// original node's resolve memo classifies it as a PATTERN literal, which would mis-resolve as an operand;
/// a fresh atom re-resolves cleanly as a value. Falls back to reusing `e` for a non-atom (never happens for
/// a literal element, but keeps the helper total).
pub(super) fn clone_literal_atom(db: &mut Db, e: StructId) -> StructId {
    match db.ast.get(e) {
        crate::ast::Struct::Atom(lid) => {
            let leaf = db.ast.leaf(*lid).clone();
            db.push_atom(leaf)
        }
        _ => e,
    }
}

/// A FRESH DEEP COPY of the subtree rooted at `id` — every List node re-pushed with copied children, every
/// NAME atom re-pushed (a fresh binder/reference the re-resolve re-binds against its NEW position), and every
/// CONSTANT-leaf atom SHARED (a literal resolves to its own value regardless of scope, so sharing it is
/// sound — the same rule `sums::copy_subtree` uses). Unlike the module-local `Arenas::copy_subtree` copies
/// (accum/effects/sums/param_sidecar), this goes through `Db::push_*` so the copy's parent/`child_ix` index
/// is recorded — required before a following `resolve_subtree`, whose scope ascent reads `parent_of`. Used
/// by the MATCH-INTO-IF fusion to give each `if` branch its OWN copy of the arms (a single node cannot have
/// two parents — `push_list` reparents — so the two branches cannot share one arm set).
/// AST-node count of the subtree at `id`, SHORT-CIRCUITED at `cap` (returns `cap` once reached, never walks
/// further). Used by `fuse_match_into_if` as a PROFITABILITY guard: the fusion deep-COPIES each arm into
/// BOTH `if` branches, so a large arm subtree is duplicated 2× per level — and when the arm body is itself a
/// nested match the copy recurses the fusion, giving O(size²)+ synthesized nodes (a compile-time blowup on a
/// deeply-nested same-scrutinee match, v-inference-profiled ~O(N^3.7)). Capping the walk keeps this guard
/// O(cap), not O(subtree), so the guard itself never contributes to the blowup.
pub(super) fn ast_subtree_size_capped(db: &mut Db, id: StructId, cap: usize) -> usize {
    fn walk(db: &mut Db, id: StructId, cap: usize, acc: &mut usize) {
        if *acc >= cap {
            return;
        }
        *acc += 1;
        if let crate::ast::Struct::List(children) = db.ast.get(id).clone() {
            for c in children {
                if *acc >= cap {
                    return;
                }
                walk(db, c, cap, acc);
            }
        }
    }
    let mut acc = 0;
    walk(db, id, cap, &mut acc);
    acc
}

/// The match-FUSION arm-clone, threading the FUSED match's SCRUTINEE (`fused_scrut`)
/// so a pinned `SumPayload` binder can be classified by scrutinee IDENTITY:
///  - a binder of the MATCH BEING FUSED (its `SumPayload.scrutinee` IS `fused_scrut`) must be COPIED — the
///    clone re-emits that match over the branch value, and the copied binder re-resolves against the new
///    branch scrutinee (the whole point of the fusion; the rust `emit_sum_payload` resolves a binder by its
///    (scrutinee, path), so a SHARED binder would read the ORIGINAL now-detached switch → "sum payload has
///    no bound match arm" decline);
///  - a binder of an ENCLOSING match (its scrutinee is NOT `fused_scrut` — e.g. an outer
///    `(match cs ((list c .. t) … <cloned arm reads c>))`) is a genuine CAPTURE and must be SHARED,
///    exactly like a non-payload pinned name; copying it fresh re-resolves it lexically against the orphaned
///    clone → spurious CDZ0101 (the rest-pattern-binder-in-an-inlined-callee bug).
///
/// The classification is by scrutinee IDENTITY, NOT subtree containment: the pinned-binder path only sees
/// binders resolved by β-SUBSTITUTION (`resolved_subtrees`); a nested match's OWN binders INSIDE a cloned
/// arm resolve lexically and re-resolve against the clone via the name-copy fall-through, so they never
/// need this copy-test (verified: nested-fused-match-in-arm + inlined-nested-match-in-arm both emit on both
/// backends). The identity check is exact — sharper than the `is_within(scrutinee, clone_root)` containment
/// test the prior impl (`11c39b005`) threaded, which over-shared the fused match's OWN binder (its scrutinee
/// also sits outside the arm subtree) and regressed a rust Result/Option pipeline (fixed by `a5f7cfafb`).
///
/// `fused_scrut` is `Some(the fused match's scrutinee)` at every current call site; `None` (share every
/// pinned SumPayload) is available for a future non-fusion clone that has no own-binder to re-resolve.
pub(super) fn clone_subtree_db_for_fused(
    db: &mut Db,
    id: StructId,
    fused_scrut: Option<StructId>,
) -> StructId {
    match db.ast.get(id).clone() {
        crate::ast::Struct::Atom(lid) => match db.ast.leaf(lid).clone() {
            crate::ast::Leaf::Name(_) => {
                // A pinned free-var CAPTURE — a name whose resolution was recorded (`resolved_subtrees`) by
                // a β-SUBSTITUTION, not lexically — must be SHARED, not copied fresh (the `run (mk k) k`
                // case). But a `SumPayload` binder of the MATCH BEING FUSED must still COPY + re-resolve
                // against the branch scrutinee. Distinguish by the binder's own scrutinee: it belongs to
                // the fused match iff its scrutinee IS `fused_scrut` (an exact identity test, not containment);
                // otherwise it reads an ENCLOSING match and is a capture to share. See the fn doc.
                if db.resolved_subtrees.contains(&id) {
                    let copy_payload = matches!(
                        crate::resolve::resolved_of(db, id),
                        crate::resolved::Resolved::SumPayload { scrutinee, .. }
                            | crate::resolved::Resolved::RecordField { scrutinee, .. }
                            if fused_scrut == Some(scrutinee)
                    );
                    if !copy_payload {
                        return id;
                    }
                }
                let crate::ast::Leaf::Name(n) = db.ast.leaf(lid).clone() else {
                    unreachable!()
                };
                db.push_name(&n)
            }
            // A constant leaf resolves to its own value regardless of scope — share it.
            _ => id,
        },
        crate::ast::Struct::List(children) => {
            let copied: Vec<StructId> = children
                .iter()
                .map(|&c| clone_subtree_db_for_fused(db, c, fused_scrut))
                .collect();
            db.push_list(copied)
        }
    }
}

/// PRE-PASS for `lower_match_list` (the LIST analogue of Inc-20's tuple-of-bools exhaustiveness): a set of
/// `(list <bool-lit> .. rest)` arms whose position-0 bool literals SATURATE the element type ({true, false}
/// both present) JOINTLY covers every NON-EMPTY list — the first element is `true` or `false`, nothing else —
/// so together with a `(list)` arm the match is total WITHOUT a `_`.
//= spec/capabilities/core-semantics.md#a-list-is-deconstructed-by-element-patterns-with-an-optional-rest
//# A set of list-element arms MUST be treated as exhaustive when it covers both the empty list and every non-empty list — for example an empty-list arm together with a leading-element-plus-rest arm, or an arm ending in a rest binder that names no leading elements — and a set of arms that leaves some length uncovered MUST be a compile-time error under *Matching Is Exhaustive Or Rejected* unless a later arm (a name or wildcard pattern) covers the remainder.
///
/// The length-dispatch matcher cannot see this on its own: each bool-lead arm desugars (via the literal
/// pass) to a GUARDED rest arm, and a guarded arm is excluded from length-coverage → a spurious CDZ0210.
/// The fix mirrors Inc-20's scalar `bool_exhaustive`: the LAST saturating arm's position-0 test is
/// REDUNDANT — reaching it (first-match-wins) means every earlier bool-lead arm failed, so the first element
/// is not their value, hence (Bool being two-valued) it IS this arm's value. Rewrite that arm's bool literal
/// to a wildcard `_`, turning it into an unconditional `(list _ .. rest)` cover — which the length matcher
/// DOES count (an unguarded lead-1 rest closes the non-empty tail). The other saturating arm keeps its
/// literal (→ a guard via the literal pass on the recursion, correct: its value is genuinely tested). This
/// fixes BOTH the exhaustiveness check and the runtime fall-through in one move, reusing every existing pass.
///
/// Fires ONLY when (a) the element type agrees with Bool, (b) both bool values appear at position 0 among
/// BARE (unguarded) `(list <bool-lit> .. rest)` arms — exactly one leading element followed by a rest binder,
/// and (c) there is NO existing unguarded whole-tail cover (a bare `_`/name arm or a lead-0 `(list .. rest)`),
/// else the match is already total and the rewrite would only introduce a redundant arm (CDZ0213). Returns
/// `Some(Core)` iff it fired (rebuilds the match, re-resolves the new subtree, and recurses through `core_of`).
pub(super) fn desugar_saturating_bool_list_elements(
    db: &mut Db,
    scrutinee: StructId,
    arms: &[(StructId, StructId)],
) -> Option<Core> {
    // The element type — bail unless it agrees with Bool (a bool literal at an element position of a
    // non-Bool list is a type error caught elsewhere; requiring Bool keeps this narrowly the documented
    // saturation case, and matches the permissive `Any` treatment a binding position gets).
    let elem_ty = match crate::infer::type_of(db, scrutinee) {
        crate::ty::Ty::List(e) => (*e).clone(),
        _ => crate::ty::Ty::Any,
    };
    if !elem_ty.agrees_with(&crate::ty::Ty::Bool) {
        return None;
    }
    // Classify each SOURCE arm. `bool_lead[i]` = Some(b) iff arm i is a BARE (unguarded)
    // `(list <bool-lit=b> .. <name>)` — exactly one leading element (a bool literal) followed by a rest
    // binder. `has_tail_cover` = some UNGUARDED arm already covers the whole (or the entire non-empty) tail,
    // so the match is already total and this rewrite would only add a redundant arm.
    let mut bool_lead: Vec<Option<bool>> = Vec::with_capacity(arms.len());
    let mut has_tail_cover = false;
    for &(pat, _) in arms {
        // A `(guard …)`-wrapped arm is explicitly conditional — never a saturating candidate, and it does
        // not count as a tail cover either (its guard may fail).
        if db.ast.as_form(pat, "guard").is_some() {
            bool_lead.push(None);
            continue;
        }
        // A bare name / `_` is an unguarded whole-tail cover.
        if db.ast.as_name(pat).is_some() {
            has_tail_cover = true;
            bool_lead.push(None);
            continue;
        }
        let Some(es) = db
            .ast
            .compound_form_of(pat, CompoundCtor::List)
            .map(<[_]>::to_vec)
        else {
            bool_lead.push(None);
            continue;
        };
        match db
            .ast
            .rest_marker(&es)
            .map(|(i, _, trailing_start)| (i, trailing_start))
        {
            // A lead-0 rest `(list .. rest)` / `(list (.. rest))` covers every length → an unguarded whole-tail cover.
            Some((0, _)) => {
                has_tail_cover = true;
                bool_lead.push(None);
            }
            // A single leading element + rest at the end, that element a bool literal → a saturating candidate.
            Some((1, trailing_start)) if trailing_start == es.len() => {
                match crate::resolve::resolved_of(db, es[0]) {
                    crate::resolved::Resolved::Bool(b) => bool_lead.push(Some(b)),
                    _ => bool_lead.push(None),
                }
            }
            // Any other rest arity, or a fixed-arity arm (covers only its own length) — not relevant.
            _ => bool_lead.push(None),
        }
    }
    // Already total via an existing tail cover → don't fire.
    if has_tail_cover {
        return None;
    }
    // Saturation: both bool values present at position 0 among the candidate arms.
    let has_true = bool_lead.contains(&Some(true));
    let has_false = bool_lead.contains(&Some(false));
    if !(has_true && has_false) {
        return None;
    }
    // The LAST saturating candidate (source order) — its position-0 test is redundant once reached.
    let last = bool_lead.iter().rposition(Option::is_some)?;
    // Rewrite: replace that arm's leading bool literal with a wildcard `_`, turning it into an unconditional
    // `(list _ .. rest)` cover. Rebuild the whole match, re-resolve the synthesized subtree (the reused
    // arms keep their memoized resolution; the new `_` element + list node resolve fresh), and recurse.
    let mut new_arms: Vec<StructId> = Vec::with_capacity(arms.len());
    for (ai, &(pat, body)) in arms.iter().enumerate() {
        if ai != last {
            new_arms.push(db.push_list(vec![pat, body]));
            continue;
        }
        let es = db
            .ast
            .compound_form_of(pat, CompoundCtor::List)
            .map(<[_]>::to_vec)
            .expect("the saturating arm is a bare list pattern");
        let list_head = match db.ast.get(pat) {
            crate::ast::Struct::List(items) if !items.is_empty() => items[0],
            _ => db.push_name("list"),
        };
        // es = [<bool-lit>, "..", rest] — replace element 0 (the bool literal) with `_`, keep `.. rest`.
        let wild = db.push_name("_");
        let mut list_children = vec![list_head, wild];
        list_children.extend(es[1..].iter().copied());
        let new_list = db.push_list(list_children);
        new_arms.push(db.push_list(vec![new_list, body]));
    }
    let match_head = db.push_name("match");
    let mut items = vec![match_head, scrutinee];
    items.extend(new_arms);
    let rewritten = db.push_list(items);
    crate::resolve::resolve_subtree(db, rewritten);
    trace!(target: "rcdzc::lower", scrutinee = scrutinee.0, "list match with saturating bool first-elements → drop the last arm's redundant position-0 test");
    Some(core_of(db, rewritten))
}

/// PRE-PASS for `lower_match_list`: rewrite every arm whose list pattern has a refutable SCALAR/STRING
/// LITERAL leading element into an equivalent guarded arm the length-dispatch matcher already handles.
///
/// A literal leading element `(list 0 a .. r)` is an element-VALUE refinement `Core::MatchList` cannot
/// express (it dispatches only on LENGTH). But it is exactly a fresh binder at that position PLUS a guard
/// testing that binder equals the literal: `(list 0 a .. r)` ≡ `(guard (list __le0 a .. r) (= __le0 0))`.
/// This reuses the ENTIRE Inc-5 guard pipeline (resolve Case 6l/6lg binds `__le0`→`Elem(0)`; `ListArm.guard`
/// gates the arm; a false guard falls through; a guarded arm is EXCLUDED from length-coverage
/// exhaustiveness — correct, since a literal test may not match, so a `_`/rest catch-all stays required) —
/// NO new IR, NO backend change. Multiple literal elements in one arm conjoin with `and`; an existing arm
/// guard is conjoined too (the literal tests AND the author's guard). This is the list-element analogue of
/// the Inc-8 runtime-string desugar and the case-of-match rewrite — synthesize the AST, re-resolve the new
/// subtree, and lower it. Returns `Some(Core)` iff at least one arm had a literal element (so the rewrite
/// fired); `None` leaves the existing path untouched (no cost when no arm needs it).
pub(super) fn desugar_refutable_literal_list_elements(
    db: &mut Db,
    scrutinee: StructId,
    arms: &[(StructId, StructId)],
) -> Option<Core> {
    // CAPTURE the ORIGINAL match's lexical parent NOW, before any `push_list` re-wires `scrutinee`'s
    // parent: the original match is `scrutinee`'s current parent (scrutinee is its tail-0 child). The
    // rewritten match REPLACES it, and must ascend to the SAME enclosing scope so a user guard cond that
    // reads an enclosing `let` binding (`(let ((lim 5)) (match xs ((guard (list 0 x) (> x lim)) …)))`)
    // re-resolves that binding after this def is INLINED (the β-copy re-parents the `let`/`match` and this
    // desugar re-runs on the copy). Without it `rewritten`'s parent is `None`, so `resolve_subtree` below
    // memoizes the guard cond's `lim` as UNBOUND — the false CDZ0101 that surfaces only on the inlined path
    // (a direct-export / non-inlined call resolves the original before the copy, dodging it).
    let orig_match_parent: Option<(Option<StructId>, u32)> = db
        .parent_of(scrutinee)
        .map(|orig_match| (db.parent_of(orig_match), db.child_ix_of(orig_match) as u32));
    // Detect whether ANY arm's list pattern has a refutable literal LEADING element. Scan the arm patterns
    // (peeling a `(guard …)` wrapper), find each `(list …)`'s leading positions (before a `..` marker), and
    // check each for a scalar/string literal. Bail early if none — the common case pays only this scan.
    let arm_pats: Vec<StructId> = arms.iter().map(|&(pat, _)| pat).collect();
    let mut any_literal = false;
    for &pat in &arm_pats {
        let inner = match db.ast.as_form(pat, "guard") {
            Some(g) if g.len() == 2 => g[0],
            _ => pat,
        };
        let Some(es) = db
            .ast
            .compound_form_of(inner, CompoundCtor::List)
            .map(<[_]>::to_vec)
        else {
            continue;
        };
        let lead = db
            .ast
            .rest_marker(&es)
            .map(|(i, _, _)| i)
            .unwrap_or(es.len());
        for &e in &es[..lead] {
            if is_refutable_literal_element(db, e) {
                any_literal = true;
            }
        }
    }
    if !any_literal {
        return None;
    }
    // Rewrite each arm. For a list pattern with literal leading elements, replace each such element with a
    // fresh binder `__le{arm}_{pos}` (unique per arm+position so distinct arms never collide, and it is
    // unspellable so it cannot shadow a user name) and collect a `(= __le{arm}_{pos} <lit>)` test. The
    // arm's new guard is the conjunction of those tests AND any pre-existing guard.
    let mut new_arms: Vec<StructId> = Vec::with_capacity(arms.len());
    for (ai, &(pat, body)) in arms.iter().enumerate() {
        let (inner, existing_guard) = match db.ast.as_form(pat, "guard") {
            Some(g) if g.len() == 2 => (g[0], Some(g[1])),
            _ => (pat, None),
        };
        let list_es = db
            .ast
            .compound_form_of(inner, CompoundCtor::List)
            .map(<[_]>::to_vec);
        let Some(es) = list_es else {
            // Not a list pattern (a bare binder / `_` catch-all) — reuse the arm verbatim.
            new_arms.push(db.push_list(vec![pat, body]));
            continue;
        };
        let head = db.ast.get(inner);
        // The `(list …)` head token (a `Name` "list" or the reserved `Str` "list") — preserve it so the
        // rewritten pattern re-resolves as a list pattern exactly as the original did.
        let list_head = match head {
            crate::ast::Struct::List(items) if !items.is_empty() => items[0],
            _ => db.push_name("list"),
        };
        let lead = db
            .ast
            .rest_marker(&es)
            .map(|(i, _, _)| i)
            .unwrap_or(es.len());
        let mut new_es: Vec<StructId> = Vec::with_capacity(es.len());
        let mut tests: Vec<StructId> = Vec::new();
        for (pos, &e) in es.iter().enumerate() {
            if pos < lead && is_refutable_literal_element(db, e) {
                let name = format!("__le{ai}_{pos}");
                // TWO DISTINCT occurrences of the fresh binder: one in the PATTERN position (the element
                // binder) and a SEPARATE reference in the guard-cond. A single node cannot have two parents
                // — `push_list` reparents — so reusing one node would leave the guard reference parented to
                // the LIST, where `is_list_pattern_element_occurrence` classifies it as an inert pattern
                // binder (`Resolved::Unit`), and its type would be `Unit` not the element type (so `=` would
                // wrongly route to a heap `value-eq`). The pattern binder is inert; the guard reference
                // resolves via Case 6lg to the element's `SumPayload` read.
                let binder = db.push_name(&name);
                let binder_ref = db.push_name(&name);
                let eq_head = db.push_name("=");
                // `(= __le{pos} <lit>)`. The literal RHS is a FRESH copy of the element node (not the
                // original `e` reused): the original `e` sits at a pattern ELEMENT position (its resolve
                // memo classifies it as a pattern literal); reusing it as an EXPRESSION operand would carry
                // that stale pattern-context resolution. A fresh literal atom re-resolves cleanly as a value.
                let lit = clone_literal_atom(db, e);
                let eq = db.push_list(vec![eq_head, binder_ref, lit]);
                tests.push(eq);
                new_es.push(binder);
            } else {
                new_es.push(e);
            }
        }
        // Reassemble the list pattern with the literal elements replaced by fresh binders.
        let mut list_children = vec![list_head];
        list_children.extend(new_es);
        let new_list = db.push_list(list_children);
        // Combine the value tests (left-to-right `and`) with any existing guard into the arm's new guard.
        let mut guard: Option<StructId> = existing_guard;
        for test in tests {
            guard = Some(match guard {
                None => test,
                Some(g) => {
                    let and_head = db.push_name("and");
                    db.push_list(vec![and_head, g, test])
                }
            });
        }
        let new_pat = match guard {
            Some(g) => {
                let guard_head = db.push_name("guard");
                db.push_list(vec![guard_head, new_list, g])
            }
            None => new_list,
        };
        new_arms.push(db.push_list(vec![new_pat, body]));
    }
    // Rebuild `(match scrutinee arm…)`, re-resolve the synthesized subtree (so each fresh binder's Elem read
    // and each `(= __le <lit>)` guard resolve against their new positions — exactly as the case-of-match
    // rewrite re-resolves), and lower it through the ordinary path (which routes back to `lower_match_list`,
    // now with no literal elements → the guarded-arm path handles them).
    let match_head = db.push_name("match");
    // Capture the arm ids before `items` moves `new_arms`: only the ARMS are forgotten + re-resolved below
    // (the scrutinee's resolution is preserved — see the forget-arms-only note).
    let arm_ids: Vec<StructId> = new_arms.clone();
    let mut items = vec![match_head, scrutinee];
    items.extend(new_arms);
    let rewritten = db.push_list(items);
    // WIRE `rewritten`'s PARENT pointer to the original match's parent, so its lexical-scope ASCENT reaches
    // the enclosing scope (the `let` a guard cond reads) during the scope-skip extension + `resolve_subtree`
    // below. Only the parent POINTER is set (not the parent's child-LIST): lowering consumes the returned
    // `Core` directly, never walks DOWN from the parent to `rewritten`, so leaving the parent's children
    // pointing at the original match is harmless — whereas rewriting the child-list would risk corrupting a
    // sibling walk. `extend_scope_skip_into_subtree` seeds the root's skip from this parent, and the
    // exhaustive `resolve` walk ascends through it, so the guard cond's enclosing-`let` reference resolves.
    if let Some((new_parent, child_ix)) = orig_match_parent {
        db.reparent(rewritten, new_parent, child_ix);
    }
    // The synth arms hold a `(guard (list …) (and (and (= __le0 …) …) …))` whose guard COND is an O(N)-
    // DEEP left-nested `and`-chain (`and` is strictly binary). Those synth nodes are past-load, so without
    // scope-skip coverage each `__leK` guard reference (and each prelude `=`/`and`) walks O(depth) `and`
    // forms to reach the enclosing `(guard …)` → O(N²). Cover the subtree with the CANDIDATE-AWARE
    // extension (the `and`-spine is non-binding, the one `(guard …)` node is a candidate its children skip
    // TO) so every inner reference hops O(1). See `Db::extend_scope_skip_into_subtree`.
    db.extend_scope_skip_into_subtree(rewritten);
    // FORGET any stale memoized resolution in the ARMS before re-resolving: the user guard cond
    // (`(> x lim)`) is REUSED from the (copied) source arm, so a reference may already be memoized against
    // its pre-desugar position (unbound, from the β-copy). Clearing the column lets `resolve_subtree`
    // re-resolve against the now-correct parent ascent. Fresh synth nodes are unmemoized; this matters only
    // for the reused cond/body — bounded to the arms.
    //
    // Forget the ARMS ONLY, NOT the SCRUTINEE (the scrutinee is a reference resolved in the ENCLOSING
    // context, which this desugar does not change — it only wraps the arms); and forget the arms
    // KEEP-PINNED. Both preserve a resolution that ascent CANNOT recompute here — a reference bound by an
    // OUTER ARM PATTERN, a SIBLING of this rewrite rather than an ancestor. When the NESTED-LIST desugar
    // (Inc-14) routes its body re-match `(match __ne (<inner-list-pat> body) …)` HERE (inner list has a
    // LITERAL element), it has ALREADY `resolve_subtree`-PINNED both the `__ne` SCRUTINEE and every body
    // reference to its outer `(guard (list __ne (.. r)) …)` binders — including a SIBLING outer-REST binder
    // `r` the body reads. Blanket-forgetting drops those, and re-resolving by plain parent-ascent fails
    // (the outer arm's pattern binders are not reachable that way) → spurious CDZ0101 `unbound __ne0` / `r`
    // (breaker #8347 scrutinee, #8358 sibling rest binder). `forget_subtree_keep_pinned` PRESERVES the pinned
    // (already-correct) refs while still forgetting+re-resolving the UNPINNED reused β-copied guard cond
    // (which memoized unbound against its pre-desugar position and must recompute). A non-nested (top-level)
    // literal-list match's scrutinee/arms are resolved normally either way (a plain variable scrutinee is not
    // pinned to an unreachable binder; the β-copied cond is the only thing needing re-resolution).
    for &arm in &arm_ids {
        crate::resolve::forget_subtree_keep_pinned(db, arm);
    }
    crate::resolve::resolve_subtree(db, rewritten);
    trace!(target: "rcdzc::lower", scrutinee = scrutinee.0, "list match with refutable literal elements → fresh-binder + value-test guards");
    Some(core_of(db, rewritten))
}

/// If `elem_pat` is a REFUTABLE MULTI-VARIANT CONSTRUCTOR leading list-element sub-pattern — a `(C.V …)` /
/// `(V …)` / bare-member `(. Sum V)` whose owning sum has ≥2 variants — return `Some(head)` (the ctor head
/// occurrence). Such an element matches only when the element's DISCRIMINANT is `V`, so — like a literal —
/// it is refutable and the length-dispatch matcher cannot express it directly; it desugars to a fresh
/// binder + a discriminant-test guard + a body that re-matches the binder (`desugar_refutable_ctor_list_
/// elements`). A SINGLE-variant ctor is IRREFUTABLE (handled inline by `list_element_irrefutable_or_decline`
/// / Inc-1), so it returns `None` here. A non-ctor element (literal, tuple, bare binder) returns `None`.
pub(super) fn refutable_ctor_element_head(db: &mut Db, elem_pat: StructId) -> Option<StructId> {
    // The ctor head: a bare name / member `(. Sum V)` used whole (a NULLARY variant — `C.R`), or a `(head
    // arg…)` application's head (a payload variant — `Op.Add`). The same head extraction
    // `classify_binding_ctor` performs.
    let head = match db.ast.get(elem_pat) {
        crate::ast::Struct::Atom(_) => elem_pat,
        crate::ast::Struct::List(children) => match children.first().copied() {
            Some(first) if db.ast.as_name(first) == Some(".") => elem_pat,
            Some(first) => first,
            None => return None,
        },
    };
    // Resolve the head's owning sum via a FRESH copy of it. A NULLARY-variant head that IS the whole
    // element (`(. C R)` / a bare `Red`) sits in LIST-ELEMENT position, where resolve classifies the
    // occurrence INERT (`Resolved::Unit`, like any list-element binder) — so `variant_owner_decl` on the
    // in-place node reads no variant scheme and returns None, and the element fell through to a spurious
    // CDZ0201 "not a constructor". A fresh clone (`clone_ctor_head`, out of element context) re-resolves as
    // an ordinary variant reference — the same node the desugar's guard/body will use. (An APPLIED head
    // `(. Op Add)` is already a distinct non-element node, so cloning it is harmless.)
    let resolve_head = clone_ctor_head(db, head);
    let decl = crate::eval::variant_owner_decl(db, resolve_head)?;
    let variant_count = db.type_decl_by_occ(decl).map(|d| d.variants.len())?;
    if variant_count > 1 {
        // A MULTI-variant ctor is refutable by its DISCRIMINANT — the classic case.
        return Some(head);
    }
    // A SINGLE-variant ctor (a newtype `(Wrap …)`) has no discriminant, so it is IRREFUTABLE when its
    // payload is — a bare binder `(Wrap x)` always matches once the length holds (handled inline by
    // `list_element_irrefutable_or_decline`). But a REFUTABLE PAYLOAD makes the whole element refutable:
    // `(Wrap 0)` matches only a `Wrap` whose payload is `0`. The SAME desugar handles it — the synthesized
    // guard `(match __lc ((Wrap 0) …) (_ false))` tests the payload value (no disc test, but the kept
    // literal payload refines it; `ctor_pattern_with_wildcard_payloads` keeps a refutable payload), and the
    // body re-match binds nothing extra. So route a single-variant ctor with a refutable payload arg here
    // too. (The list-element analog of the single-variant sum-match literal-payload support, Inc-48/49.)
    let has_refutable_payload = match db.ast.get(elem_pat) {
        crate::ast::Struct::List(children) => children
            .clone()
            .iter()
            .skip(1) // skip the ctor head; the rest are payload args
            .any(|&arg| !ctor_payload_arg_is_irrefutable(db, arg)),
        // A bare-atom / bare-member single-variant ctor has no payload arg → irrefutable, not routed here.
        crate::ast::Struct::Atom(_) => false,
    };
    has_refutable_payload.then_some(head)
}

/// Whether a constructor-payload sub-pattern `arg` is IRREFUTABLE (always matches its sub-value): a bare
/// binder / `_`, or a tuple whose parts are irrefutable. A literal (`Int`/`Bool`/`Str`/`Float`/`Bytes`/
/// `Sym`/`Char`) or a nested constructor pattern is REFUTABLE. (Only `tuple` is special-cased below —
/// there is no record PATTERN form: a product is a single-variant sum matched as a `(tuple …)` payload, so
/// a "record" arm would be dead. If a first-class record pattern is ever added, extend the recursion here.)
/// Used to decide whether a single-variant ctor list-element needs the refutable-ctor desugar (a refutable
/// payload makes the otherwise-irrefutable newtype element refutable). A bare name in payload position
/// resolves INERT in a pattern (a fresh binder), so `as_name` detects it structurally without consulting
/// resolve.
pub(super) fn ctor_payload_arg_is_irrefutable(db: &mut Db, arg: StructId) -> bool {
    // A bare name (`x`) or `_` is the trivial irrefutable payload binder.
    if db.ast.as_name(arg).is_some() {
        return true;
    }
    // A tuple payload `(tuple a b)` is irrefutable iff every element is (recursively). Any other compound
    // (a nested ctor, a list pattern) or a literal is refutable. `compound_form_of` recognizes the native
    // `#tuple(…)` ctor-leaf head too (not only the name/string alias).
    if let Some(elems) = db
        .ast
        .compound_form_of(arg, crate::ast::CompoundCtor::Tuple)
    {
        let elems = elems.to_vec();
        return elems
            .iter()
            .all(|&e| ctor_payload_arg_is_irrefutable(db, e));
    }
    false
}

/// PRE-PASS for `lower_match_list` (the CONSTRUCTOR analogue of Inc-23's `desugar_saturating_bool_list_
/// elements`): a set of `(list <ctor-elem> .. rest)` arms whose position-0 constructors SATURATE the element
/// sum's variant set (EVERY variant present) JOINTLY covers every NON-EMPTY list — the first element is one
/// of the sum's variants, nothing else — so together with a `(list)` arm the match is total WITHOUT a `_`.
//= spec/capabilities/core-semantics.md#a-list-is-deconstructed-by-element-patterns-with-an-optional-rest
//# A set of list-element arms MUST be treated as exhaustive when it covers both the empty list and every non-empty list — for example an empty-list arm together with a leading-element-plus-rest arm, or an arm ending in a rest binder that names no leading elements — and a set of arms that leaves some length uncovered MUST be a compile-time error under *Matching Is Exhaustive Or Rejected* unless a later arm (a name or wildcard pattern) covers the remainder.
///
/// The bool case (Inc-23) replaces the last saturating arm's literal with `_` (a bool literal is a pure
/// test, binds nothing). A CTOR element binds a PAYLOAD, so it cannot be dropped — instead the LAST
/// saturating arm becomes UNCONDITIONAL by moving the payload-binding into its BODY (the ctor desugar's
/// body-rematch shape, minus the guard):
///   `((list (C.V p…) rest… .. r) body)`  [as the last saturating arm]  ≡
///   `((list __ls rest… .. r) (match __ls ((C.V p…) body) (_ (trap …))))`
/// The list pattern `(list __ls .. rest)` is now an UNGUARDED lead-1 rest — an unconditional cover the
/// length matcher counts (closing the non-empty tail). The body re-match binds `C.V`'s payload; its
/// `_ → trap` arm is UNREACHABLE by saturation + first-match-wins (reaching this arm means every OTHER
/// variant's earlier arm failed its discriminant, so — the sum's variants being exhausted — the element IS
/// `C.V`). The earlier saturating arms keep their ctor element → the ctor desugar makes them guarded
/// (discriminant-tested), so routing is unchanged. Fixes BOTH the exhaustiveness check and runtime totality.
///
/// Fires ONLY when (a) the element type is a `Ty::Sum` (or newtype) with a known variant set, (b) EVERY
/// variant appears at position 0 among BARE (unguarded) lead-1 `(list <ctor> .. rest)` arms — exactly one
/// leading element (a ctor of that sum) followed by a rest binder, and (c) there is NO existing unguarded
/// whole-tail cover (a bare `_`/name arm or a lead-0 `(list .. rest)`), else the match is already total and
/// the rewrite would only add a redundant arm (CDZ0213). A single-variant sum is irrefutable (handled
/// inline, never reaches here). Returns `Some(Core)` iff it fired (rebuilds the match, re-resolves, recurses).
/// Normalize a BARE-MEMBER ctor list-element `(. Sum V)` (a nullary variant written `C.R`) into the
/// PAREN-APPLIED form `((. Sum V))` — a 1-child list whose head is a FRESH copy of the member. A bare member
/// re-parented as a synthesized inner-match / guard-cond pattern head re-lowers as member ACCESS (CDZ0201)
/// in the emit path (a synthesized-member-pattern resolve gap the ctor desugar hits); the paren-applied
/// form resolves cleanly as a nullary-variant pattern (verified — `((C.R) …)` compiles + dispatches exactly
/// as `(C.V p…)` does). An APPLIED `(C.V p…)` / paren-nullary `(R)` element is already fine — returned
/// unchanged. The head is FRESH-cloned (`clone_ctor_head`) so it re-resolves out of its stale element-inert
/// context. Used by the saturating-ctor pass to make the bare-member spelling a first-class candidate.
pub(super) fn wrap_bare_member_element(db: &mut Db, elem: StructId) -> StructId {
    let is_bare_member = matches!(
        db.ast.get(elem),
        crate::ast::Struct::List(children)
            if children.first().is_some_and(|&h| db.ast.as_name(h) == Some("."))
    );
    if is_bare_member {
        let head = clone_ctor_head(db, elem);
        db.push_list(vec![head])
    } else {
        elem
    }
}

/// Rebuild the list pattern `pat` (`(list <elem0> …rest)`) with its LEADING element replaced by
/// `wrap_bare_member_element` — i.e. a bare-member `(. C V)` lead becomes `((. C V))`, everything else
/// unchanged. Preserves the `list` head token and every following element / rest binder. Used to normalize a
/// saturating arm's bare-member lead before it flows to the ctor desugar (earlier arms) or the body-rematch
/// (the last arm).
pub(super) fn list_pattern_with_wrapped_lead(db: &mut Db, pat: StructId) -> StructId {
    let Some(es) = db
        .ast
        .compound_form_of(pat, CompoundCtor::List)
        .map(<[_]>::to_vec)
    else {
        return pat;
    };
    if es.is_empty() {
        return pat;
    }
    let list_head = match db.ast.get(pat) {
        crate::ast::Struct::List(items) if !items.is_empty() => items[0],
        _ => db.push_name("list"),
    };
    let wrapped0 = wrap_bare_member_element(db, es[0]);
    let mut children = vec![list_head, wrapped0];
    children.extend(es[1..].iter().copied());
    db.push_list(children)
}

pub(super) fn desugar_saturating_ctor_list_elements(
    db: &mut Db,
    scrutinee: StructId,
    arms: &[(StructId, StructId)],
) -> Option<Core> {
    // The element sum's declaration + its FULL variant-name set. Bail unless the element type resolves to a
    // sum (or newtype) with a known decl — a non-sum element (or an unsolved `Any`) is not this case.
    let elem_ty = match crate::infer::type_of(db, scrutinee) {
        crate::ty::Ty::List(e) => (*e).clone(),
        _ => return None,
    };
    let decl = match elem_ty {
        crate::ty::Ty::Sum { decl, .. } | crate::ty::Ty::Nominal { decl, .. } => decl,
        _ => return None,
    };
    let all_variants: std::collections::BTreeSet<String> = db
        .type_decl_by_occ(decl)?
        .variants
        .iter()
        .map(|v| v.name.clone())
        .collect();
    // A single-variant sum's element is irrefutable (no discriminant to test), handled by the inline
    // binding path — not a saturation case.
    if all_variants.len() < 2 {
        return None;
    }
    // Classify each SOURCE arm. `ctor_lead[i]` = Some(variant-name) iff arm i is a BARE (unguarded)
    // `(list <ctor of this sum> .. <name>)` — exactly one leading element (a ctor) followed by a rest
    // binder. `has_tail_cover` = some UNGUARDED arm already covers the whole non-empty tail.
    let mut ctor_lead: Vec<Option<String>> = Vec::with_capacity(arms.len());
    let mut has_tail_cover = false;
    for &(pat, _) in arms {
        if db.ast.as_form(pat, "guard").is_some() {
            ctor_lead.push(None);
            continue;
        }
        if db.ast.as_name(pat).is_some() {
            has_tail_cover = true;
            ctor_lead.push(None);
            continue;
        }
        let Some(es) = db
            .ast
            .compound_form_of(pat, CompoundCtor::List)
            .map(<[_]>::to_vec)
        else {
            ctor_lead.push(None);
            continue;
        };
        match db
            .ast
            .rest_marker(&es)
            .map(|(i, _, trailing_start)| (i, trailing_start))
        {
            Some((0, _)) => {
                has_tail_cover = true;
                ctor_lead.push(None);
            }
            // A single leading element + rest, that element a refutable ctor of the element sum → a
            // saturating candidate; record the variant it names. A BARE-MEMBER element `(. C R)` (a nullary
            // variant written `C.R`) is a candidate too — the rewrite NORMALIZES it to the paren-applied form
            // `((. C R))` (which resolves cleanly as a variant pattern where the bare member re-lowers as
            // member ACCESS — a synthesized-member-pattern resolve gap), see `wrap_bare_member_element`.
            Some((1, trailing_start)) if trailing_start == es.len() => {
                match refutable_ctor_element_head(db, es[0]) {
                    Some(head) => ctor_lead.push(ctor_head_display_name(db, head)),
                    None => ctor_lead.push(None),
                }
            }
            _ => ctor_lead.push(None),
        }
    }
    if has_tail_cover {
        return None;
    }
    // Saturation: EVERY variant of the element sum appears among the candidate arms.
    let covered: std::collections::BTreeSet<String> = ctor_lead.iter().flatten().cloned().collect();
    if covered != all_variants {
        return None;
    }
    // The LAST saturating candidate (source order) — reaching it means every OTHER variant's earlier arm
    // failed its discriminant, so (the variants being exhausted) its element IS this arm's variant.
    let last = ctor_lead.iter().rposition(Option::is_some)?;
    // Rewrite that arm: `(list <ctor> .. rest)` → `(list __ls .. rest)` (unconditional lead-1 cover), and
    // wrap its body in `(match __ls (<ctor> body) (_ (trap)))` to bind the ctor's payload sub-patterns.
    let mut new_arms: Vec<StructId> = Vec::with_capacity(arms.len());
    for (ai, &(pat, body)) in arms.iter().enumerate() {
        if ai != last {
            // Reuse the earlier arm — but NORMALIZE a bare-member `(. C V)` lead element to `((. C V))` so
            // the ctor desugar (which processes these arms on the recursion) resolves it as a variant pattern
            // rather than mis-lowering it as member access. A non-saturating / non-bare-member arm is a no-op.
            let normalized = list_pattern_with_wrapped_lead(db, pat);
            new_arms.push(db.push_list(vec![normalized, body]));
            continue;
        }
        let es = db
            .ast
            .compound_form_of(pat, CompoundCtor::List)
            .map(<[_]>::to_vec)
            .expect("the saturating arm is a bare list pattern");
        let list_head = match db.ast.get(pat) {
            crate::ast::Struct::List(items) if !items.is_empty() => items[0],
            _ => db.push_name("list"),
        };
        let ctor_pat = es[0]; // the leading ctor element (es = [<ctor>, "..", rest])
        // Rebuild the ctor pattern with a FRESH head (via `clone_ctor_head`) — the original `C.V` head was
        // resolved in list-ELEMENT position (INERT), and reusing it as an inner MATCH-arm pattern head leaves
        // it mis-resolved (the member form needs to re-resolve as a variant reference from scratch — exactly
        // what `refutable_ctor_element_head` does). A bare-member `(. C V)` is additionally WRAPPED to the
        // paren-applied `((. C V))` form (`wrap_bare_member_element`), which resolves cleanly as a
        // nullary-variant pattern where the bare member alone re-lowers as member access. Payload
        // sub-patterns are reused (they re-resolve cleanly against the fresh binder scrutinee once forgotten).
        let body_ctor_pat = {
            let cloned = clone_ctor_pattern_head(db, ctor_pat);
            wrap_bare_member_element(db, cloned)
        };
        // The new list pattern: a fresh bare binder in place of the ctor element, keeping `.. rest`.
        let binder_name = format!("__ls{ai}");
        let binder = db.push_name(&binder_name);
        let mut list_children = vec![list_head, binder];
        list_children.extend(es[1..].iter().copied());
        let new_list = db.push_list(list_children);
        // The body re-match binds the ctor's payload: `(match __ls (<ctor> body) (_ (trap …)))`. The
        // `_ → trap` is dead by saturation but keeps the inner match exhaustive (mirrors the ctor desugar).
        let body_scrut = db.push_name(&binder_name);
        let body_true_arm = db.push_list(vec![body_ctor_pat, body]);
        let trap_head = db.push_name("trap");
        let trap_msg =
            db.push_str("unreachable: list-ctor-element variant set exhausted by earlier arms");
        let trap = db.push_list(vec![trap_head, trap_msg]);
        let wild_b = db.push_name("_");
        let body_false_arm = db.push_list(vec![wild_b, trap]);
        let body_match_head = db.push_name("match");
        let new_body = db.push_list(vec![
            body_match_head,
            body_scrut,
            body_true_arm,
            body_false_arm,
        ]);
        // The last arm's new body RE-PARENTS the original arm `body` (and its rest binder into `new_list`)
        // from a list-arm position into a fresh inner match. `resolved_of` is memoized and
        // `resolve_subtree`'s walk-guard skips an already-resolved node, so those re-parented nodes would
        // keep a stale resolution unless forgotten. Forget JUST this arm's new list + body (NOT the whole
        // match — the EARLIER arms are untouched originals whose member heads `(. C R)` are already
        // correctly resolved in element position, and blanket-forgetting them would break their
        // re-resolution). `resolve_subtree` below then recomputes the forgotten + fresh nodes in context.
        crate::resolve::forget_subtree(db, new_list);
        crate::resolve::forget_subtree(db, new_body);
        new_arms.push(db.push_list(vec![new_list, new_body]));
    }
    let match_head = db.push_name("match");
    let mut items = vec![match_head, scrutinee];
    items.extend(new_arms);
    let rewritten = db.push_list(items);
    crate::resolve::resolve_subtree(db, rewritten);
    trace!(target: "rcdzc::lower", scrutinee = scrutinee.0, "list match with saturating ctor first-elements → last arm unconditional + body re-match");
    Some(core_of(db, rewritten))
}

/// PRE-PASS for `lower_match_list` (the CONSTRUCTOR twin of `desugar_refutable_literal_list_elements`):
/// rewrite an arm whose list pattern has a refutable MULTI-VARIANT CONSTRUCTOR leading element into an
/// equivalent guarded arm the length-dispatch matcher already handles. This is the compiler tree-walk idiom
/// — matching a list of tagged nodes by the head's TAG: `(match instrs ((list (Op.Add x) .. r) …) ((list
/// (Op.Neg x) .. r) …) (_ …))`.
///
/// A ctor element `(C.V p…)` at leading position `i` is refutable (matches only a `C.V`-discriminant
/// element) AND binds payload sub-patterns — so unlike a literal (a pure `=` test) it needs BOTH a
/// discriminant test for fall-through AND payload binding for the body. The desugar splits those across the
/// guard and a body re-match, reusing the Inc-5 guard pipeline + the ordinary sum matcher:
///   `((list (C.V p…) rest… .. r) body)`  ≡
///   `((guard (list __lc rest… .. r) (match __lc ((C.V …wildcards) true) (_ false)))
///       (match __lc ((C.V p…) body) (_ (trap …))))`
/// The GUARD's discriminant test gates the arm (a non-`C.V` element → false → FALL THROUGH to the next arm,
/// exactly as the length-dispatch matcher wants); the BODY re-matches the SAME element binder to bind the
/// payload sub-patterns `p…` for `body` (the inner `_ → trap` arm is DEAD — the guard already proved the
/// discriminant — but keeps the inner match exhaustive). A guarded arm is EXCLUDED from length-coverage
/// exhaustiveness, so the outer match still needs a `_`/rest catch-all (correct — a discriminant test may
/// fail). Three distinct occurrences of the fresh name `__lc{arm}` (the Inc-11 two-parents lesson): the
/// inert PATTERN-position element binder, the guard-cond match scrutinee, and the body-rematch scrutinee.
///
/// N refutable-ctor elements per arm (N ≥ 1, `c5d45540`): each gets a fresh binder in the list pattern,
/// all their discriminant tests are ANDed into the arm guard, and the body re-matches NEST inside-out
/// (the innermost holds the original body, so every ctor's payload sub-patterns are in scope). The
/// single-ctor case is exactly N == 1, so the loop generalizes it uniformly. NO new IR, NO backend
/// change. Returns `Some(Core)` iff the rewrite fired.
pub(super) fn desugar_refutable_ctor_list_elements(
    db: &mut Db,
    scrutinee: StructId,
    arms: &[(StructId, StructId)],
) -> Option<Core> {
    // Detect whether ANY arm's list pattern has a refutable multi-variant-ctor LEADING element. Bail early
    // if none (the common case pays only this scan).
    let leading_of = |db: &mut Db, pat: StructId| -> Option<Vec<StructId>> {
        let inner = match db.ast.as_form(pat, "guard") {
            Some(g) if g.len() == 2 => g[0],
            _ => pat,
        };
        let es = db
            .ast
            .compound_form_of(inner, CompoundCtor::List)
            .map(<[_]>::to_vec)?;
        let lead = db
            .ast
            .rest_marker(&es)
            .map(|(i, _, _)| i)
            .unwrap_or(es.len());
        Some(es[..lead].to_vec())
    };
    let mut any_ctor = false;
    for &(pat, _) in arms {
        if let Some(leading) = leading_of(db, pat) {
            for e in leading {
                if refutable_ctor_element_head(db, e).is_some() {
                    any_ctor = true;
                }
            }
        }
    }
    if !any_ctor {
        return None;
    }
    // Rewrite each arm.
    let mut new_arms: Vec<StructId> = Vec::with_capacity(arms.len());
    for (ai, &(pat, body)) in arms.iter().enumerate() {
        let (inner, existing_guard) = match db.ast.as_form(pat, "guard") {
            Some(g) if g.len() == 2 => (g[0], Some(g[1])),
            _ => (pat, None),
        };
        let list_es = db
            .ast
            .compound_form_of(inner, CompoundCtor::List)
            .map(<[_]>::to_vec);
        let Some(es) = list_es else {
            new_arms.push(db.push_list(vec![pat, body]));
            continue;
        };
        let lead = db
            .ast
            .rest_marker(&es)
            .map(|(i, _, _)| i)
            .unwrap_or(es.len());
        // Locate the refutable-ctor leading element positions.
        let ctor_positions: Vec<usize> = (0..lead)
            .filter(|&p| refutable_ctor_element_head(db, es[p]).is_some())
            .collect();
        if ctor_positions.is_empty() {
            // No ctor element in this arm — reuse verbatim (a literal-only / irrefutable arm; a literal
            // element is handled by the literal desugar pass which runs after this one).
            new_arms.push(db.push_list(vec![pat, body]));
            continue;
        }
        // N refutable-ctor elements in one arm (N ≥ 1): give each a fresh binder in the list pattern, AND
        // all their discriminant-tests into the guard, and NEST one body re-match per ctor binder (the
        // innermost holds the original body, so every ctor payload sub-pattern is in scope for it). The
        // single-ctor case (N == 1) is exactly this with one binder — the loop generalizes it uniformly.
        let head = db.ast.get(inner);
        let list_head = match head {
            crate::ast::Struct::List(items) if !items.is_empty() => items[0],
            _ => db.push_name("list"),
        };
        // A fresh binder name per ctor position (`__lc{arm}_{k}`), paired with its original ctor pattern.
        let ctor_binders: Vec<(String, StructId)> = ctor_positions
            .iter()
            .enumerate()
            .map(|(k, &cpos)| (format!("__lc{ai}_{k}"), es[cpos]))
            .collect();
        // Rebuild the list pattern with EACH ctor element replaced by its fresh bare binder.
        let mut new_es: Vec<StructId> = Vec::with_capacity(es.len());
        for (p, &e) in es.iter().enumerate() {
            match ctor_positions.iter().position(|&cp| cp == p) {
                Some(k) => new_es.push(db.push_name(&ctor_binders[k].0)),
                None => new_es.push(e),
            }
        }
        let mut list_children = vec![list_head];
        list_children.extend(new_es);
        let new_list = db.push_list(list_children);
        // The DISCRIMINANT-TEST guard over every ctor binder. Each ctor binder `__lc_k` is tested by a
        // `(match __lc_k (<ctor-pat> <inner>) (_ false))`: the matched arm carries the ctor pattern so a
        // discriminant mismatch → `_ → false` (the arm falls through). Two shapes:
        //   - NO user guard: the matched arm is `(<ctor-with-wildcard-payloads> true)` — a pure disc test
        //     (payloads wildcarded; nothing reads them here, the body re-match binds them).
        //   - WITH a user guard reading a ctor PAYLOAD binder (`(guard (list (Op.Add n) .. r) (> n 3))`):
        //     the payload MUST be in scope for the user cond. So the INNERMOST disc-test's matched arm uses
        //     the REAL ctor pattern (binders live) and returns the USER COND (not `true`); enclosing
        //     disc-tests also use the real pattern so EVERY ctor's payload is in scope for the user cond.
        //     This binds `n` for the guard — the fix for the CDZ0101 false-reject on that idiom. (Before, the
        //     user cond was ANDed at the OUTER level where no payload binder is in scope.)
        let true_node = db.push_atom(crate::ast::Leaf::Bool(true));
        let false_node = db.push_atom(crate::ast::Leaf::Bool(false));
        // Nest the disc-tests INSIDE-OUT: the innermost matched arm holds the user cond (or `true`), each
        // enclosing disc-test binds its ctor's payload for the arm within. So every ctor payload binder is in
        // scope for the user cond, and any disc mismatch short-circuits to `false`.
        let mut inner_result = existing_guard.unwrap_or(true_node);
        for (bname, ctor_pat) in ctor_binders.iter().rev() {
            let disc_scrut = db.push_name(bname);
            // Use the REAL ctor pattern when a user guard is present (bind payloads for the cond); a pure
            // wildcard pattern otherwise (byte-identical to the pre-fix disc test — no payload binding).
            let matched_pat = if existing_guard.is_some() {
                clone_refutable_payload(db, *ctor_pat)
            } else {
                ctor_pattern_with_wildcard_payloads(db, *ctor_pat)
            };
            // `true`/`false` are BOOL LITERAL atoms (`Leaf::Bool`), NOT bare names — a `push_name("true")`
            // resolves fine in `check` but the emit path reports CDZ0101 "unbound name `true`".
            let disc_true_arm = db.push_list(vec![matched_pat, inner_result]);
            let wild = db.push_name("_");
            let disc_false_arm = db.push_list(vec![wild, false_node]);
            let disc_match_head = db.push_name("match");
            inner_result = db.push_list(vec![
                disc_match_head,
                disc_scrut,
                disc_true_arm,
                disc_false_arm,
            ]);
        }
        let guard_cond = inner_result;
        let guard_head = db.push_name("guard");
        let new_pat = db.push_list(vec![guard_head, new_list, guard_cond]);
        // The BODY re-match, NESTED from the INNERMOST ctor binder out: the innermost match holds the
        // original `body` (all ctor payloads in scope); each enclosing match binds its own ctor's payloads
        // and its body is the next-inner match. Building inside-out means each step wraps the accumulator.
        let mut new_body = body;
        for (bname, ctor_pat) in ctor_binders.iter().rev() {
            let body_scrut = db.push_name(bname);
            let body_true_arm = db.push_list(vec![*ctor_pat, new_body]);
            let trap_head = db.push_name("trap");
            let trap_msg =
                db.push_str("unreachable: list-ctor-element discriminant already gated by guard");
            let trap = db.push_list(vec![trap_head, trap_msg]);
            let wild_b = db.push_name("_");
            let body_false_arm = db.push_list(vec![wild_b, trap]);
            let body_match_head = db.push_name("match");
            new_body = db.push_list(vec![
                body_match_head,
                body_scrut,
                body_true_arm,
                body_false_arm,
            ]);
        }
        new_arms.push(db.push_list(vec![new_pat, new_body]));
    }
    let match_head = db.push_name("match");
    let mut items = vec![match_head, scrutinee];
    items.extend(new_arms);
    let rewritten = db.push_list(items);
    crate::resolve::resolve_subtree(db, rewritten);
    trace!(target: "rcdzc::lower", scrutinee = scrutinee.0, "list match with a refutable ctor element → fresh-binder + disc-test guard + body re-match");
    Some(core_of(db, rewritten))
}

/// If `elem_pat` is a REFUTABLE nested LIST leading element — a `(list …)` with ≥1 leading element before a
/// `..` (`(list a .. r1)`), or a fixed-arity `(list a b)` — return `Some((inner_lead, inner_is_fixed))`:
/// `inner_lead` = the number of leading element sub-patterns, `inner_is_fixed` = it has NO `..` (so it
/// matches an EXACT inner length). Such an element is length-refutable (a `(list a .. r1)` misses the empty
/// inner list, a `(list a b)` matches only inner length 2), so it needs an INNER-LENGTH guard — see
/// `desugar_refutable_nested_list_elements`. The ZERO-LEADING rest form `(list .. r)` matches every inner
/// list (irrefutable, handled inline by `list_element_irrefutable_or_decline`) → `None`.
pub(super) fn refutable_nested_list_element(db: &Db, elem_pat: StructId) -> Option<(usize, bool)> {
    let es = db.ast.compound_form_of(elem_pat, CompoundCtor::List)?;
    match db.ast.rest_marker(es).map(|(i, _, _)| i) {
        // A rest form: refutable iff ≥1 leading element (misses the empty inner list); zero-leading is
        // irrefutable (returns None). `inner_lead` = the count before `..`, not fixed.
        Some(k) if k > 0 => Some((k, false)),
        Some(_) => None,
        // No `..` → fixed-arity: refutable iff non-empty (matches only that exact inner length). The empty
        // fixed pattern `(list)` matches only the empty inner list — also refutable (inner_lead 0, fixed).
        None => Some((es.len(), true)),
    }
}

/// PRE-PASS for `lower_match_list` (the NESTED-LIST twin of `desugar_refutable_ctor_list_elements`):
/// rewrite an arm whose list pattern has a refutable nested LIST leading element into an equivalent guarded
/// arm the length-dispatch matcher handles. A nested list element `(list p… .. r1)` at a leading position
/// dispatches on the INNER list's length, which the outer length-dispatch matcher cannot express — so it
/// desugars to a fresh binder + an INNER-LENGTH guard + a body RE-MATCH binding the inner sub-patterns, the
/// list-element analogue of the ctor desugar:
///   `((list (list p… .. r1) rest… .. r2) body)`  ≡
///   `((guard (list __ne rest… .. r2) (>= ((. List len) __ne) k))
///       (match __ne ((list p… .. r1) body) (_ (trap …))))`
/// where `k` = the inner leading count and the guard is `(= … k)` for a FIXED-arity inner list (exact
/// length) or `(>= … k)` for a rest inner list (length ≥ k). The guard gates the arm (an inner list too
/// short → false → FALL THROUGH, not a trap); the body re-matches the SAME binder to bind the inner
/// elements — and there the inner `(list p… .. r1)` IS irrefutable given the length guard held, so the sum
/// matcher's own length dispatch is satisfied (the `_ → trap` arm is dead). A guarded arm is EXCLUDED from
/// length-coverage exhaustiveness, so the outer match still needs a `_`/rest catch-all. Scope: ONE
/// refutable nested-list element per arm (the common shape); ≥2 declines honestly. NO new IR.
pub(super) fn desugar_refutable_nested_list_elements(
    db: &mut Db,
    scrutinee: StructId,
    arms: &[(StructId, StructId)],
) -> Option<Core> {
    let leading_of = |db: &mut Db, pat: StructId| -> Option<Vec<StructId>> {
        let inner = match db.ast.as_form(pat, "guard") {
            Some(g) if g.len() == 2 => g[0],
            _ => pat,
        };
        let es = db
            .ast
            .compound_form_of(inner, CompoundCtor::List)
            .map(<[_]>::to_vec)?;
        let lead = db
            .ast
            .rest_marker(&es)
            .map(|(i, _, _)| i)
            .unwrap_or(es.len());
        Some(es[..lead].to_vec())
    };
    let mut any = false;
    for &(pat, _) in arms {
        if let Some(leading) = leading_of(db, pat) {
            for e in leading {
                if refutable_nested_list_element(db, e).is_some() {
                    any = true;
                }
            }
        }
    }
    if !any {
        return None;
    }
    let mut new_arms: Vec<StructId> = Vec::with_capacity(arms.len());
    for (ai, &(pat, body)) in arms.iter().enumerate() {
        let (inner, existing_guard) = match db.ast.as_form(pat, "guard") {
            Some(g) if g.len() == 2 => (g[0], Some(g[1])),
            _ => (pat, None),
        };
        let list_es = db
            .ast
            .compound_form_of(inner, CompoundCtor::List)
            .map(<[_]>::to_vec);
        let Some(es) = list_es else {
            new_arms.push(db.push_list(vec![pat, body]));
            continue;
        };
        let lead = db
            .ast
            .rest_marker(&es)
            .map(|(i, _, _)| i)
            .unwrap_or(es.len());
        let nested_positions: Vec<usize> = (0..lead)
            .filter(|&p| refutable_nested_list_element(db, es[p]).is_some())
            .collect();
        if nested_positions.is_empty() {
            new_arms.push(db.push_list(vec![pat, body]));
            continue;
        }
        if nested_positions.len() > 1 {
            return Some(Core::Poison(Reject::decline(
                "a list arm with more than one refutable nested-list element is not supported \
                 (match one nested list per arm)",
            )));
        }
        let npos = nested_positions[0];
        let nested_pat = es[npos]; // the original `(list p… .. r1)` inner pattern
        let (inner_lead, inner_fixed) = refutable_nested_list_element(db, nested_pat).unwrap();
        let head = db.ast.get(inner);
        let list_head = match head {
            crate::ast::Struct::List(items) if !items.is_empty() => items[0],
            _ => db.push_name("list"),
        };
        let name = format!("__ne{ai}");
        let mut new_es: Vec<StructId> = Vec::with_capacity(es.len());
        for (p, &e) in es.iter().enumerate() {
            if p == npos {
                new_es.push(db.push_name(&name)); // the inert pattern-position element binder
            } else {
                new_es.push(e);
            }
        }
        let mut list_children = vec![list_head];
        list_children.extend(new_es);
        let new_list = db.push_list(list_children);
        // The INNER-LENGTH guard: `((op) ((. List len) __ne) k)` — `>=` for a rest inner list (length ≥ k),
        // `=` for a fixed-arity inner list (exact length). Fresh `__ne` occurrence for the `List.len` arg.
        let len_scrut = db.push_name(&name);
        let dot = db.push_name(".");
        let list_mod = db.push_name("List");
        let len_key = db.push_name("len");
        let len_member = db.push_list(vec![dot, list_mod, len_key]);
        let len_call = db.push_list(vec![len_member, len_scrut]);
        let k_lit = db.push_atom(crate::ast::Leaf::Int {
            value: crate::ast::IntValue::from_u128(inner_lead as u128),
            radix: crate::ast::Radix::Dec,
        });
        let cmp_op = db.push_name(if inner_fixed { "=" } else { ">=" });
        let len_test = db.push_list(vec![cmp_op, len_call, k_lit]);
        let guard_cond = match existing_guard {
            None => {
                // The inner-length test ALONE is not enough when the nested pattern has REFUTABLE CONTENT
                // (a literal / ctor element, e.g. `(list 1 b)`): the length guard passes for ANY 2-element
                // inner list, the arm fires, and the body re-match `(match __ne ((list 1 b) body) (_ (trap)))`
                // hits its `_ → trap` on a NON-matching element (inner `(2 …)`) — a SILENT TRAP miscompile on
                // what should FALL THROUGH to a sibling / wildcard arm (breaker: nested-list-with-literal).
                // So ALSO test the full inner pattern in the guard — `(match __ne (<nested-clone> true)
                // (_ false))` — so a non-matching element makes the guard FALSE (fall through) and the body
                // re-match's `_ → trap` is genuinely dead. For a binders-only inner pattern this match is true
                // whenever the length matches, so it is a harmless no-op there (same behavior as len_test
                // alone). Mirrors the `Some(g)` arm below (which already matches the inner pattern to bind
                // `g`'s binders) — here the guard body is just `true`. A CLONE of the nested pattern (the body
                // re-match reuses the original; a node has one parent).
                let content_scrut = db.push_name(&name);
                let nested_clone = clone_refutable_payload(db, nested_pat);
                let true_body = db.push_atom(crate::ast::Leaf::Bool(true));
                let content_true_arm = db.push_list(vec![nested_clone, true_body]);
                let content_wild = db.push_name("_");
                let content_false = db.push_atom(crate::ast::Leaf::Bool(false));
                let content_false_arm = db.push_list(vec![content_wild, content_false]);
                let content_match_head = db.push_name("match");
                let content_match = db.push_list(vec![
                    content_match_head,
                    content_scrut,
                    content_true_arm,
                    content_false_arm,
                ]);
                let and_head = db.push_name("and");
                db.push_list(vec![and_head, len_test, content_match])
            }
            Some(g) => {
                // The user guard cond `g` may read the INNER list's binders (`(guard (list (list a .. r1)
                // .. r2) (> a 3))` reads `a`). Those bind only in the BODY re-match (below), NOT at the outer
                // guard level — so ANDing `g` here left `a` unbound (a false CDZ0101). FIX (mirrors Inc-34's
                // ctor-element guard): evaluate `g` INSIDE a match on the fresh binder `__ne` that binds the
                // inner pattern — `(match __ne (<nested-pat> g) (_ false))` — so the inner binders are in
                // scope for `g`; the `_ → false` is dead (the length test already gated it) but keeps the
                // inner match well-formed and returns a bool. Conjoin with the length test:
                // `(and <len_test> (match __ne (<nested-pat> g) (_ false)))`. A CLONE of the nested pattern is
                // used (the body re-match reuses the original; a node has one parent).
                let g_scrut = db.push_name(&name);
                let nested_clone = clone_refutable_payload(db, nested_pat);
                let g_true_arm = db.push_list(vec![nested_clone, g]);
                let g_wild = db.push_name("_");
                let g_false = db.push_atom(crate::ast::Leaf::Bool(false));
                let g_false_arm = db.push_list(vec![g_wild, g_false]);
                let g_match_head = db.push_name("match");
                let g_in_scope = db.push_list(vec![g_match_head, g_scrut, g_true_arm, g_false_arm]);
                let and_head = db.push_name("and");
                db.push_list(vec![and_head, len_test, g_in_scope])
            }
        };
        let guard_head = db.push_name("guard");
        let new_pat = db.push_list(vec![guard_head, new_list, guard_cond]);
        // The BODY re-match: `(match __ne (<original nested list pattern> body) (_ (trap …)))` — binds the
        // inner list's sub-patterns for the body; the inner pattern is irrefutable GIVEN the length guard, so
        // the `_ → trap` arm is dead but keeps the inner match well-formed.
        let body_scrut = db.push_name(&name);
        let body_true_arm = db.push_list(vec![nested_pat, body]);
        let trap_head = db.push_name("trap");
        let trap_msg =
            db.push_str("unreachable: nested-list-element length already gated by guard");
        let trap = db.push_list(vec![trap_head, trap_msg]);
        let wild_b = db.push_name("_");
        let body_false_arm = db.push_list(vec![wild_b, trap]);
        let body_match_head = db.push_name("match");
        let new_body = db.push_list(vec![
            body_match_head,
            body_scrut,
            body_true_arm,
            body_false_arm,
        ]);
        new_arms.push(db.push_list(vec![new_pat, new_body]));
    }
    let match_head = db.push_name("match");
    let mut items = vec![match_head, scrutinee];
    items.extend(new_arms);
    let rewritten = db.push_list(items);
    crate::resolve::resolve_subtree(db, rewritten);
    trace!(target: "rcdzc::lower", scrutinee = scrutinee.0, "list match with a refutable nested-list element → fresh-binder + inner-length guard + body re-match");
    Some(core_of(db, rewritten))
}

/// A FRESH deep copy of a refutable payload sub-pattern (a literal atom, or a nested compound like `(Some
/// 0)` / `(. Sum V)`) for reuse in the guard's discriminant-test pattern. A node has ONE parent and
/// `push_list` reparents, so the guard cannot share the ORIGINAL sub-pattern (the body re-match reuses it) —
/// this rebuilds it as an independent subtree (atoms via a leaf copy, lists rebuilt child-by-child). Only
/// used for a KEPT (non-bare-binder) payload, so it never needs to preserve a binder's identity.
pub(super) fn clone_refutable_payload(db: &mut Db, e: StructId) -> StructId {
    match db.ast.get(e) {
        crate::ast::Struct::Atom(lid) => {
            let leaf = db.ast.leaf(*lid).clone();
            db.push_atom(leaf)
        }
        crate::ast::Struct::List(children) => {
            let children = children.clone();
            let cloned: Vec<StructId> = children
                .iter()
                .map(|&c| clone_refutable_payload(db, c))
                .collect();
            db.push_list(cloned)
        }
    }
}

/// A copy of the constructor pattern `ctor_pat` (`(C.V p0 p1)` / `(. Sum V)` / a bare variant name) for the
/// guard's discriminant test, with every BARE-BINDER payload argument replaced by a wildcard `_` but every
/// REFUTABLE payload sub-pattern (a literal, a nested constructor) KEPT.
///
/// Why keep refutable payloads: the guard's job is to decide whether this arm's element pattern matches
/// so the arm fires or FALLS THROUGH. The body re-match `(match __lc (<ctor-pat> body) (_ (trap)))` traps in
/// its `_` arm, so the guard MUST have already proven the FULL element pattern matches — not just the
/// discriminant. If a payload sub-pattern is a LITERAL (`(Op.Add 0)`), wildcarding it made the guard pass on
/// discriminant alone (`Op.Add` present) for an element whose actual payload is `5`, then the body re-match
/// `(Op.Add 0)` failed to match `(Op.Add 5)` and hit the `_ → trap` — a SILENT TRAP miscompile on what
/// should be a clean fall-through to a sibling `(Op.Add n)` arm (two same-variant arms refining the payload
/// by different literals). Keeping the literal in the guard test makes the guard FALSE for payload `5`, so
/// control falls through correctly and the body-rematch trap is genuinely dead.
///
/// A BARE-BINDER payload (`(Op.Add n)`) is still wildcarded — it matches any value, so the guard should not
/// bind it (the body re-match binds `n`); wildcarding keeps the guard a pure discriminant+refinement test.
/// A bare-name / bare-member ctor (nullary, or a whole `(. Sum V)`) has no payload args, reused verbatim.
pub(super) fn ctor_pattern_with_wildcard_payloads(db: &mut Db, ctor_pat: StructId) -> StructId {
    match db.ast.get(ctor_pat) {
        crate::ast::Struct::List(children) => {
            let children = children.clone();
            match children.first().copied() {
                // A bare member `(. Sum V)` used whole — no payload args, reuse verbatim.
                Some(first) if db.ast.as_name(first) == Some(".") => ctor_pat,
                Some(head) => {
                    let mut new_children = vec![head];
                    for &arg in &children[1..] {
                        // A BARE BINDER / `_` payload → wildcard it (the guard should not bind, and it
                        // matches any value so it need not be tested). A REFUTABLE payload (a literal, a
                        // nested ctor — anything NOT a bare name) is CLONED and kept (a fresh subtree, since
                        // the ORIGINAL arg is reused by the body re-match — a node has one parent), so the
                        // guard tests it and a mismatch FALLS THROUGH instead of trapping in the body re-match.
                        if db.ast.as_name(arg).is_some() {
                            new_children.push(db.push_name("_"));
                        } else {
                            new_children.push(clone_refutable_payload(db, arg));
                        }
                    }
                    db.push_list(new_children)
                }
                None => ctor_pat,
            }
        }
        // A bare-name nullary ctor — no payload, reuse verbatim.
        crate::ast::Struct::Atom(_) => ctor_pat,
    }
}

/// A wildcard-VALUE, rest-stripped copy of a map pattern `(map (k v)… .. r)` → `(map (k _)…)`: keeps each
/// entry's KEY, replaces its value binder with `_`, and drops any `.. rest`. Used to build the
/// key-presence guard for a refutable map list-element (test ONLY that the named keys are present, no value
/// binding, no rest). The keys are reused live (a key is an ordinary value expression); the values become
/// fresh `_` nodes.
pub(super) fn map_pattern_with_wildcard_values(db: &mut Db, map_pat: StructId) -> StructId {
    let tail = db
        .ast
        .compound_form_of(map_pat, CompoundCtor::Map)
        .map(<[_]>::to_vec)
        .unwrap_or_default();
    let map_head = match db.ast.get(map_pat) {
        crate::ast::Struct::List(items) if !items.is_empty() => items[0],
        _ => db.push_name("map"),
    };
    let mut children = vec![map_head];
    for &entry in &tail {
        // Stop at a `..` marker (either the flat `..` or the wrapped `(.. rest)` node) — the presence guard
        // tests only the NAMED keys, not the rest.
        if db.ast.as_name(entry) == Some("..") || db.ast.as_form(entry, "..").is_some() {
            break;
        }
        // Extract the entry's KEY, handling the native `FieldPair (= k v)` (3-element) + the name-head
        // `(= k v)` alias + the legacy positional `(k v)` (2-element). The native `#map` element's FieldPair
        // entry was previously SKIPPED (only `kv.len() == 2` matched) → the presence guard got an EMPTY map
        // pattern that gates nothing → an ABSENT key passed the guard, and the body re-match then trapped
        // ("keys already gated by guard") instead of the arm failing + falling through (05 case 0367).
        let key = db
            .ast
            .field_pair_parts(entry)
            .or_else(|| db.ast.field_pair(entry))
            .map(|(k, _)| k)
            .or_else(|| match db.ast.get(entry) {
                crate::ast::Struct::List(kv) if kv.len() == 2 => Some(kv[0]),
                _ => None,
            });
        if let Some(key) = key {
            let wild = db.push_name("_");
            let new_entry = db.push_list(vec![key, wild]);
            children.push(new_entry);
        }
    }
    db.push_list(children)
}

/// Whether `elem_pat` is a MAP pattern `(map (k v)… .. r)` — a refutable list-arm element (it matches only
/// a map containing the named keys). Like a refutable ctor element, it desugars to a fresh binder + a
/// key-presence-test guard + a body that re-matches the binder to bind the values
/// (`desugar_refutable_map_list_elements`). A non-map element returns `false`.
pub(super) fn is_map_element_pattern(db: &Db, elem_pat: StructId) -> bool {
    db.ast
        .compound_form_of(elem_pat, CompoundCtor::Map)
        .is_some()
}

/// PRE-PASS for `lower_match_list` (the MAP twin of `desugar_refutable_ctor_list_elements`): rewrite an arm
/// whose list pattern has a refutable MAP leading element `(list (map (k v)…) rest… .. r)` into an
/// equivalent guarded arm the length-dispatch matcher + the (direct) map matcher already handle. A map
/// element matches only a map containing its named keys AND binds the values — so, like a ctor element, it
/// needs BOTH a presence test for fall-through AND value binding for the body:
///   `((list (map (k v)…) rest… .. r) body)`  ≡
///   `((guard (list __lc rest… .. r) (match __lc ((map (k _)…) true) (_ false)))
///       (match __lc ((map (k v)…) body) (_ (trap …))))`
/// The GUARD's key-presence test (a wildcard-VALUE map pattern) gates the arm; the BODY re-matches the SAME
/// element binder to bind the values `v…` (the DIRECT map matcher — `lower_match_map` / the `MapField`
/// resolution — binds them). One refutable-map element per arm (the common shape); ≥2 declines. NO new IR.
/// Returns `Some(Core)` iff the rewrite fired.
pub(super) fn desugar_refutable_map_list_elements(
    db: &mut Db,
    scrutinee: StructId,
    arms: &[(StructId, StructId)],
) -> Option<Core> {
    // The leading elements of an arm's (optionally guarded) list pattern — `None` if not a list pattern.
    let leading_of = |db: &mut Db, pat: StructId| -> Option<Vec<StructId>> {
        let inner = match db.ast.as_form(pat, "guard") {
            Some(g) if g.len() == 2 => g[0],
            _ => pat,
        };
        let es = db
            .ast
            .compound_form_of(inner, CompoundCtor::List)
            .map(<[_]>::to_vec)?;
        let lead = db
            .ast
            .rest_marker(&es)
            .map(|(i, _, _)| i)
            .unwrap_or(es.len());
        Some(es[..lead].to_vec())
    };
    // Bail early unless some arm has a MAP leading element (the common case pays only this scan).
    let mut any_map = false;
    for &(pat, _) in arms {
        if let Some(leading) = leading_of(db, pat)
            && leading.iter().any(|&e| is_map_element_pattern(db, e))
        {
            any_map = true;
        }
    }
    if !any_map {
        return None;
    }
    let mut new_arms: Vec<StructId> = Vec::with_capacity(arms.len());
    for (ai, &(pat, body)) in arms.iter().enumerate() {
        let (inner, existing_guard) = match db.ast.as_form(pat, "guard") {
            Some(g) if g.len() == 2 => (g[0], Some(g[1])),
            _ => (pat, None),
        };
        let list_es = db
            .ast
            .compound_form_of(inner, CompoundCtor::List)
            .map(<[_]>::to_vec);
        let Some(es) = list_es else {
            new_arms.push(db.push_list(vec![pat, body]));
            continue;
        };
        let lead = db
            .ast
            .rest_marker(&es)
            .map(|(i, _, _)| i)
            .unwrap_or(es.len());
        let map_positions: Vec<usize> = (0..lead)
            .filter(|&p| is_map_element_pattern(db, es[p]))
            .collect();
        if map_positions.is_empty() {
            new_arms.push(db.push_list(vec![pat, body]));
            continue;
        }
        if map_positions.len() > 1 {
            // ≥2 map elements in one arm — the body-rematch nesting is a later increment. Decline honestly.
            return Some(Core::Poison(Reject::decline(
                "a list arm with more than one map element is not supported (match one map element per arm)",
            )));
        }
        let mpos = map_positions[0];
        let map_pat = es[mpos]; // the original `(map (k v)…)` element pattern
        let list_head = match db.ast.get(inner) {
            crate::ast::Struct::List(items) if !items.is_empty() => items[0],
            _ => db.push_name("list"),
        };
        let name = format!("__lm{ai}");
        // Rebuild the list pattern with the map element replaced by a fresh bare binder.
        let mut new_es: Vec<StructId> = Vec::with_capacity(es.len());
        for (p, &e) in es.iter().enumerate() {
            if p == mpos {
                new_es.push(db.push_name(&name));
            } else {
                new_es.push(e);
            }
        }
        let mut list_children = vec![list_head];
        list_children.extend(new_es);
        let new_list = db.push_list(list_children);
        // The KEY-PRESENCE guard: `(match __lm ((map (k _)…) true) (_ false))` — a wildcard-value map pattern
        // tests only that the named keys are present (the `MapHasKeys` gate), no value binding in the guard.
        let presence_scrut = db.push_name(&name);
        let presence_pat = map_pattern_with_wildcard_values(db, map_pat);
        let true_node = db.push_atom(crate::ast::Leaf::Bool(true));
        let false_node = db.push_atom(crate::ast::Leaf::Bool(false));
        let presence_true_arm = db.push_list(vec![presence_pat, true_node]);
        let wild = db.push_name("_");
        let presence_false_arm = db.push_list(vec![wild, false_node]);
        let presence_match_head = db.push_name("match");
        let presence_test = db.push_list(vec![
            presence_match_head,
            presence_scrut,
            presence_true_arm,
            presence_false_arm,
        ]);
        let guard_cond = match existing_guard {
            None => presence_test,
            Some(g) => {
                let and_head = db.push_name("and");
                db.push_list(vec![and_head, g, presence_test])
            }
        };
        let guard_head = db.push_name("guard");
        let new_pat = db.push_list(vec![guard_head, new_list, guard_cond]);
        // The BODY re-match: `(match __lm ((map (k v)…) <body>) (_ (trap …)))` — the DIRECT map matcher binds
        // the value sub-patterns for the original body; the `_` arm is dead (the guard proved key presence)
        // but keeps the inner match exhaustive.
        let body_scrut = db.push_name(&name);
        let body_true_arm = db.push_list(vec![map_pat, body]);
        let trap_head = db.push_name("trap");
        let trap_msg = db.push_str("unreachable: list-map-element keys already gated by guard");
        let trap = db.push_list(vec![trap_head, trap_msg]);
        let wild_b = db.push_name("_");
        let body_false_arm = db.push_list(vec![wild_b, trap]);
        let body_match_head = db.push_name("match");
        let new_body = db.push_list(vec![
            body_match_head,
            body_scrut,
            body_true_arm,
            body_false_arm,
        ]);
        new_arms.push(db.push_list(vec![new_pat, new_body]));
    }
    let match_head = db.push_name("match");
    let mut items = vec![match_head, scrutinee];
    items.extend(new_arms);
    let rewritten = db.push_list(items);
    crate::resolve::resolve_subtree(db, rewritten);
    trace!(target: "rcdzc::lower", scrutinee = scrutinee.0, "list match with a refutable map element → fresh-binder + key-presence guard + body re-match");
    Some(core_of(db, rewritten))
}

/// Whether `elem_pat` is a REFUTABLE TUPLE list-element pattern — a `(tuple c0 c1 …)` with ≥1 component
/// that `check_binding_pattern` flags NON-EXHAUSTIVE (a literal atom or a multi-variant constructor: the
/// only sub-patterns that can fail to match a well-typed value). A binder-only tuple `(tuple a b)`, and any
/// tuple whose non-name components are themselves IRREFUTABLE destructures — a record `(record (= f x))`, a
/// nested tuple, a single-variant ctor — is irrefutable (it matches any value of that shape, bound inline by
/// `check_binding_pattern`), so it returns `false` and is left on the pre-existing irrefutable path; only a
/// genuinely refutable component like the `1` in `(tuple 1 b)` needs the value-refinement desugar (the tuple
/// twin of `is_map_element_pattern` / `refutable_ctor_element_head`). A non-tuple element returns `false`.
///
/// Refutability is the AUTHORITATIVE `check_binding_pattern` `Err(NonExhaustive)` signal (the same predicate
/// `list_element_irrefutable_or_decline` / `map_value_irrefutable_or_decline` use), NOT a coarse
/// `as_name().is_none()` structural test: the structural test mis-flagged an irrefutable `(record (= v a))`
/// component as refutable and wrongly routed it through the value-refinement desugar (→ CDZ0900 decline),
/// regressing `dst2` (an irrefutable list-of-tuples-of-records arm) from pass in #8367. Any OTHER
/// `check_binding_pattern` error (a shape mismatch, non-linearity) is not a refutability signal → not routed
/// here (it surfaces on the pre-existing irrefutable path with its true code).
pub(super) fn is_refutable_tuple_element(db: &mut Db, elem_pat: StructId) -> bool {
    let Some(comps) = db
        .ast
        .compound_form_of(elem_pat, CompoundCtor::Tuple)
        .map(<[_]>::to_vec)
    else {
        return false;
    };
    comps.iter().any(|&c| {
        matches!(
            check_binding_pattern(db, c, &crate::ty::Ty::Any),
            Err(ref r) if r.code == Some(Code::NonExhaustive)
        )
    })
}

/// PRE-PASS for `lower_match_list` (the TUPLE twin of `desugar_refutable_map_list_elements` /
/// `desugar_refutable_nested_list_elements`): rewrite an arm whose list pattern has a refutable TUPLE leading
/// element `(list (tuple 1 b) rest… .. r)` into an equivalent guarded arm the length-dispatch matcher + the
/// (direct) tuple matcher already handle. A tuple has FIXED arity (no length dispatch needed — the type
/// fixes it), so the ONLY refutation is its literal/ctor components; it needs a value-test guard for
/// fall-through AND value binding for the body:
///   `((list (tuple 1 b) rest… .. r) body)`  ≡
///   `((guard (list __lt rest… .. r) (match __lt ((tuple 1 _) true) (_ false)))
///       (match __lt ((tuple 1 b) body) (_ (trap …))))`
/// The GUARD's value test (a wildcard-BINDER tuple pattern via `ctor_pattern_with_wildcard_payloads` — keeps
/// the literal `1`, wildcards the binder `b`) gates the arm so a non-matching literal FALLS THROUGH; the BODY
/// re-matches the SAME element binder to bind `b` for the original body. One refutable-tuple element per arm
/// (the common shape); ≥2 declines. NO new IR. Returns `Some(Core)` iff the rewrite fired. (Completes the
/// refutable-list-element-refinement arc — the last element kind after ctor/map/nested-list; #8364.)
pub(super) fn desugar_refutable_tuple_list_elements(
    db: &mut Db,
    scrutinee: StructId,
    arms: &[(StructId, StructId)],
) -> Option<Core> {
    let leading_of = |db: &mut Db, pat: StructId| -> Option<Vec<StructId>> {
        let inner = match db.ast.as_form(pat, "guard") {
            Some(g) if g.len() == 2 => g[0],
            _ => pat,
        };
        let es = db
            .ast
            .compound_form_of(inner, CompoundCtor::List)
            .map(<[_]>::to_vec)?;
        let lead = db
            .ast
            .rest_marker(&es)
            .map(|(i, _, _)| i)
            .unwrap_or(es.len());
        Some(es[..lead].to_vec())
    };
    let mut any_tuple = false;
    for &(pat, _) in arms {
        if let Some(leading) = leading_of(db, pat)
            && leading.iter().any(|&e| is_refutable_tuple_element(db, e))
        {
            any_tuple = true;
        }
    }
    if !any_tuple {
        return None;
    }
    let mut new_arms: Vec<StructId> = Vec::with_capacity(arms.len());
    for (ai, &(pat, body)) in arms.iter().enumerate() {
        let (inner, existing_guard) = match db.ast.as_form(pat, "guard") {
            Some(g) if g.len() == 2 => (g[0], Some(g[1])),
            _ => (pat, None),
        };
        let list_es = db
            .ast
            .compound_form_of(inner, CompoundCtor::List)
            .map(<[_]>::to_vec);
        let Some(es) = list_es else {
            new_arms.push(db.push_list(vec![pat, body]));
            continue;
        };
        let lead = db
            .ast
            .rest_marker(&es)
            .map(|(i, _, _)| i)
            .unwrap_or(es.len());
        let tuple_positions: Vec<usize> = (0..lead)
            .filter(|&p| is_refutable_tuple_element(db, es[p]))
            .collect();
        if tuple_positions.is_empty() {
            new_arms.push(db.push_list(vec![pat, body]));
            continue;
        }
        if tuple_positions.len() > 1 {
            return Some(Core::Poison(Reject::decline(
                "a list arm with more than one refutable tuple element is not supported (match one \
                 refutable tuple element per arm)",
            )));
        }
        let tpos = tuple_positions[0];
        let tuple_pat = es[tpos]; // the original `(tuple 1 b)` element pattern
        let list_head = match db.ast.get(inner) {
            crate::ast::Struct::List(items) if !items.is_empty() => items[0],
            _ => db.push_name("list"),
        };
        let name = format!("__lt{ai}");
        // Rebuild the list pattern with the tuple element replaced by a fresh bare binder.
        let mut new_es: Vec<StructId> = Vec::with_capacity(es.len());
        for (p, &e) in es.iter().enumerate() {
            if p == tpos {
                new_es.push(db.push_name(&name));
            } else {
                new_es.push(e);
            }
        }
        let mut list_children = vec![list_head];
        list_children.extend(new_es);
        let new_list = db.push_list(list_children);
        // The VALUE-test guard: `(match __lt ((tuple 1 _) true) (_ false))` — a wildcard-BINDER tuple pattern
        // (keeps the literal `1`, wildcards the binder `b`) tests ONLY the refutable components, no value
        // binding in the guard, so a non-matching literal FALLS THROUGH.
        let test_scrut = db.push_name(&name);
        let test_pat = ctor_pattern_with_wildcard_payloads(db, tuple_pat);
        let true_node = db.push_atom(crate::ast::Leaf::Bool(true));
        let false_node = db.push_atom(crate::ast::Leaf::Bool(false));
        let test_true_arm = db.push_list(vec![test_pat, true_node]);
        let wild = db.push_name("_");
        let test_false_arm = db.push_list(vec![wild, false_node]);
        let test_match_head = db.push_name("match");
        let value_test = db.push_list(vec![
            test_match_head,
            test_scrut,
            test_true_arm,
            test_false_arm,
        ]);
        let guard_cond = match existing_guard {
            None => value_test,
            Some(g) => {
                // The user guard cond `g` may read the tuple's COMPONENT binders (`(guard (list (tuple 1 b))
                // (> b 3))` reads `b`). Those bind only in the BODY re-match (below), NOT at the outer guard
                // level — so ANDing `g` with the value_test left `b` unbound (a false CDZ0101). Mirror the
                // nested-list desugar: evaluate `g` INSIDE a match on `__lt` that binds the FULL tuple pattern
                // — `(match __lt ((tuple 1 b) g) (_ false))` — so `b` is in scope for `g`, and the literal `1`
                // is tested by the same match (subsuming value_test: a non-matching literal → `_ → false` →
                // fall through). A FULL clone of the tuple pattern (the body re-match reuses the original; a
                // node has one parent).
                let g_scrut = db.push_name(&name);
                let g_clone = clone_refutable_payload(db, tuple_pat);
                let g_true_arm = db.push_list(vec![g_clone, g]);
                let g_wild = db.push_name("_");
                let g_false = db.push_atom(crate::ast::Leaf::Bool(false));
                let g_false_arm = db.push_list(vec![g_wild, g_false]);
                let g_match_head = db.push_name("match");
                db.push_list(vec![g_match_head, g_scrut, g_true_arm, g_false_arm])
            }
        };
        let guard_head = db.push_name("guard");
        let new_pat = db.push_list(vec![guard_head, new_list, guard_cond]);
        // The BODY re-match: `(match __lt ((tuple 1 b) <body>) (_ (trap …)))` — the DIRECT tuple matcher binds
        // the component sub-patterns for the original body; the `_` arm is dead (the guard proved the value
        // match) but keeps the inner match exhaustive.
        let body_scrut = db.push_name(&name);
        let body_true_arm = db.push_list(vec![tuple_pat, body]);
        let trap_head = db.push_name("trap");
        let trap_msg = db.push_str("unreachable: list-tuple-element value already gated by guard");
        let trap = db.push_list(vec![trap_head, trap_msg]);
        let wild_b = db.push_name("_");
        let body_false_arm = db.push_list(vec![wild_b, trap]);
        let body_match_head = db.push_name("match");
        let new_body = db.push_list(vec![
            body_match_head,
            body_scrut,
            body_true_arm,
            body_false_arm,
        ]);
        new_arms.push(db.push_list(vec![new_pat, new_body]));
    }
    let match_head = db.push_name("match");
    let mut items = vec![match_head, scrutinee];
    items.extend(new_arms);
    let rewritten = db.push_list(items);
    crate::resolve::resolve_subtree(db, rewritten);
    trace!(target: "rcdzc::lower", scrutinee = scrutinee.0, "list match with a refutable tuple element → fresh-binder + value-test guard + body re-match");
    Some(core_of(db, rewritten))
}

/// The (key, value) sub-pattern of a record-pattern field, across the three field forms: the native
/// `FieldPair` (head `Leaf::FieldPair`), the name-head `(= k v)` alias, and the legacy positional `(k v)`.
/// `None` for a `.. rest` marker or a malformed field. The record twin of the key/value extraction in
/// `map_pattern_with_wildcard_values`.
fn record_field_kv(db: &Db, pair: StructId) -> Option<(StructId, StructId)> {
    db.ast
        .field_pair_parts(pair)
        .or_else(|| db.ast.field_pair(pair))
        .or_else(|| match db.ast.get(pair) {
            crate::ast::Struct::List(kv) if kv.len() == 2 => Some((kv[0], kv[1])),
            _ => None,
        })
}

/// Whether `elem_pat` is a REFUTABLE RECORD list-element pattern — a `(record (= k v) …)` with ≥1 field
/// whose VALUE sub-pattern `check_binding_pattern` flags NON-EXHAUSTIVE (a literal atom or a multi-variant
/// constructor). A record of only bare-binder / `_` field values is IRREFUTABLE (it matches any record
/// having those fields, bound inline by `check_binding_pattern`), so it returns `false` and is left on the
/// pre-existing irrefutable path (that is exactly the dst2/dm1/dt2 shape); only a literal-bearing record
/// like `(record (= v 1))` needs the value-refinement desugar — the true last refutable-list-element kind
/// after ctor/newtype/nested-list/map/tuple (#8380). Refutability is the AUTHORITATIVE recursive
/// `check_binding_pattern` signal (the same predicate `is_refutable_tuple_element` / the map & list
/// irrefutable checks use), NOT a coarse structural test: a NESTED irrefutable compound field value declines
/// upstream (not-yet-wired) with a non-`NonExhaustive` code, so it is correctly NOT routed here. A non-record
/// element returns `false`.
pub(super) fn is_refutable_record_element(db: &mut Db, elem_pat: StructId) -> bool {
    let Some(fields) = db
        .ast
        .compound_form_of(elem_pat, CompoundCtor::Record)
        .map(<[_]>::to_vec)
    else {
        return false;
    };
    fields.iter().any(|&pair| {
        // A `.. rest` marker is not a field-value refutation (it binds the unnamed fields, irrefutable).
        if db.ast.as_name(pair) == Some("..") || db.ast.as_form(pair, "..").is_some() {
            return false;
        }
        match record_field_kv(db, pair) {
            Some((_, value)) => matches!(
                check_binding_pattern(db, value, &crate::ty::Ty::Any),
                Err(ref r) if r.code == Some(Code::NonExhaustive)
            ),
            None => false,
        }
    })
}

/// A copy of the record pattern `record_pat` for the guard's value test, with every BARE-BINDER / `_` field
/// VALUE replaced by a wildcard `_` but every REFUTABLE field value (a literal, a nested ctor) CLONED and
/// KEPT, and any trailing `.. rest` dropped. The record twin of `ctor_pattern_with_wildcard_payloads` /
/// `map_pattern_with_wildcard_values`: the guard `(match __lr ((record (= v 1)) true) (_ false))` must test
/// the refutable field values (so a non-matching literal FALLS THROUGH), bind nothing (the body re-match
/// binds), and — since a record pattern names only the fields it wants — the presence of the named fields is
/// implied. Keys / the `record` head / the `=` markers are reused live (structural nodes); the refutable
/// values are cloned (the ORIGINAL record pattern is reused by the body re-match — a node has one parent).
pub(super) fn record_pattern_with_wildcard_values(db: &mut Db, record_pat: StructId) -> StructId {
    let fields = db
        .ast
        .compound_form_of(record_pat, CompoundCtor::Record)
        .map(<[_]>::to_vec)
        .unwrap_or_default();
    let head = match db.ast.get(record_pat) {
        crate::ast::Struct::List(items) if !items.is_empty() => items[0],
        _ => db.push_name("record"),
    };
    let mut children = vec![head];
    for &pair in &fields {
        // Stop at a `..` marker — the value test needs only the NAMED fields.
        if db.ast.as_name(pair) == Some("..") || db.ast.as_form(pair, "..").is_some() {
            break;
        }
        let Some((key, value)) = record_field_kv(db, pair) else {
            children.push(pair);
            continue;
        };
        // A BARE BINDER / `_` value → wildcard (the guard should not bind it, and it matches any value). A
        // REFUTABLE value (a literal / nested ctor — anything NOT a bare name) is CLONED and kept so the guard
        // tests it and a mismatch FALLS THROUGH instead of trapping in the body re-match.
        let new_value = if db.ast.as_name(value).is_some() {
            db.push_name("_")
        } else {
            clone_refutable_payload(db, value)
        };
        // Rebuild the field pair as the canonical name-head `(= key value)` (the direct record matcher reads
        // both this and the native FieldPair form; the name-head form is the simplest to synthesize).
        let eq = db.push_name("=");
        children.push(db.push_list(vec![eq, key, new_value]));
    }
    db.push_list(children)
}

/// PRE-PASS for `lower_match_list` (the RECORD twin of `desugar_refutable_tuple_list_elements`): rewrite an
/// arm whose list pattern has a refutable RECORD leading element `(list (record (= v 1)) rest… .. r)` into an
/// equivalent guarded arm the length-dispatch matcher + the (direct) record matcher already handle. A record
/// is a fixed-shape product (a static field set, no length dispatch), so — exactly like a tuple — the ONLY
/// refutation is its literal/ctor field values; it needs a value-test guard for fall-through AND value
/// binding for the body:
///   `((list (record (= v 1)) rest… .. r) body)`  ≡
///   `((guard (list __lr rest… .. r) (match __lr ((record (= v 1)) true) (_ false)))
///       (match __lr ((record (= v a)) body) (_ (trap …))))`
/// The GUARD's value test (a wildcard-value record pattern via `record_pattern_with_wildcard_values` — keeps
/// the literal `1`, wildcards the binder fields) gates the arm so a non-matching literal FALLS THROUGH; the
/// BODY re-matches the SAME element binder to bind the field binders for the original body. One
/// refutable-record element per arm (the common shape); ≥2 declines (mirrors the tuple ">1" limit). NO new
/// IR. Returns `Some(Core)` iff the rewrite fired. (Completes the refutable-list-element-refinement arc — the
/// TRUE last element kind after ctor/newtype/nested-list/map/tuple; #8380.)
pub(super) fn desugar_refutable_record_list_elements(
    db: &mut Db,
    scrutinee: StructId,
    arms: &[(StructId, StructId)],
) -> Option<Core> {
    let leading_of = |db: &mut Db, pat: StructId| -> Option<Vec<StructId>> {
        let inner = match db.ast.as_form(pat, "guard") {
            Some(g) if g.len() == 2 => g[0],
            _ => pat,
        };
        let es = db
            .ast
            .compound_form_of(inner, CompoundCtor::List)
            .map(<[_]>::to_vec)?;
        let lead = db
            .ast
            .rest_marker(&es)
            .map(|(i, _, _)| i)
            .unwrap_or(es.len());
        Some(es[..lead].to_vec())
    };
    let mut any_record = false;
    for &(pat, _) in arms {
        if let Some(leading) = leading_of(db, pat)
            && leading.iter().any(|&e| is_refutable_record_element(db, e))
        {
            any_record = true;
        }
    }
    if !any_record {
        return None;
    }
    let mut new_arms: Vec<StructId> = Vec::with_capacity(arms.len());
    for (ai, &(pat, body)) in arms.iter().enumerate() {
        let (inner, existing_guard) = match db.ast.as_form(pat, "guard") {
            Some(g) if g.len() == 2 => (g[0], Some(g[1])),
            _ => (pat, None),
        };
        let list_es = db
            .ast
            .compound_form_of(inner, CompoundCtor::List)
            .map(<[_]>::to_vec);
        let Some(es) = list_es else {
            new_arms.push(db.push_list(vec![pat, body]));
            continue;
        };
        let lead = db
            .ast
            .rest_marker(&es)
            .map(|(i, _, _)| i)
            .unwrap_or(es.len());
        let record_positions: Vec<usize> = (0..lead)
            .filter(|&p| is_refutable_record_element(db, es[p]))
            .collect();
        if record_positions.is_empty() {
            new_arms.push(db.push_list(vec![pat, body]));
            continue;
        }
        if record_positions.len() > 1 {
            return Some(Core::Poison(Reject::decline(
                "a list arm with more than one refutable record element is not supported (match one \
                 refutable record element per arm)",
            )));
        }
        let rpos = record_positions[0];
        let record_pat = es[rpos]; // the original `(record (= v 1))` element pattern
        let list_head = match db.ast.get(inner) {
            crate::ast::Struct::List(items) if !items.is_empty() => items[0],
            _ => db.push_name("list"),
        };
        let name = format!("__lr{ai}");
        // Rebuild the list pattern with the record element replaced by a fresh bare binder.
        let mut new_es: Vec<StructId> = Vec::with_capacity(es.len());
        for (p, &e) in es.iter().enumerate() {
            if p == rpos {
                new_es.push(db.push_name(&name));
            } else {
                new_es.push(e);
            }
        }
        let mut list_children = vec![list_head];
        list_children.extend(new_es);
        let new_list = db.push_list(list_children);
        // The VALUE-test guard: `(match __lr ((record (= v 1)) true) (_ false))`.
        let test_scrut = db.push_name(&name);
        let test_pat = record_pattern_with_wildcard_values(db, record_pat);
        let true_node = db.push_atom(crate::ast::Leaf::Bool(true));
        let false_node = db.push_atom(crate::ast::Leaf::Bool(false));
        let test_true_arm = db.push_list(vec![test_pat, true_node]);
        let wild = db.push_name("_");
        let test_false_arm = db.push_list(vec![wild, false_node]);
        let test_match_head = db.push_name("match");
        let value_test = db.push_list(vec![
            test_match_head,
            test_scrut,
            test_true_arm,
            test_false_arm,
        ]);
        let guard_cond = match existing_guard {
            None => value_test,
            Some(g) => {
                // The user guard cond `g` may read the record's FIELD binders (`(guard (list (record (= v a)))
                // (> a 3))` reads `a`). Those bind only in the BODY re-match, NOT at the outer guard level — so
                // ANDing `g` with the value_test would leave `a` unbound (a false CDZ0101). Mirror the tuple /
                // nested-list desugar: evaluate `g` INSIDE a match on `__lr` that binds the FULL record pattern
                // — `(match __lr ((record (= v a)) g) (_ false))` — so `a` is in scope for `g`, and the literal
                // field is tested by the same match (subsuming value_test: a non-matching field → `_ → false` →
                // fall through). A FULL clone of the record pattern (the body re-match reuses the original).
                let g_scrut = db.push_name(&name);
                let g_clone = clone_refutable_payload(db, record_pat);
                let g_true_arm = db.push_list(vec![g_clone, g]);
                let g_wild = db.push_name("_");
                let g_false = db.push_atom(crate::ast::Leaf::Bool(false));
                let g_false_arm = db.push_list(vec![g_wild, g_false]);
                let g_match_head = db.push_name("match");
                db.push_list(vec![g_match_head, g_scrut, g_true_arm, g_false_arm])
            }
        };
        let guard_head = db.push_name("guard");
        let new_pat = db.push_list(vec![guard_head, new_list, guard_cond]);
        // The BODY re-match: `(match __lr ((record (= v a)) <body>) (_ (trap …)))` — the DIRECT record matcher
        // binds the field sub-patterns for the original body; the `_` arm is dead (the guard proved the value
        // match) but keeps the inner match exhaustive.
        let body_scrut = db.push_name(&name);
        let body_true_arm = db.push_list(vec![record_pat, body]);
        let trap_head = db.push_name("trap");
        let trap_msg = db.push_str("unreachable: list-record-element value already gated by guard");
        let trap = db.push_list(vec![trap_head, trap_msg]);
        let wild_b = db.push_name("_");
        let body_false_arm = db.push_list(vec![wild_b, trap]);
        let body_match_head = db.push_name("match");
        let new_body = db.push_list(vec![
            body_match_head,
            body_scrut,
            body_true_arm,
            body_false_arm,
        ]);
        new_arms.push(db.push_list(vec![new_pat, new_body]));
    }
    let match_head = db.push_name("match");
    let mut items = vec![match_head, scrutinee];
    items.extend(new_arms);
    let rewritten = db.push_list(items);
    crate::resolve::resolve_subtree(db, rewritten);
    trace!(target: "rcdzc::lower", scrutinee = scrutinee.0, "list match with a refutable record element → fresh-binder + value-test guard + body re-match");
    Some(core_of(db, rewritten))
}

pub(super) fn lower_match_list(
    db: &mut Db,
    scrutinee: StructId,
    arms: &[(StructId, StructId)],
) -> Core {
    // WELL-FORMEDNESS (before any desugar): each LITERAL leading element must type-agree with the list's
    // element type. `(list "x" .. r)` on an `(List Int64)` writes a String where an Int64 is required —
    // without this it desugars to a `(= __le "x")` guard that silently never matches (String-vs-Int folds
    // to `false`), accepting a mistyped element as dead code. Reject it CDZ0201 naming both types, anchored
    // at the literal — the list-element twin of the scalar-match + map-key pattern-type checks. Runs before
    // the literal desugar (which would otherwise hide the mismatch inside a synthesized guard). `Any`
    // element type (unsolved) is not checked (not-yet-constrained).
    if let crate::ty::Ty::List(elem) = crate::infer::type_of(db, scrutinee)
        && !matches!(*elem, crate::ty::Ty::Any)
    {
        for &(pat, _) in arms {
            let inner = match db.ast.as_form(pat, "guard") {
                Some(g) if g.len() == 2 => g[0],
                _ => pat,
            };
            let Some(es) = db
                .ast
                .compound_form_of(inner, CompoundCtor::List)
                .map(<[_]>::to_vec)
            else {
                continue;
            };
            let lead = db
                .ast
                .rest_marker(&es)
                .map(|(i, _, _)| i)
                .unwrap_or(es.len());
            for &e in &es[..lead] {
                if is_refutable_literal_element(db, e) {
                    let et = crate::infer::type_of(db, e);
                    if !matches!(et, crate::ty::Ty::Any) && !et.agrees_with(&elem) {
                        return Core::Poison(
                            Reject::coded(
                                Code::Malformed,
                                format!(
                                    "this list-element pattern is {}, but the list's elements are {}",
                                    et.render_name(&db.name_ctx()),
                                    elem.render_name(&db.name_ctx())
                                ),
                            )
                            .at(e),
                        );
                    }
                }
            }
        }
    }
    // PRE-PASS (nested list): a refutable nested LIST leading element (`(list (list a .. r1) .. r2)`)
    // dispatches on the INNER list's length — desugar to a fresh binder + an inner-length guard + a body
    // re-match binding the inner sub-patterns. Run FIRST (a nested-list element is neither a ctor nor a
    // literal, so the other passes skip it); it rebuilds the match and recurses through `core_of`.
    if let Some(core) = desugar_refutable_nested_list_elements(db, scrutinee, arms) {
        return core;
    }
    // PRE-PASS (map): a refutable MAP leading element (`(list (map (k v)) .. r)`) — a list of key-value
    // records destructured by key — desugars to a fresh binder + a key-presence guard + a body re-match
    // binding the values (the direct map matcher). Run alongside the ctor pass (a map element is neither a
    // ctor nor a literal, so the other passes skip it); it rebuilds the match and recurses through `core_of`.
    if let Some(core) = desugar_refutable_map_list_elements(db, scrutinee, arms) {
        return core;
    }
    // PRE-PASS (tuple): a refutable TUPLE leading element (`(list (tuple 1 b) .. r)`) — a literal/ctor in a
    // tuple component — desugars to a fresh binder + a value-test guard + a body re-match binding the
    // components (the direct tuple matcher). A tuple is fixed-arity, so it needs only the value test (no
    // length dispatch). The last refutable-element kind after ctor/map/nested-list (#8364); a tuple element
    // is none of those, so the other passes skip it. Rebuilds the match and recurses through `core_of`.
    if let Some(core) = desugar_refutable_tuple_list_elements(db, scrutinee, arms) {
        return core;
    }
    // PRE-PASS (record): a refutable RECORD leading element (`(list (record (= v 1)) .. r)`) — a literal/ctor
    // in a record field value — desugars to a fresh binder + a value-test guard + a body re-match binding the
    // field sub-patterns (the direct record matcher). A record is a fixed-shape product, so — like a tuple —
    // it needs only the value test (no length dispatch). The TRUE last refutable-element kind after
    // ctor/map/nested-list/tuple (#8380); a record element is none of those, so the other passes skip it.
    // Rebuilds the match and recurses through `core_of`.
    if let Some(core) = desugar_refutable_record_list_elements(db, scrutinee, arms) {
        return core;
    }
    // PRE-PASS (ctor saturation): `(list) + (list (Some x) .. r) + (list (None) .. r)` covers every length —
    // the ctor-lead arms saturate the element sum's variant set, so the match is total without a `_`. Run
    // BEFORE the ctor desugar so it sees the un-desugared ctor arms: it turns the LAST saturating arm into an
    // unconditional `(list __ls .. r)` cover (payload-binding moved to the body), while the earlier ctor arms
    // still desugar to their discriminant guards on the recursion. The bool analogue of Inc-23.
    if let Some(core) = desugar_saturating_ctor_list_elements(db, scrutinee, arms) {
        return core;
    }
    // PRE-PASS (ctor): a refutable MULTI-VARIANT CONSTRUCTOR leading element (`(list (Op.Add x) .. r)`) —
    // the tree-walk-tagged-nodes idiom — desugars to a fresh binder + a discriminant-test guard + a body
    // re-match binding the payload. Run BEFORE the literal pass: it rebuilds the match and recurses through
    // `core_of` → `lower_match_list`, where the literal pass then handles any remaining literal elements.
    if let Some(core) = desugar_refutable_ctor_list_elements(db, scrutinee, arms) {
        return core;
    }
    // PRE-PASS (bool saturation): `(list) + (list true .. r) + (list false .. r)` covers every length — the
    // two bool-lead arms saturate the first element, so the match is total without a `_`. Run BEFORE the
    // literal pass so the LAST saturating arm becomes an unconditional `(list _ .. r)` cover (an unguarded
    // tail the length matcher counts) while the earlier bool arm still desugars to its value-test guard.
    if let Some(core) = desugar_saturating_bool_list_elements(db, scrutinee, arms) {
        return core;
    }
    // PRE-PASS: a refutable SCALAR/STRING LITERAL leading element (`(list 0 a .. r)`, `(list "add" x)`) is
    // an element-VALUE refinement the length-dispatch matcher cannot express directly. Desugar it to a
    // fresh binder + a `(= binder <lit>)` guard (the Inc-5 guard machinery handles the rest), then this
    // function re-runs with no literal elements. Fires only when some arm actually has a literal element.
    if let Some(core) = desugar_refutable_literal_list_elements(db, scrutinee, arms) {
        return core;
    }
    // Each arm's length condition + an optional GUARD (a boolean the arm's binders are in scope for). A
    // guarded arm fires only when its length holds AND the guard is true; on a false guard it FALLS THROUGH
    // to the next arm, and it does NOT count toward length-coverage exhaustiveness (its guard may fail) —
    // exactly as a guarded scalar/sum arm behaves.
    enum Arm {
        Fixed(usize, Option<StructId>, StructId), // a fixed-arity `(list …)` of this exact arity
        Rest(usize, Option<StructId>, StructId), // a rest `(list p… .. rest)` — matches length ≥ k (lead=k)
        Wild(Option<StructId>, StructId),        // a bare binder / `_` — matches any length
    }
    // The list's element type (from the scrutinee), used to classify each element sub-pattern's shape
    // (`Any` when unsolved — permissive, the same treatment a binding position gets).
    let elem_ty = match crate::infer::type_of(db, scrutinee) {
        crate::ty::Ty::List(e) => (*e).clone(),
        _ => crate::ty::Ty::Any,
    };
    let mut classified: Vec<Arm> = Vec::with_capacity(arms.len());
    for &(pat, body) in arms {
        // Peel a `(guard <inner-pattern> <cond>)` wrapper: the arm's list/binder pattern is `<inner>`, and
        // `<cond>` is the guard (a boolean the pattern's binders are in scope for, resolve Case 6lg / 5g).
        let (pat, guard) = match db.ast.as_form(pat, "guard") {
            Some(g) if g.len() == 2 => (g[0], Some(g[1])),
            Some(g) => {
                // A wrong-arity `(guard …)` — a surplus element gets the shared delete fix; too few is
                // message-only. Anchored at the offending arm PATTERN, not the enclosing match.
                return Core::Poison(crate::resolve::fixed_arity_reject(
                    pat,
                    g,
                    2,
                    "a guarded pattern must be (guard <pattern> <cond>)",
                ));
            }
            None => (pat, None),
        };
        if db.ast.as_name(pat).is_some() {
            classified.push(Arm::Wild(guard, body));
            continue;
        }
        match db.ast.compound_form_of(pat, CompoundCtor::List) {
            Some(es) => {
                let es = es.to_vec();
                // LINEARITY across the WHOLE list pattern (`core-semantics.md §145`: "the whole pattern
                // MUST remain linear (CDZ0102)") — the same check the tuple/sum matchers run, previously
                // MISSING for list patterns (so `(list a a)` / `(list a b .. a)` silently bound `a` twice).
                // `check_pattern_linear` skips `..`/`_`/ctor heads and faults a repeated binder.
                if let Err(r) = check_pattern_linear(db, pat) {
                    return Core::Poison(r);
                }
                // Split at a `..` marker: `lead` leading element sub-patterns, then (for a rest pattern) the
                // rest binder. A rest pattern needs EXACTLY one binder after `..`.
                match db.ast.rest_marker(&es) {
                    Some((i, operand, trailing_start)) => {
                        if trailing_start != es.len() {
                            // Anchor at the offending list PATTERN, not the enclosing match, so the
                            // squiggle lands on `(list … .. …)` rather than the whole `(match …)`.
                            return Core::Poison(
                                Reject::coded(
                                    Code::Malformed,
                                    "a list rest pattern is `(list p… .. rest)` — exactly one binder after `..`",
                                )
                                .at(pat),
                            );
                        }
                        // The rest binder itself must be a bare name / `_` (it binds the tail SUBLIST — a
                        // nested pattern or literal is not allowed there). A CODED Malformed (not a decline)
                        // so it surfaces UNCONDITIONALLY as a well-formedness fault, even when the arm body
                        // does not reference the nested pattern's inner binders (a decline is
                        // reachability-gated, so `cdz check` on such an arm reported NOTHING — v-diagnostics
                        // via v-guide). Byte-identical to the resolve `Case 6mr` message + anchored at the
                        // same `pat`, so when the body DOES reference an inner binder both fire at this node
                        // and same-node dedup collapses them into ONE diagnostic.
                        if db.ast.as_name(operand).is_none() {
                            return Core::Poison(
                                Reject::coded(
                                    Code::Malformed,
                                    crate::diag::LIST_REST_BINDER_NAME_ONLY,
                                )
                                .at(pat),
                            );
                        }
                        // Each LEADING element sub-pattern must be IRREFUTABLE (composes to any depth); a
                        // refutable/unsupported one declines.
                        for &e in &es[..i] {
                            if let Err(r) = list_element_irrefutable_or_decline(db, e, &elem_ty) {
                                return Core::Poison(r);
                            }
                        }
                        classified.push(Arm::Rest(i, guard, body));
                    }
                    None => {
                        // Fixed arity: each element sub-pattern must be IRREFUTABLE (composes to any depth).
                        for &e in &es {
                            if let Err(r) = list_element_irrefutable_or_decline(db, e, &elem_ty) {
                                return Core::Poison(r);
                            }
                        }
                        classified.push(Arm::Fixed(es.len(), guard, body));
                    }
                }
            }
            None => {
                // A SCALAR / STRING LITERAL arm pattern (`8`, `true`, `1.5`, `"add"`) over a LIST
                // scrutinee. A literal pattern matches the WHOLE scrutinee value, so its type must be the
                // scrutinee's — but a scalar/string can never equal a `List`, so this is a permanent TYPE
                // MISMATCH (the pattern face of the `(match (list 1 2) (true 99))` ill-typedness), NOT a
                // not-yet-lowerable shape. Reject it coded CDZ0203 up front — it previously fell through to
                // the codeless "a list match arm … is not supported" decline below, which read as
                // unimplemented rather than ill-typed (v-deferral decline-correctness, v-cdz-smith sweep).
                // (Names were classified as binders above, so a non-name `Atom` here is a literal.)
                if matches!(db.ast.get(pat), crate::ast::Struct::Atom(_)) {
                    return Core::Poison(
                        Reject::coded(
                            Code::TypeMismatch,
                            "a scalar-literal pattern cannot match a `List` scrutinee — a literal pattern \
                             matches the whole value, but the value here is a list, not a scalar (an \
                             element pattern is `(list …)`; a whole-list binder is a bare name)",
                        )
                        .at(pat),
                    );
                }
                // The arm is neither a `(list …)` element pattern nor a bare binder. The COMMON way to
                // reach here is a constructor-shaped head over a LIST scrutinee — `(Zorp x)`, `(List.Cons
                // h t)` — i.e. a name used as a pattern ctor that a list scrutinee has no ctors for. When
                // that head is a name resolving to a CODED poison (an UNBOUND name → CDZ0101, a `List.Cons`
                // non-member → CDZ0201), propagate that coded fault instead of the generic UNCODED decline
                // — the LIST twin of the sum matcher's `variant_disc_of`-miss path (7582), which propagates
                // `core_of(head)`'s coded member-fold poison rather than an uncoded "not a variant ctor".
                // A coded poison is surfaced for EVERY body by `type_errors`' `match_pattern_fault` accessor
                // (which returns only CODED faults), closing the "check missed a lowering-only decline on a
                // parameterized / recursive body" hole (`cdz check` was silent while `cdz compile` declined
                // — the same gap M81 closed for variant-typo pattern heads). A head that is NOT an
                // unbound/non-member name (a literal, a bound name resolving cleanly, a tuple/map pattern)
                // yields no coded poison, so this keeps the honest uncoded decline for a genuinely
                // not-yet-lowerable list-arm shape — no false alarm.
                // `pat` here is the arm pattern already PEELED of any `(guard …)` wrapper (above), so its
                // first child is the constructor head directly.
                let ctor_head = match db.ast.get(pat) {
                    crate::ast::Struct::List(children) => children.first().copied(),
                    _ => None,
                };
                if let Some(head) = ctor_head
                    && let Core::Poison(reject) = core_of(db, head)
                    && reject.code.is_some()
                {
                    return Core::Poison(reject);
                }
                return Core::Poison(Reject::decline(
                    "a list match arm that is not an element pattern or a binder is not supported",
                ));
            }
        }
    }
    // WELL-FORMEDNESS: a list is OPEN (any length), so the arms must JOINTLY cover every length n ≥ 0.
    // A `Wild` / `Rest(k)` covers the infinite tail [k, ∞) (a `Wild` = `Rest(0)`); a `Fixed(k)` covers the
    // single length {k}. Let `m` be the SMALLEST tail-start among all catch-all arms (0 if any `Wild`).
    // If there is no `Wild`/`Rest` arm at all, no arm covers the infinite tail → non-exhaustive. Otherwise
    // lengths [m, ∞) are covered by that arm; the finite prefix 0..m must be covered by `Fixed` arms (no
    // `Rest(j)` with j < m exists, since m is the minimum). Else CDZ0210.
    //
    // A GUARDED arm covers NOTHING unconditionally (its guard may fail), so it does not contribute to
    // coverage — only UNGUARDED arms count (`guard.is_none()`), exactly as a guarded scalar/sum arm is
    // excluded from exhaustiveness. So a guarded rest/wild does not close the tail, and a guarded fixed arm
    // does not cover its length.
    let tail_start = classified.iter().filter_map(|a| match a {
        Arm::Wild(None, _) => Some(0),
        Arm::Rest(k, None, _) => Some(*k),
        _ => None,
    });
    let Some(m) = tail_start.min() else {
        // NO catch-all arm → no arm covers the infinite tail. The mechanical repair is a WILDCARD `_` arm
        // (covers every remaining length), the list analogue of the scalar add-wildcard fix — bodied with
        // a diverging `(trap "TODO")` so it type-checks whatever the other arms return. Anchored at the
        // `(match …)` form (parent of the scrutinee); no parent → the bare reject.
        let reject = Reject::coded(
            Code::NonExhaustive,
            "a list match must cover every length (end in a `_`, a whole-list binder, or a `(list .. rest)` arm)",
        );
        return Core::Poison(match db.parent_of(scrutinee) {
            Some(match_form) => reject.with_fix(Fix::insert_arms_heuristic(
                match_form,
                vec!["(_ (trap \"TODO\"))".to_string()],
            )),
            None => reject,
        });
    };
    // Every length in 0..m must have a matching UNGUARDED `Fixed` arm (a guarded one may not fire).
    let missing: Vec<usize> = (0..m)
        .filter(|&n| {
            !classified
                .iter()
                .any(|a| matches!(a, Arm::Fixed(k, None, _) if *k == n))
        })
        .collect();
    if !missing.is_empty() {
        // A `Rest(m)`/`Wild` covers `[m, ∞)`, but a shorter length `n < m` has no `Fixed(n)` arm. The
        // repair is to add exactly those missing-length arms — `(list _ _ … n underscores) (trap "TODO")`
        // (a length-0 gap is the empty `((list) (trap "TODO"))`). Underscore elements (the matcher only
        // needs the ARITY covered; the author renames as needed), diverging body. Fixed arms must precede
        // the catch-all to be reachable, but `insert_arms` appends AFTER the last arm — which is where the
        // `Rest`/`Wild` sits, so the inserted fixed arm would be dead. Still, applying it makes the match
        // exhaustive (the appended arm's length is now covered by SOME arm — the appended one shadows
        // nothing since the earlier catch-all already matched); `--verify-fixes` confirms it clears the
        // CDZ0210. (WHERE to place it for reachability is the author's call — the fix resolves the gap.)
        let arms: Vec<String> = missing
            .iter()
            .map(|&n| {
                let unders = vec!["_"; n].join(" ");
                let pat = if n == 0 {
                    "(list)".to_string()
                } else {
                    format!("(list {unders})")
                };
                format!("({pat} (trap \"TODO\"))")
            })
            .collect();
        let reject = Reject::coded(
            Code::NonExhaustive,
            "a list match must cover every length (a rest pattern leaves shorter lengths uncovered)",
        );
        return Core::Poison(match db.parent_of(scrutinee) {
            Some(match_form) => reject.with_fix(Fix::insert_arms_heuristic(match_form, arms)),
            None => reject,
        });
    }
    // The runtime `Core::MatchList` arms built from the classified arms (each an `(cond, guard, body)`).
    // Used both when the scrutinee is a runtime list AND when a constant fold aborts because a guard reads
    // a runtime value (so it cannot be decided at compile time).
    let build_match_list = |classified: &[Arm]| -> Core {
        Core::MatchList {
            scrutinee,
            arms: classified
                .iter()
                .map(|arm| {
                    let (cond, guard, body) = match arm {
                        Arm::Fixed(k, g, body) => (crate::core::ListArmCond::LenEq(*k), *g, *body),
                        Arm::Rest(lead, g, body) => {
                            (crate::core::ListArmCond::LenGe(*lead), *g, *body)
                        }
                        Arm::Wild(g, body) => (crate::core::ListArmCond::Any, *g, *body),
                    };
                    crate::core::ListArm { cond, guard, body }
                })
                .collect(),
        }
    };
    // A CONSTANT scrutinee FOLDS: the length selects the arm; the body's element binders read the
    // constant elements via their `SumPayload` `Elem`/`RestFrom` folds, so lowering the SELECTED body is
    // all that is needed (no β-substitution). A GUARDED arm folds only if its guard folds to a constant
    // bool: `true` selects it, `false` falls through to the next arm; a guard that does NOT fold to a const
    // (reads a runtime value) ABORTS the fold to the runtime probe chain (like the scalar guard fold).
    match core_of(db, scrutinee) {
        Core::ListNew { elems } => {
            let n = elems.len();
            for arm in &classified {
                let (len_matches, guard, body) = match arm {
                    Arm::Fixed(k, g, body) => (*k == n, *g, *body),
                    Arm::Rest(lead, g, body) => (n >= *lead, *g, *body), // rest: length ≥ leading count
                    Arm::Wild(g, body) => (true, *g, *body),
                };
                if !len_matches {
                    continue; // this arm's length doesn't match the constant — try the next
                }
                match guard {
                    None => return core_of(db, body),
                    Some(g) => match core_of(db, g) {
                        Core::ConstBool(true) => return core_of(db, body),
                        Core::ConstBool(false) => continue, // guard fails → fall through to the next arm
                        // The guard did not fold to a const bool (reads a runtime value) → we cannot decide
                        // this arm at compile time; emit the runtime dispatch chain instead.
                        _ => return build_match_list(&classified),
                    },
                }
            }
            Core::Poison(Reject::decline(
                "list match: no arm matched the constant list (unreachable — a catch-all was required)",
            ))
        }
        Core::Poison(r) => Core::Poison(r),
        // A RUNTIME list scrutinee — emit `Core::MatchList`, which dispatches on `vec-len` at run time.
        // Each arm's length condition drives the dispatch; the leading element binders + rest binder read
        // the runtime list on their own (`SumPayload` `Elem`/`RestFrom` → `vec-get`/`vec-split`).
        _ => build_match_list(&classified),
    }
}

/// Classify a single LEADING list-element sub-pattern for a `match` arm: accept it (Ok) iff it is
/// IRREFUTABLE — a wildcard/name (binds anything), a tuple pattern of irrefutable elements, or a
/// single-variant constructor of irrefutable payloads, composed to any depth (`core-semantics.md §145`).
/// A dispatch-by-LENGTH list match binds an accepted element's sub-binders down an `Elem`/`Payload` path
/// (resolve Case 6l) and always matches once the length condition holds, so an irrefutable element needs
/// no per-element runtime test.
///
/// This reuses [`check_binding_pattern`] — a binding position is exactly "must be irrefutable", the same
/// property a leading element needs — with ONE reinterpretation for the match context: a REFUTABLE element
/// (a literal, a multi-variant constructor) is `check_binding_pattern`'s CDZ0210, but in a `match` arm a
/// refutable element is not an error — it is a not-yet-supported element-VALUE refinement (`Core::MatchList`
/// dispatches only on length), so map that CDZ0210 to a DECLINE (a later increment refines on element
/// values). A SHAPE error (CDZ0201 — e.g. a tuple pattern against a scalar element) and a NON-LINEAR binder
/// (CDZ0102) stay HARD rejects (the whole-arm linearity check in `lower_match_list` also covers CDZ0102).
/// Accept a map-pattern VALUE sub-pattern `v` iff it is IRREFUTABLE — a bare binder / `_`, or an
/// irrefutable nested pattern (a `(tuple …)` of irrefutable elements, a single-variant `(Mk n)`), composed
/// to any depth. Its binders read via `MapField` `value_steps` (resolve descends them). A REFUTABLE value
/// (a literal, or a multi-variant ctor `(Some n)`) does NOT reach this guard: `map_value_liftable` lifts it
/// FIRST into a dispatching `(match __mv (<value> body) (_ <catch-all>))` (`5bc7215e` literals, `d4a471d3`
/// multi-variant ctors), so by here every value is bare or irrefutable. This residual guard reuses
/// `check_binding_pattern` (a binding position IS "must be irrefutable") against `Any` — refutability is a
/// property of the pattern SHAPE, not the value type — mapping a stray CDZ0210 (a refutable value the lift
/// somehow missed) to a codeless DECLINE, exactly as `list_element_irrefutable_or_decline` does for a list
/// element.
pub(super) fn map_value_irrefutable_or_decline(db: &mut Db, v: StructId) -> Result<(), Reject> {
    match check_binding_pattern(db, v, &crate::ty::Ty::Any) {
        Ok(()) => Ok(()),
        Err(r) if r.code == Some(Code::NonExhaustive) => Err(Reject::unsupported(
            "a refutable map value sub-pattern (a literal or multi-variant constructor) needs \
             value-discriminant refinement, which the key-directed map matcher does not support",
        )),
        Err(r) => Err(r),
    }
}

pub(super) fn list_element_irrefutable_or_decline(
    db: &mut Db,
    elem_pat: StructId,
    elem_ty: &crate::ty::Ty,
) -> Result<(), Reject> {
    // A NESTED LIST element `(list …)` is refutable UNLESS it is the zero-leading rest form `(list .. r)`
    // (which matches every inner list — truly irrefutable, `RestFrom(0)` reads the whole inner list safely
    // even when empty). A LEADING-element nested list `(list a .. r1)` / `(list a b)` is length-refutable
    // (`(list a .. r1)` misses the EMPTY inner list — `core-semantics.md §A List Is Deconstructed`: "a
    // single-leading-element-plus-rest pattern MUST match any NON-empty list"), and the length-dispatch
    // matcher tests only the OUTER length — so binding `a` = `Elem(i), Elem(0)` on an empty inner list
    // would TRAP instead of falling through to the next arm. Matching it soundly needs an INNER-length
    // guard (the list-element analogue of the Inc-11/12 refutable-element desugars) — a later increment; for
    // now DECLINE the leading-element nested-list case honestly rather than emit a latent trap-on-empty
    // miscompile. (The BINDER resolution for both forms is in place — `find_leading_binder_in_list_pattern`
    // descends a nested `(list …)` element via `find_binder_in_list` — so once the guard lands the body
    // reads its nested binders with no further resolve work.)
    if is_list_pattern(db, elem_pat) {
        let has_leading = db
            .ast
            .compound_form_of(elem_pat, CompoundCtor::List)
            .map(|es| {
                let dd = db.ast.rest_marker(es).map(|(i, _, _)| i);
                match dd {
                    // A rest form `(list p… .. rest)` with ≥1 leading element is refutable.
                    Some(k) => k > 0,
                    // No `..` → a fixed-arity nested list `(list a b)` is refutable (a specific length).
                    None => !es.is_empty(),
                }
            })
            .unwrap_or(false);
        if has_leading {
            return Err(Reject::unsupported(
                "a nested list element with leading positions (e.g. `(list a .. rest)`) is refutable — it \
                 needs an inner-length guard the length-dispatch matcher does not emit; only the \
                 zero-leading rest form `(list .. rest)` is irrefutable here",
            ));
        }
        // The zero-leading rest form `(list .. rest)` is irrefutable — fall through to `check_binding_pattern`
        // (which accepts it and validates the rest binder), so the nested rest binder resolves + folds.
    }
    match check_binding_pattern(db, elem_pat, elem_ty) {
        Ok(()) => Ok(()),
        // A refutable element (literal / multi-variant ctor) → not-yet-supported in a length-dispatch list
        // match; decline honestly rather than reject (nothing is ill-formed — the refinement is unbuilt).
        Err(r) if r.code == Some(Code::NonExhaustive) => Err(Reject::unsupported(
            "a refutable list element sub-pattern (a literal or multi-variant constructor) needs \
             element-value refinement, which the length-dispatch list matcher does not support",
        )),
        // A shape error (CDZ0201), a non-linear binder (CDZ0102), or a not-yet-supported irrefutable
        // shape (a record / single-variant-sum element declines) propagates as-is.
        Err(r) => Err(r),
    }
}

/// Whether a map-pattern value sub-pattern `v` is a BARE binder / `_` (the shape the const-fold + runtime
/// paths handle directly). A NON-bare value (`(tuple x y)`, `(Some n)`, a literal `0`) needs
/// `desugar_map_value_subpatterns` to lift it into the body as a destructuring match on a fresh binder.
pub(super) fn is_bare_map_value(db: &Db, v: StructId) -> bool {
    db.ast.as_name(v).is_some()
}

/// Whether a map-pattern value sub-pattern `v` introduces NO binders — a literal `0` / `true` / `"s"`, or a
/// compound of only wildcards `(tuple _ _)`. Only such a value is safe to lift into the body by
/// `desugar_map_value_subpatterns`: its destructure-match `(match __mv (v body) …)` reuses the ORIGINAL
/// body unchanged (no binder to re-resolve), so the lowering-time desugar suffices. A value that
/// INTRODUCES binders (`(tuple x y)`, `(Some n)`) needs those binders to resolve at the RESOLVE/fault
/// stage (before lowering) — a lowering-time desugar is too late (fault-collection over the original AST
/// reports the nested binder unbound first). That is a resolve-descent increment (the map analogue of
/// Inc-1's list-element `SumPayload` sub-path); until then a binder-introducing value declines in
/// `lower_match_map`. Uses `collect_pattern_binders` into a throwaway set — empty ⇒ binder-free.
pub(super) fn map_value_is_binder_free(db: &mut Db, v: StructId) -> bool {
    let mut seen = std::collections::HashSet::new();
    // A malformed sub-pattern (Err) is not our concern here — treat as NOT binder-free so it declines
    // downstream rather than being lifted.
    collect_pattern_binders(db, v, &mut seen).is_ok() && seen.is_empty()
}

/// Whether a map value sub-pattern `v` is LIFTED into the body by `desugar_map_value_subpatterns` (a
/// `(match __mv (v body) (_ <catch-all>))` that DISPATCHES + binds) rather than handled by Inc-17's
/// resolve-stage `MapField.value_steps` descent. Two cases are lifted: (a) a BINDER-FREE value (a literal /
/// all-wildcard compound — lifting reuses the body unchanged, no binder to re-resolve); (b) a REFUTABLE
/// value that INTRODUCES binders (`(Some n)` — a multi-variant ctor / a literal nested with a binder),
/// which NEEDS the dispatching `(match __mv …)` (a bare presence read would trap on a non-matching value)
/// AND whose binders re-resolve against the synthesized `(<v> …)` arm. An IRREFUTABLE binder-introducing
/// value (`(tuple x y)`, single-variant `(Mk n)`) is NOT lifted — it always matches, so Inc-17's
/// value_steps read binds it directly (no dispatch needed, and lifting would fight the resolve memo).
pub(super) fn map_value_liftable(db: &mut Db, v: StructId) -> bool {
    if is_bare_map_value(db, v) {
        return false; // a bare binder needs no lift
    }
    if map_value_is_binder_free(db, v) {
        return true; // (a) literal / all-wildcard — lift (reuses body unchanged)
    }
    // (b) a binder-introducing value: lift iff REFUTABLE (a refutable value needs the dispatch; an
    // irrefutable one is bound by Inc-17's value_steps). Refutable ⇔ `check_binding_pattern` = CDZ0210.
    matches!(
        check_binding_pattern(db, v, &crate::ty::Ty::Any),
        Err(ref r) if r.code == Some(Code::NonExhaustive)
    )
}

/// PRE-PASS for `lower_match_map` (the map analogue of the Inc-1 list-element compose + Inc-11/12
/// refutable-element dispatch): a map-pattern VALUE sub-pattern MAY itself be any pattern — `(map ("k"
/// (tuple a b)))`, `(map ("k" (Some v)))`, `(map ("k" 0))` — not just a bare binder. Such a value is lifted
/// into the body: bind a fresh `__mvN` at that key, then `(match __mvN (<subpat> body) (_ <else>))`
/// destructures/tests it. An IRREFUTABLE sub-pattern (tuple, single-variant ctor) always matches (the `_`
/// arm is dead); a REFUTABLE one (literal, multi-variant ctor) may fail → falls through to `<else>` (the
/// next arm). This runs FIRST in `lower_match_map` — the rewritten arm's value positions are all bare
/// `__mvN` binders, so the const-fold + runtime (`desugar_runtime_map_match`) paths handle it unchanged.
/// `<else>` is threaded backward from the catch-all (as `desugar_runtime_map_match` does), so a refutable
/// value's non-match falls to the next arm. NO new IR. Returns `Some(Core)` iff some arm has a non-bare
/// value sub-pattern (else `None` — the common bare-binder case pays only the scan).
pub(super) fn desugar_map_value_subpatterns(
    db: &mut Db,
    scrutinee: StructId,
    arms: &[(StructId, StructId)],
) -> Option<Core> {
    // Detect any non-bare, BINDER-FREE value sub-pattern in a map arm. Only these are lifted (a
    // binder-introducing value needs resolve-stage descent, not this lowering desugar — see
    // `map_value_is_binder_free`). Bail if none.
    let mut any = false;
    for &(pat, _) in arms {
        let inner = match db.ast.as_form(pat, "guard") {
            Some(g) if g.len() == 2 => g[0],
            _ => pat,
        };
        if let Some((entries, _rest)) = crate::resolve::map_pattern_of(db, inner) {
            for (_, v) in entries.iter().copied() {
                if !is_bare_map_value(db, v) && map_value_liftable(db, v) {
                    any = true;
                }
            }
        }
    }
    if !any {
        return None;
    }
    // Find the FIRST unguarded catch-all — the innermost `<else>` a refutable value falls through to. (A
    // map match needs one anyway; mirror `desugar_runtime_map_match`.)
    let catch_all_ix = arms.iter().position(|&(pat, _)| {
        let inner = match db.ast.as_form(pat, "guard") {
            Some(g) if g.len() == 2 => g[0],
            _ => pat,
        };
        db.ast.as_form(pat, "guard").is_none() && db.ast.as_name(inner).is_some()
    });
    let Some(catch_all_ix) = catch_all_ix else {
        return Some(Core::Poison(Reject::coded(
            Code::NonExhaustive,
            "a map match must end in a catch-all (`_` or a whole-map binder) — a map's key set is unbounded",
        )));
    };
    // Rebuild the arms: for each arm before the catch-all, lift every non-bare value into the body. `else`
    // is threaded backward so a refutable value's non-match falls to the next arm; but the REBUILT match is
    // then re-lowered as a whole (the const/runtime paths dispatch), so `else` here is only the innermost
    // destructure-match fall-through. Keep the arm STRUCTURE (a `(map …)` arm) and rewrite in place; the
    // subsequent passes handle dispatch. Because a value-destructure can only be observed AFTER the arm's
    // keys are known present, the `(match __mv (<subpat> body) (_ <else>))` fall-through must go to the
    // catch-all body (the next arm's dispatch is handled by re-lowering the rebuilt match).
    let catch_all_body = arms[catch_all_ix].1;
    let mut new_arms: Vec<StructId> = Vec::with_capacity(arms.len());
    for (ai, &(pat, body)) in arms.iter().enumerate() {
        let (inner, guard) = match db.ast.as_form(pat, "guard") {
            Some(g) if g.len() == 2 => (g[0], Some(g[1])),
            _ => (pat, None),
        };
        let Some((entries, rest)) = crate::resolve::map_pattern_of(db, inner) else {
            new_arms.push(db.push_list(vec![pat, body]));
            continue;
        };
        // Which entries have a non-bare value? Rebuild the map pattern with those values replaced by fresh
        // `__mvN` binders, and collect `(fresh-name, original-subpattern)` to wrap the body.
        let map_head = match db.ast.get(inner) {
            crate::ast::Struct::List(items) if !items.is_empty() => items[0],
            _ => db.push_name("map"),
        };
        let mut new_entry_nodes: Vec<StructId> = Vec::with_capacity(entries.len());
        let mut lifts: Vec<(String, StructId)> = Vec::new();
        for (ei, &(k, v)) in entries.iter().enumerate() {
            // Lift a non-bare LIFTABLE value (binder-free OR refutable); an irrefutable binder-introducing
            // value is left in place (Inc-17 resolve-descent binds it via `MapField.value_steps`).
            if !is_bare_map_value(db, v) && map_value_liftable(db, v) {
                let name = format!("__mv{ai}_{ei}");
                let fresh = db.push_name(&name);
                let entry = db.push_list(vec![k, fresh]);
                new_entry_nodes.push(entry);
                lifts.push((name, v));
            } else {
                // Reuse the entry verbatim (keep the original key + value nodes).
                let entry = db.push_list(vec![k, v]);
                new_entry_nodes.push(entry);
            }
        }
        // Reassemble the `(map <entries…> [.. rest])` pattern.
        let mut map_children = vec![map_head];
        map_children.extend(new_entry_nodes.iter().copied());
        if let Some(rest_occ) = rest {
            let dd = db.push_name("..");
            map_children.push(dd);
            map_children.push(rest_occ);
        }
        let new_map = db.push_list(map_children);
        // Wrap the body: innermost-first, `(match __mvN (<subpat> body) (_ <catch-all-body>))` per lifted
        // value. A body reference to a sub-pattern binder resolves against the synthesized `(<subpat> …)`
        // (fresh nodes — nothing pre-memoized, unlike the map-value-binder case). The `_ → catch-all-body`
        // fall-through covers a refutable sub-pattern's non-match.
        let mut wrapped = body;
        for (name, subpat) in lifts.into_iter().rev() {
            let mv_ref = db.push_name(&name);
            let true_arm = db.push_list(vec![subpat, wrapped]);
            let wild = db.push_name("_");
            let else_arm = db.push_list(vec![wild, catch_all_body]);
            let match_head = db.push_name("match");
            wrapped = db.push_list(vec![match_head, mv_ref, true_arm, else_arm]);
        }
        // Re-attach the guard (if any) around the whole rewritten pattern.
        let new_pat = match guard {
            Some(g) => {
                let guard_head = db.push_name("guard");
                db.push_list(vec![guard_head, new_map, g])
            }
            None => new_map,
        };
        new_arms.push(db.push_list(vec![new_pat, wrapped]));
    }
    let match_head = db.push_name("match");
    let mut items = vec![match_head, scrutinee];
    items.extend(new_arms);
    let rewritten = db.push_list(items);
    // A REFUTABLE lifted value's binders (`n` in `(map ("a" (Some n)))`) resolved to a `MapField` at the
    // FAULT stage (Inc-17 descends value sub-patterns), but the body is now RE-PARENTED under the
    // synthesized `(match __mv ((Some n) body) …)` — so those binders must RE-RESOLVE against the
    // destructuring pattern. Drop the stale resolutions first (a binder-free lift has nothing to forget, so
    // this is a no-op cost there), then walk.
    crate::resolve::forget_subtree(db, rewritten);
    crate::resolve::resolve_subtree(db, rewritten);
    trace!(target: "rcdzc::lower", scrutinee = scrutinee.0, "map match with a lifted value sub-pattern → fresh-binder + body destructure-match");
    Some(core_of(db, rewritten))
}

/// PRE-PASS for `lower_match_map`: match a RUNTIME map by rewriting the whole match to a nested
/// PRESENCE-TEST `if`-chain the existing machinery lowers (the Inc-9 blocker, unblocked via the
/// Inc-11/12/14 desugar idiom). A runtime map's keys are not known at compile time, so the const-fold path
/// in `lower_match_map` declines "constant maps only". A key-directed arm `((map (k1 v1) …) .. rest) body)`
/// matches iff EVERY named key is present, so each arm becomes:
///
/// ```text
/// (if <k1-present> (if <k2-present> … body <else>) <else>)
/// ```
///
/// where `<kN-present>` = `(match (Map.lookup m kN) ((Some _) true) ((None) false))` and `<else>` = the
/// next arm's desugar (threaded backward from the catch-all, the innermost `<else>`). The arm BODY is
/// REUSED VERBATIM — its value/rest binders keep their `MapField` resolution (a body reference resolved
/// BEFORE this desugar and is MEMOIZED; re-binding it via a synthesized `(Some v)` would NOT take — the
/// memo wins), so the desugar supplies only DISPATCH; inside the all-keys-present branch the body's
/// `MapField` binders lower to runtime reads (`lower_map_field_runtime` emits `Map.lookup`/`Map.remove`).
/// A GUARD wraps `(if guard body <else>)`. This is the Inc-9 dispatch structure; the value read is
/// `lower_map_field`'s job. Returns `Some(Core)` iff the scrutinee is a RUNTIME map (non-`MapNew`); a
/// constant map keeps the fold.
pub(super) fn desugar_runtime_map_match(
    db: &mut Db,
    scrutinee: StructId,
    arms: &[(StructId, StructId)],
) -> Option<Core> {
    // Only fire for a RUNTIME map scrutinee — a FULLY-CONSTANT `MapNew` keeps the existing const-fold path
    // (it selects the arm at compile time, no lookups emitted). But a `MapNew` can carry a RUNTIME key or
    // value (a call/param result that did not fold — `(map ((f x) 42))`): the const path's key-presence
    // test (`const_compound_eq`) reports a runtime key ABSENT (its compile-time equality is unknown), so it
    // would take the catch-all for a key that IS present at run time — a silent MISCOMPILE (the map-match
    // twin of the `Set.contains`/`Set.remove` runtime-element fold bug). Route such a map to the runtime
    // matcher below (a `Map.lookup` chain, which compares keys by value at run time). Only a `MapNew` all of
    // whose keys AND values are compile-time constants (`is_const_value`) keeps the fold; a `Poison`
    // propagates unchanged.
    if matches!(core_of(db, scrutinee), Core::Poison(_)) {
        return None;
    }
    if matches!(core_of(db, scrutinee), Core::MapNew { .. }) && is_const_value(db, scrutinee) {
        return None;
    }
    // Require a catch-all (a bare binder / `_`) — a map's key set is unbounded, so without one the match is
    // non-exhaustive. Find the FIRST unguarded catch-all; arms after it are dead. (The const path enforces
    // the same rule; mirror it here so the desugar has a well-defined innermost `<else>`.)
    let catch_all_ix = arms.iter().position(|&(pat, _)| {
        let inner = match db.ast.as_form(pat, "guard") {
            Some(g) if g.len() == 2 => g[0],
            _ => pat,
        };
        db.ast.as_form(pat, "guard").is_none() && db.ast.as_name(inner).is_some()
    });
    let Some(catch_all_ix) = catch_all_ix else {
        return Some(Core::Poison(Reject::coded(
            Code::NonExhaustive,
            "a map match must end in a catch-all (`_` or a whole-map binder) — a map's key set is unbounded",
        )));
    };
    // Validate the key-directed arms before the catch-all: each must be a `(map …)` pattern. By the time
    // this runs, `desugar_map_value_subpatterns` has already LIFTED every non-bare value (a REFUTABLE
    // literal/multi-variant ctor into a dispatching `(match __mv …)`, an IRREFUTABLE compound bound by
    // Inc-17's `value_steps`), so the values reaching here are bare binders or irrefutable compounds. Over
    // a RUNTIME map the nested-binder READ is wired (`f3f8f94e`): `lower_map_field_runtime` walks
    // `value_steps` after the `Map.lookup` via `synth_value_path_read`.
    for &(pat, _) in &arms[..catch_all_ix] {
        let inner = match db.ast.as_form(pat, "guard") {
            Some(g) if g.len() == 2 => g[0],
            _ => pat,
        };
        let Some((entries, _rest)) = crate::resolve::map_pattern_of(db, inner) else {
            // A `(map …)` form whose `..` rest is MALFORMED (`..` not followed by exactly one binder):
            // name the REST-SHAPE fault clearly (CDZ0201), the MAP twin of the list rest-shape message
            // (`lower_match_list` "a list rest pattern is `(list p… .. rest)` — exactly one binder after
            // `..`"). Without this, the generic "not a `(map …)` pattern" fired — or, worse, the arm's
            // binders leaked UNBOUND (v-diagnostics note); the resolver now keeps those binders inert
            // (`map_form_is_malformed_rest`), so THIS coded reject is the sole, actionable diagnostic.
            if crate::resolve::map_form_is_malformed_rest(db, inner) {
                return Some(Core::Poison(
                    Reject::coded(
                        Code::Malformed,
                        "a map rest pattern is `(map (k v) … .. rest)` — exactly one binder after `..`",
                    )
                    .at(inner),
                ));
            }
            // A non-`(map …)`, non-binder arm — the COMMON way to reach here is a ctor-shaped head over a
            // MAP scrutinee (`((Zorp x) …)` unbound, `((. Map Foo) …)` non-member): a map has no user
            // constructors. When that head resolves to a CODED poison (CDZ0101 unbound / CDZ0201
            // non-member), propagate it instead of the generic UNCODED decline — the MAP twin of the
            // list matcher's coded-head propagation (Inc 39), so `type_errors`' `match_pattern_fault`
            // surfaces it in `cdz check` on EVERY body, not only the nullary-exported ones the emit-path
            // lowering walk reaches (the check≡compile hole a recursive/parameterized body fell into). A
            // head that is not an unbound/non-member name yields no coded poison → the honest uncoded
            // decline stands for a genuinely not-yet-lowerable map-arm shape; no false alarm.
            let ctor_head = match db.ast.get(inner) {
                crate::ast::Struct::List(children) => children.first().copied(),
                _ => None,
            };
            if let Some(head) = ctor_head
                && let Core::Poison(reject) = core_of(db, head)
                && reject.code.is_some()
            {
                return Some(Core::Poison(reject));
            }
            return Some(Core::Poison(Reject::decline(
                "a map match arm that is not a `(map …)` pattern or a binder is not supported",
            )));
        };
        // The irrefutability guard now sees only bare binders + irrefutable compounds (refutable values
        // were lifted out above); their binders read via `MapField` `value_steps`.
        for &(_, v) in &entries {
            if let Err(r) = map_value_irrefutable_or_decline(db, v) {
                return Some(Core::Poison(r));
            }
        }
    }
    // The innermost `<else>` = the catch-all arm's body (reused in place; a binder catch-all reads the whole
    // map via the Case-5 binder→scrutinee rule).
    let mut else_node = arms[catch_all_ix].1;
    // Fold the key-directed arms (before the catch-all) from LAST backward into nested presence-test `if`s.
    // KEY DESIGN: the arm body is REUSED VERBATIM — its value/rest binders keep their `MapField` resolution
    // (a body reference resolved BEFORE this desugar and is memoized; re-binding via a synthesized `(Some
    // v)` would NOT take — the memo wins). So the desugar supplies only the DISPATCH: a nested
    // `(if <k-present> … <else>)` presence chain, where each `<k-present>` = `(match (Map.lookup m k) ((Some
    // _) true) ((None) false))`. Inside the (all-keys-present) branch the body's `MapField` binders lower to
    // runtime reads (`lower_map_field` emits `Map.lookup`/`Map.remove` for a runtime scrutinee). This is the
    // Inc-9 dispatch+rest structure; the value read is `lower_map_field`'s job.
    for &(pat, body) in arms[..catch_all_ix].iter().rev() {
        let (inner, guard) = match db.ast.as_form(pat, "guard") {
            Some(g) if g.len() == 2 => (g[0], Some(g[1])),
            _ => (pat, None),
        };
        let (entries, _rest) = crate::resolve::map_pattern_of(db, inner)?;
        // RESOLVE the reused body + guard cond NOW, while they are still parented under the intact
        // `(guard (map …) cond)` arm — BEFORE the `push_list` below re-parents them into the synthesized
        // `(if guard body <else>)` chain, whose ancestry no longer reaches the `(map …)` arm. A map
        // value/rest binder resolves via `binder_in` Case M / `guard_cond_map_binds`, both of which ASCEND
        // to the enclosing `(map …)` pattern to recover the scrutinee — a PARENT-STRUCTURE-DEPENDENT
        // resolution. On the ORIGINAL match the arm's binders were already resolved+memoized before this
        // desugar, so reuse-verbatim works; but on an EVAL-MINTED CLONE of the match (a partial-const map —
        // looked-up key const, another key runtime — makes the guard-fold speculatively re-materialize the
        // arm) the clone's body/guard are FRESH, UNMEMOIZED nodes. Once wrapped in the if-chain their ascent
        // no longer finds the `(map …)` arm, so `v`/`rest` re-resolve UNBOUND → CDZ0101 → invalid module.
        // Populating the memo here (against the still-intact arm) fixes the clone; it is a no-op on the
        // original (already memoized). The map twin of the list desugar's orig_match_parent re-resolve.
        crate::resolve::resolve_subtree(db, body);
        if let Some(g) = guard {
            crate::resolve::resolve_subtree(db, g);
        }
        // The taken-branch body: reuse verbatim, then wrap a guard `(if guard body <else>)` (a false guard
        // falls through to the same else as a missing key).
        let mut taken = body;
        if let Some(guard) = guard {
            let if_head = db.push_name("if");
            taken = db.push_list(vec![if_head, guard, taken, else_node]);
        }
        // Nest a presence test per named key: `(if <k-present> <inner> <else>)`, INNERMOST last so the first
        // key is the OUTERMOST test. `<k-present>` = `(match (Map.lookup m k) ((Some _) true) ((None) false))`.
        let mut chain = taken;
        for &(k, _v) in entries.iter().rev() {
            let dot = db.push_name(".");
            let map_mod = db.push_name("Map");
            let lookup_key = db.push_name("lookup");
            let lookup_member = db.push_list(vec![dot, map_mod, lookup_key]);
            let k_copy = clone_key_expr(db, k);
            let lookup_call = db.push_list(vec![lookup_member, scrutinee, k_copy]);
            let some_head = db.push_name("Some");
            let wild_p = db.push_name("_");
            let some_pat = db.push_list(vec![some_head, wild_p]);
            let true_node = db.push_atom(crate::ast::Leaf::Bool(true));
            let some_arm = db.push_list(vec![some_pat, true_node]);
            let none_head = db.push_name("None");
            let none_pat = db.push_list(vec![none_head]);
            let false_node = db.push_atom(crate::ast::Leaf::Bool(false));
            let none_arm = db.push_list(vec![none_pat, false_node]);
            let match_head = db.push_name("match");
            let present = db.push_list(vec![match_head, lookup_call, some_arm, none_arm]);
            let if_head = db.push_name("if");
            chain = db.push_list(vec![if_head, present, chain, else_node]);
        }
        else_node = chain;
    }
    // Give the synthesized presence-test chain scope-skip entries BEFORE resolving it: the chain nests
    // O(arms) deep, and every node is a non-binding `if`/`match`/`.`/application form, so without this a
    // prelude name (`Map`/`lookup`/`Some`/`None`) in an inner arm walks O(depth) parents to conclude "not
    // lexically bound" → O(N²) over N arms. The pass-through skip makes each such resolution O(1).
    db.extend_scope_skip_pass_through(else_node);
    crate::resolve::resolve_subtree(db, else_node);
    trace!(target: "rcdzc::lower", scrutinee = scrutinee.0, "runtime map match → nested presence-test if-chain (body MapField reads runtime)");
    Some(core_of(db, else_node))
}

/// A FRESH copy of a map-pattern KEY expression for use in a synthesized `Map.lookup`/`Map.remove` call.
/// A key is a value expression (a literal or a scoped name) sitting in a map PATTERN; reused verbatim it
/// would carry its pattern-context parent, so copy it: a literal atom clones its leaf; a bare name clones
/// the name; any other (a compound key expression) is reused (its own subtree resolves independently).
pub(super) fn clone_key_expr(db: &mut Db, k: StructId) -> StructId {
    match db.ast.get(k) {
        crate::ast::Struct::Atom(lid) => {
            let leaf = db.ast.leaf(*lid).clone();
            db.push_atom(leaf)
        }
        _ => k,
    }
}

/// Read a `(set …)` element-membership pattern: its named ELEMENTS (each an ordinary value expression —
/// NOT a binder, since a set element IS the value) and an OPTIONAL single rest binder. Accepts any `#set`
/// spelling via `compound_form_of`; splits an optional trailing `.. rest` via `rest_marker` (wrapped
/// `(.. r)` or flat). Returns `None` if `pat` is not a set form OR its `..` is MALFORMED (a `..` not
/// followed by exactly one rest binder) — the set twin of `map_pattern_of`.
pub(crate) fn set_pattern_of(db: &Db, pat: StructId) -> Option<(Vec<StructId>, Option<StructId>)> {
    let tail = db
        .ast
        .compound_form_of(pat, crate::ast::CompoundCtor::Set)?;
    let (elems_tail, rest) = match db.ast.rest_marker(tail) {
        Some((i, operand, trailing_start)) => {
            if trailing_start != tail.len() {
                return None; // `..` must be followed by exactly one rest binder
            }
            (&tail[..i], Some(operand))
        }
        None => (tail, None),
    };
    Some((elems_tail.to_vec(), rest))
}

/// Whether `pat` is a `(set …)` FORM whose `..` rest is MALFORMED (a `..` marker NOT followed by exactly
/// one binder) — the set twin of `map_form_is_malformed_rest`, so the matcher names the rest-shape fault
/// clearly rather than the generic "not a set pattern".
fn set_form_is_malformed_rest(db: &Db, pat: StructId) -> bool {
    let Some(tail) = db.ast.compound_form_of(pat, crate::ast::CompoundCtor::Set) else {
        return false;
    };
    match db.ast.rest_marker(tail) {
        Some((_, _, trailing_start)) => trailing_start != tail.len(),
        None => false,
    }
}

/// `((. Set <member>) <a> <b>)` — a curried `Set` member call (`contains`/`remove`) over two args, the
/// surface a source `(Set.contains s e)` / `(Set.remove s e)` lowers through. A FRESH `.`/`Set`/member node
/// set per call (a node has one parent); `s` and `e` are spliced verbatim (already-resolved / freshly
/// cloned by the caller).
fn set_member_call(db: &mut Db, member: &str, a: StructId, b: StructId) -> StructId {
    let dot = db.push_name(".");
    let set_mod = db.push_name("Set");
    let member_key = db.push_name(member);
    let access = db.push_list(vec![dot, set_mod, member_key]);
    db.push_list(vec![access, a, b])
}

/// Lower a match over a SET scrutinee by ELEMENT-MEMBERSHIP patterns (`core-semantics.md` §A Set Is Matched
/// By Element-Membership Patterns). A `(set e… .. rest)` arm matches when the set CONTAINS every named
/// element `e` (each an ordinary value expression compared by the set's own value equality); a set lacking
/// any named element does NOT match, so matching falls through to a later arm. An optional `.. rest` binds a
/// set of the same type containing every element EXCEPT the named ones. A set's element set is UNBOUNDED, so
/// a `(set …)` arm covers no shape — the match needs a MANDATORY catch-all (a whole-set binder / `_`, else
/// CDZ0210). This is the KEYS-ONLY twin of `lower_match_map`: a set is a map with only keys and no value
/// binders, so membership is a `Set.contains` presence-test `if`-chain and the residual is a `Set.remove`
/// chain BOUND BY A `let` (`rest : (Set E)` types via the `Set.remove` scheme — no new PathStep/infer arm,
/// per the inference contract). The desugar works over BOTH a constant `Core::SetOf` and a runtime set
/// (`Set.contains`/`Set.remove` evaluate either way), so — unlike the map matcher's split const/runtime
/// paths — one membership-chain desugar serves both.
//= spec/capabilities/core-semantics.md#a-set-is-matched-by-element-membership-patterns
//# A membership pattern naming elements MUST match a set that CONTAINS every named element, binding a trailing rest binder to the set of remaining elements; because a set's element set is unbounded, a match on a set MUST end in a name or wildcard catch-all (else a compile-time error under Matching Is Exhaustive Or Rejected).
pub(super) fn lower_match_set(
    db: &mut Db,
    scrutinee: StructId,
    arms: &[(StructId, StructId)],
) -> Core {
    if matches!(core_of(db, scrutinee), Core::Poison(_)) {
        return core_of(db, scrutinee);
    }
    // MANDATORY catch-all: the FIRST unguarded bare-binder / `_` arm (arms after it are dead). A set's
    // element set is unbounded, so without one the match is non-exhaustive (CDZ0210) — mirror the map matcher.
    let catch_all_ix = arms.iter().position(|&(pat, _)| {
        let inner = match db.ast.as_form(pat, "guard") {
            Some(g) if g.len() == 2 => g[0],
            _ => pat,
        };
        db.ast.as_form(pat, "guard").is_none() && db.ast.as_name(inner).is_some()
    });
    let Some(catch_all_ix) = catch_all_ix else {
        return Core::Poison(Reject::coded(
            Code::NonExhaustive,
            "a set match must end in a catch-all (`_` or a whole-set binder) — a set's element set is unbounded"
                .to_string(),
        ));
    };
    // Validate the membership arms before the catch-all: each must be a well-formed `(set …)` pattern. A
    // MALFORMED `..` (not followed by exactly one binder) names the rest-shape fault (CDZ0201) — the set twin
    // of the map/list rest-shape message.
    for &(pat, _) in &arms[..catch_all_ix] {
        let inner = match db.ast.as_form(pat, "guard") {
            Some(g) if g.len() == 2 => g[0],
            _ => pat,
        };
        let Some((elems, rest)) = set_pattern_of(db, inner) else {
            if set_form_is_malformed_rest(db, inner) {
                return Core::Poison(
                    Reject::coded(
                        Code::Malformed,
                        "a set rest pattern is `(set e… .. rest)` — exactly one binder after `..`"
                            .to_string(),
                    )
                    .at(inner),
                );
            }
            return Core::Poison(Reject::decline(
                "a set match arm that is not a `(set …)` pattern or a binder is not supported",
            ));
        };
        // A bare-NAME element that resolves UNBOUND (`#set(a b)` with `a` not in scope). A set element is an
        // ordinary membership VALUE expression, NOT a binder (a set has no positional structure to bind —
        // core-semantics §A Set Is Matched By Element-Membership Patterns, v-spec-oracle ruling #6685), so the
        // CODE stays CDZ0101 (unbound value) — but a user who wrote `#set(a b)` INTENDING to bind gets a bare
        // "unbound name `a`" that reads like a typo. STEER them here (BEFORE the fold reuses + reparents the
        // element out of its set-pattern position, which is why the resolver's generic unbound path can't add
        // this context). An IN-SCOPE element resolves cleanly (#6693); only a genuinely-unbound name is caught.
        for &e in &elems {
            if let Some(nm) = db.ast.as_name(e).map(str::to_string)
                && matches!(
                    crate::resolve::resolved_of(db, e),
                    crate::resolved::Resolved::Poison(ref rj) if rj.code == Some(Code::Unbound)
                )
            {
                return Core::Poison(
                    Reject::coded(
                        Code::Unbound,
                        format!(
                            "unbound name `{nm}` — a set pattern names its members BY VALUE and does not bind \
                             them (a set has no positional structure to destructure). Use an in-scope value to \
                             test membership, or bind the whole set with `_` / a name and query it with \
                             `Set.contains` / `Set.len`."
                        ),
                    )
                    .at(e),
                );
            }
        }
        let _ = rest;
    }
    // Whether ANY membership arm has a `.. rest` binder. A rest arm's body (+ guard cond) may reference the
    // rest binder — which the resolver binds to `Resolved::SetRest` (Case 6set-rest, typed `(Set E)`) — so
    // it is DEEP-COPIED (a fresh, unmemoized `rest` the follow-up `resolve_subtree` re-binds to the
    // synthesized `let`, NOT the `SetRest` core compute.rs declines), then bound by a `let` over the
    // `Set.remove` residual. A membership-only match reuses bodies verbatim (memos stand) and needs no graft.
    let any_rest = arms[..catch_all_ix].iter().any(|&(pat, _)| {
        let inner = match db.ast.as_form(pat, "guard") {
            Some(g) if g.len() == 2 => g[0],
            _ => pat,
        };
        matches!(set_pattern_of(db, inner), Some((_, Some(_))))
    });
    // Graft target (used only when `any_rest`): a copied rest-arm body's free names ascend the synthesized
    // chain → the original match → the enclosing binder, exactly as `fuse_match_into_if` grafts.
    let orig_match = db.parent_of(scrutinee);
    let scrut_ix = db.child_ix_of(scrutinee) as u32;
    // The innermost `<else>` = the catch-all body (a binder catch-all reads the whole set via the Case-5
    // binder→scrutinee rule). Fold the membership arms from LAST backward into nested `Set.contains` `if`s.
    let mut else_node = arms[catch_all_ix].1;
    for &(pat, body) in arms[..catch_all_ix].iter().rev() {
        let (inner, guard) = match db.ast.as_form(pat, "guard") {
            Some(g) if g.len() == 2 => (g[0], Some(g[1])),
            _ => (pat, None),
        };
        let (elems, rest) =
            set_pattern_of(db, inner).expect("validated above (well-formed set pattern)");
        // For a REST arm, the body (+ guard) reference `rest` (resolved to `SetRest`) — DEEP-COPY them so the
        // copy's `rest` is fresh and re-binds to the `let` below (a pinned β-capture is shared,
        // `clone_subtree_db_for_fused(_, None)`). A membership-only arm REUSES body/guard verbatim (memos
        // stand — an in-scope element keeps its resolution; no residual binder to introduce).
        let (body_u, guard_u) = match rest {
            Some(_) => (
                clone_subtree_db_for_fused(db, body, None),
                guard.map(|g| clone_subtree_db_for_fused(db, g, None)),
            ),
            None => (body, guard),
        };
        let mut taken = body_u;
        if let Some(g) = guard_u {
            let if_head = db.push_name("if");
            taken = db.push_list(vec![if_head, g, taken, else_node]);
        }
        // A REST binder (`.. rest`, not `_`): bind it to `(Set.remove (… (Set.remove s e1) …) en)` — the set
        // MINUS the named elements — via a `let`. The copied body's `rest` then re-resolves to this `let`
        // (nearest binder), so no bare `SetRest` core reaches compute.rs. `rest = _` skips the dead `let`.
        if let Some(rest_binder) = rest
            && db.ast.as_name(rest_binder) != Some("_")
        {
            let mut residual = scrutinee;
            for &e in &elems {
                let e_copy = clone_key_expr(db, e);
                residual = set_member_call(db, "remove", residual, e_copy);
            }
            let binder_occ = clone_key_expr(db, rest_binder);
            let binding = db.push_list(vec![binder_occ, residual]);
            let bindings = db.push_list(vec![binding]);
            let let_head = db.push_name("let");
            taken = db.push_list(vec![let_head, bindings, taken]);
        }
        // Nest a `Set.contains` presence test per named element: `(if (Set.contains s e) <inner> <else>)`,
        // INNERMOST last so the FIRST element is the OUTERMOST test. A MEMBERSHIP-only arm uses each element
        // once, so it is REUSED VERBATIM (keeping its resolved memo — a cloned NAME atom would lose its
        // pattern-position scope ancestry and re-resolve unbound). A REST arm already consumed each element in
        // the `Set.remove` chain above, so here it CLONES (a second occurrence), re-resolving via the graft.
        let clone_elems = rest.is_some();
        let mut chain = taken;
        for &e in elems.iter().rev() {
            let e_use = if clone_elems {
                clone_key_expr(db, e)
            } else {
                e
            };
            let contains = set_member_call(db, "contains", scrutinee, e_use);
            let if_head = db.push_name("if");
            chain = db.push_list(vec![if_head, contains, chain, else_node]);
        }
        else_node = chain;
    }
    // GRAFT the synthesized chain under the original match node (at the scrutinee's child slot) when a rest
    // arm copied a body — so a free name in the copy ascends → this match → the enclosing binder (the
    // fusion's discipline). The original match is lowered away after `core_of`, so the transient slot reuse
    // is harmless. A membership-only match reuses bodies (memos stand) and needs no graft.
    if any_rest && let Some(orig) = orig_match {
        db.reparent(else_node, Some(orig), scrut_ix);
    }
    // Scope-skip the synthesized chain (O(arms) deep of non-binding `if`/`.`/application forms) so a prelude
    // name (`Set`/`contains`/`remove`) resolution is O(1), not O(depth) — mirror the map matcher. ONLY for a
    // membership-only match: `extend_scope_skip_pass_through` assumes every synth form BINDS NOTHING (marks
    // it scope-transparent), but a REST arm's synthesized `let` DOES bind `rest`; marking it pass-through
    // would make the copied body's `rest` skip the binding → resolve UNBOUND. When any rest arm is present,
    // SKIP the optimization (resolution still works via the ordinary parent walk; a rest match is small).
    if !any_rest {
        db.extend_scope_skip_pass_through(else_node);
    }
    crate::resolve::resolve_subtree(db, else_node);
    trace!(target: "rcdzc::lower", scrutinee = scrutinee.0, "set match → nested Set.contains if-chain (rest = Set.remove residual let)");
    core_of(db, else_node)
}

/// Lower a match over a MAP scrutinee by KEY-DIRECTED patterns (ask-61, core-semantics.md §A Map Is
/// Matched By Key-Directed Patterns). A `(map (k p) …)` arm matches when the map HAS every named key `k`
/// (each bound to a value the body reads via a `MapField`); a bare binder / `_` is a catch-all. A CONSTANT
/// `Core::MapNew` scrutinee FOLDS: a named key is present iff some entry's key is `const_compound_eq` to
/// it, so the first arm whose keys are all present is selected and its body lowered (the body's `MapField`
/// binders then fold). A RUNTIME map is handled by `desugar_runtime_map_match` (a nested `Map.lookup`
/// chain) — the pre-pass below. A map's key set is UNBOUNDED, so a `(map …)` arm covers no shape — the
/// match needs a catch-all (else CDZ0210). A value sub-pattern MAY be a bare binder OR an irrefutable
/// compound (tuple / single-variant ctor, read via `value_steps`); a refutable value sub-pattern declines.
///
/// The runtime-map matcher (`90cd317e`) realizes most of this section: a `(map (k v) …)` pattern names
/// keys with value binders and MAY end in a `.. rest`; it matches iff every named key is PRESENT (the
/// presence-test chain), and a lacking key falls through to a later arm; each named key is an ordinary
/// value expression compared by the map's own value equality (`Map.lookup`/`const_compound_eq`); the rest
/// binder binds the map minus the named keys (`Map.remove` chain); the catch-all is MANDATORY (a map's
/// key set is unbounded — else CDZ0210); and the pattern observes the map ONLY through key presence and
/// associated values (`Map.lookup`/`Map.remove`), never its CHAMP node ordering or placement.
//= spec/capabilities/core-semantics.md#a-map-is-matched-by-key-directed-patterns
//# A map MUST be matchable by a key-directed pattern that names some number of keys, each with a value binder position, and MAY end in a rest binder for the remaining entries.
//= spec/capabilities/core-semantics.md#a-map-is-matched-by-key-directed-patterns
//# A key-directed pattern naming keys `k₁ … kₙ` MUST match a map that CONTAINS every named key, binding each key's value binder to the value the map associates with that key; a map lacking any named key MUST NOT match that pattern, so that matching falls through to a later arm.
//= spec/capabilities/core-semantics.md#a-map-is-matched-by-key-directed-patterns
//# Each named key MUST be an ordinary value expression, compared to the map's keys by the same value equality the map itself uses, so that a key computed at run time selects an entry exactly as a constant key does.
//= spec/capabilities/core-semantics.md#a-map-is-matched-by-key-directed-patterns
//# A pattern MAY end in a rest binder that binds a map of the same type containing every entry of the matched map EXCEPT the named keys, so that the named entries are consumed and the remainder is available for further matching.
//= spec/capabilities/core-semantics.md#a-map-is-matched-by-key-directed-patterns
//# Because a map's key set is unbounded, no finite set of key-directed patterns can cover every map, so a match on a map MUST end in a name or wildcard pattern that binds the whole map; a set of key-directed arms with no such catch-all MUST be a compile-time error under *Matching Is Exhaustive Or Rejected*.
//= spec/capabilities/core-semantics.md#a-map-is-matched-by-key-directed-patterns
//# A key-directed pattern MUST observe a map only through the presence of keys and the values it associates with them; it MUST NOT expose or depend on any internal ordering or node structure of the map's representation, so that the same pattern matches a map regardless of how the map is represented.
// A value binder position is itself a binder position (Patterns Compose): a value MAY be a wildcard, a
// name, a tuple pattern, or a constructor pattern (single- OR multi-variant), matched recursively to any
// depth against the value at the key — the map value-compose axis is now COMPLETE. An IRREFUTABLE value
// (bare binder / tuple / single-variant ctor) binds via Inc-17's `MapField.value_steps` (resolve descends
// the sub-path; `880c95b6` const, `f3f8f94e` runtime); a REFUTABLE value (a literal, `5bc7215e`; or a
// multi-variant ctor `(Some n)`, `d4a471d3`) is LIFTED into a dispatching `(match __mv (<value> body) (_
// <catch-all>))` by `desugar_map_value_subpatterns` (via `map_value_liftable`), so a non-matching value
// falls through to the next arm — over both constant and runtime maps. The whole-arm CDZ0102 linearity
// walk spans the value binders. All of §4's enumerated kinds bind, refutable or not.
//= spec/capabilities/core-semantics.md#a-map-is-matched-by-key-directed-patterns
//# Each value binder position MUST be a binder position in the sense of *Patterns Compose*, so a value MAY be bound by any pattern (a wildcard, a name, a tuple pattern, a constructor pattern) matched recursively against the value at that key, and the whole pattern MUST remain linear (`CDZ0102`).
pub(super) fn lower_match_map(
    db: &mut Db,
    scrutinee: StructId,
    arms: &[(StructId, StructId)],
) -> Core {
    // WELL-FORMEDNESS (before any desugar): each pattern KEY must type-agree with the map's key type. A
    // `(map ("x" v))` pattern on a `(Map Int64 Int64)` writes a String key where an Int64 is required —
    // without this the const path folds it to a never-matching arm (`const_compound_eq` String-vs-Int is
    // `None`) and the RUNTIME path (`desugar_runtime_map_match`) emits a `Map.lookup` that never hits,
    // both accepting a mistyped key as dead code. Run BEFORE the pre-passes (the runtime desugar rebuilds
    // the match and recurses, so a check placed after it would never see a runtime map's keys) so BOTH
    // const and runtime maps reject a wrong-type key: CDZ0201 naming both types, anchored at the key — the
    // map twin of the scalar-match + list-element pattern-type checks. `Any` key (unsolved) is not checked.
    if let crate::ty::Ty::Map(key_ty, _) = crate::infer::type_of(db, scrutinee)
        && !matches!(*key_ty, crate::ty::Ty::Any)
    {
        for &(pat, _) in arms {
            let inner = match db.ast.as_form(pat, "guard") {
                Some(g) if g.len() == 2 => g[0],
                _ => pat,
            };
            if let Some((pat_entries, _rest)) = crate::resolve::map_pattern_of(db, inner) {
                for &(k, _) in &pat_entries {
                    let kt = crate::infer::type_of(db, k);
                    if !matches!(kt, crate::ty::Ty::Any) && !kt.agrees_with(&key_ty) {
                        return Core::Poison(
                            Reject::coded(
                                Code::Malformed,
                                format!(
                                    "this map-pattern key is {}, but the map's keys are {}",
                                    kt.render_name(&db.name_ctx()),
                                    key_ty.render_name(&db.name_ctx())
                                ),
                            )
                            .at(k),
                        );
                    }
                }
            }
        }
    }
    // PRE-PASS (value sub-patterns): a non-bare map VALUE sub-pattern `(map ("k" (tuple a b)))` /
    // `(map ("k" (Some v)))` / `(map ("k" 0))` is lifted into the body as `(match __mv (<subpat> body) (_
    // <catch-all>))` on a fresh `__mv` binder. Runs FIRST: the rewritten arm's values are all bare binders,
    // so the const/runtime paths below handle it unchanged. Fires only when some arm has a non-bare value.
    if let Some(core) = desugar_map_value_subpatterns(db, scrutinee, arms) {
        return core;
    }
    // PRE-PASS: a RUNTIME map scrutinee desugars to a nested `Map.lookup` chain (the const path below only
    // handles a compile-time-constant `MapNew`). Fires only for a non-constant map. The desugar rebuilds
    // the WHOLE match (dispatch by key presence) so the arm-selection is a runtime `Map.lookup`/`None`
    // fall-through; the value/rest binders are re-bound by the synthesized `(Some v)` / `let rest` — a body
    // reference re-resolves against those, NOT the original `MapField` (whose lowering only folds a
    // constant map). See `desugar_runtime_map_match`.
    if let Some(core) = desugar_runtime_map_match(db, scrutinee, arms) {
        return core;
    }
    // The constant scrutinee's entries (the corpus shape: an inline `Map.insert` chain / `(map …)`).
    let entries = match core_of(db, scrutinee) {
        Core::MapNew { entries, .. } => entries,
        Core::Poison(r) => return Core::Poison(r),
        _ => {
            return Core::Poison(Reject::decline(
                "matching a runtime map by key-directed patterns needs the runtime map matcher (constant maps only)",
            ));
        }
    };
    // A key is present in the constant map iff some entry's key compares equal to it (by value).
    let key_present = |db: &mut Db, k: StructId, es: &[(StructId, StructId)]| -> bool {
        es.iter()
            .any(|&(ek, _)| const_compound_eq(db, ek, k) == Some(true))
    };
    // WELL-FORMEDNESS: a `(map …)` arm covers no shape (a map's key set is unbounded), so the arms must
    // include a CATCH-ALL (a bare binder / `_`) or the match is non-exhaustive (CDZ0210).
    let has_catch_all = arms.iter().any(|&(pat, _)| db.ast.as_name(pat).is_some());
    if !has_catch_all {
        return Core::Poison(Reject::coded(
            Code::NonExhaustive,
            "a map match must end in a catch-all (`_` or a whole-map binder) — a map's key set is unbounded",
        ));
    }
    for &(pat, body) in arms {
        // A bare binder / `_` is a catch-all — it always matches.
        if db.ast.as_name(pat).is_some() {
            return core_of(db, body);
        }
        // A `(map (k p) … .. rest)` pattern: matches iff EVERY named key is present in the constant map.
        let map_inner = match db.ast.as_form(pat, "guard") {
            Some(g) if g.len() == 2 => g[0],
            _ => pat,
        };
        let Some((pat_entries, _rest)) = crate::resolve::map_pattern_of(db, map_inner) else {
            // A `(map …)` form with a MALFORMED `..` rest — the clear rest-shape CDZ0201 (the const-map
            // twin of the runtime-map path above; the MAP twin of the list rest-shape message).
            if crate::resolve::map_form_is_malformed_rest(db, map_inner) {
                return Core::Poison(
                    Reject::coded(
                        Code::Malformed,
                        "a map rest pattern is `(map (k v) … .. rest)` — exactly one binder after `..`",
                    )
                    .at(map_inner),
                );
            }
            // A ctor-shaped head over a (constant) MAP scrutinee — propagate the head's CODED poison
            // (CDZ0101 unbound / CDZ0201 non-member) so `cdz check` surfaces it via `match_pattern_fault`,
            // the const-map twin of the runtime-map path above (both the MAP analogue of the list
            // matcher's Inc 39 coded-head propagation). A non-name head keeps the honest uncoded decline.
            let ctor_head = match db.ast.get(map_inner) {
                crate::ast::Struct::List(children) => children.first().copied(),
                _ => None,
            };
            if let Some(head) = ctor_head
                && let Core::Poison(reject) = core_of(db, head)
                && reject.code.is_some()
            {
                return Core::Poison(reject);
            }
            return Core::Poison(Reject::decline(
                "a map match arm that is not a `(map …)` pattern or a binder is not supported",
            ));
        };
        // A value sub-pattern MAY be a bare binder OR an IRREFUTABLE nested pattern (`(tuple x y)`, a
        // single-variant `(Mk n)`) whose binders read via `MapField` `value_steps` (resolve descends them).
        // A REFUTABLE value sub-pattern (a literal, a multi-variant ctor `(Some n)`) is DISPATCHED: it is
        // lifted by `desugar_map_value_subpatterns` (runs before this) into a `(match __mv …)`, so only bare
        // binders + irrefutable compounds reach this irrefutability guard.
        for &(_, v) in &pat_entries {
            if let Err(r) = map_value_irrefutable_or_decline(db, v) {
                return Core::Poison(r);
            }
        }
        let all_present = pat_entries
            .iter()
            .all(|&(k, _)| key_present(db, k, &entries));
        if all_present {
            // A GUARD (guarded map arm) is evaluated after the keys are present. The guard cond reads the
            // value/rest binders (Case 6mg → `MapField`, folding to constants over a const scrutinee), so
            // `core_of(cond)` folds to `ConstBool`. TRUE → this arm matches; FALSE → fall through to the next
            // arm (mirrors the §4b bin const-path guard fold + the scalar guard fold). A guard that does not
            // fold to a constant over a CONST scrutinee is a decline (a runtime op leaked into a const fold).
            let guard = match db.ast.as_form(pat, "guard") {
                Some(g) if g.len() == 2 => Some(g[1]),
                _ => None,
            };
            if let Some(cond) = guard {
                match core_of(db, cond) {
                    Core::ConstBool(true) => return core_of(db, body),
                    Core::ConstBool(false) => continue,
                    Core::Poison(r) => return Core::Poison(r),
                    _ => {
                        return Core::Poison(Reject::decline(
                            "a guarded map-match arm over a constant scrutinee did not fold its guard to a constant",
                        ));
                    }
                }
            }
            // This arm matches — its body's `MapField` binders fold against the constant scrutinee.
            return core_of(db, body);
        }
        // Else fall through to the next arm (the key-directed pattern is a genuine presence test).
    }
    Core::Poison(Reject::decline(
        "map match: no arm matched the constant map (unreachable — a catch-all was required)",
    ))
}
