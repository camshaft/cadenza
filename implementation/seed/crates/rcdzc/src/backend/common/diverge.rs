//! Divergence + flow-refinement analyses over the structured `Core` — backend-AGNOSTIC (no wasm `Lir`,
//! no Rust source). Both backends consult these: `body_diverges` decides whether an `if`/`match` yields
//! no value on any path (so the emit gives it a 0-result/`Never` shape rather than declining "no machine
//! representation"), and `refined_frame_for_branch` computes the interval a branch condition guarantees
//! (so a guard the narrowed range proves dead can be elided). Hoisted here from `backend/wasm/select.rs`
//! (operator shared-backend-code directive) so neither backend reaches into the other.

use crate::ast::StructId;
use crate::core::Core;
use crate::db::{Db, ValueFact};
use crate::infer::type_of;
use crate::lower::core_of;
use crate::resolved::Prim;
use crate::ty::Ty;

/// Whether the body at `id` PROVABLY diverges on every path — it produces no value on any control path,
/// so its result type is `Never` (no machine representation). Used at the core-signature site (a
/// diverging function body gets a 0-result signature) and at the `Core::If`/`Match` emit (a both-branches-
/// diverge conditional emits an empty block + trailing `unreachable` rather than declining "no machine
/// representation"). Recognizes a bare trap AND a trap threaded through `Seq`/`Let`, both branches of an
/// `if`, and every arm of a `match`/`match-list`/`match-sum`.
pub(crate) fn body_diverges(db: &mut Db, id: StructId) -> bool {
    match core_of(db, id) {
        // All trap forms diverge — `Core::TrapDivZero`/`Core::TrapOverflow` are `Core::Trap`s that preserve
        // the divide-by-zero / integer-overflow kind (`lower::demote_conditional_trap`); each halts on every
        // path exactly the same.
        Core::Trap | Core::TrapDivZero | Core::TrapOverflow => true,
        // A sequence's value is its tail; the statements run for effect. Diverges iff the tail does.
        Core::Seq { tail, .. } => body_diverges(db, tail),
        // A `let`'s value is its body; the bindings are evaluated first. Diverges iff the body does.
        Core::Let { body, .. } => body_diverges(db, body),
        // An `if` whose BOTH branches diverge produces no value on any path — its result type is `Never`
        // (a fresh var / `Any`, no machine rep). `(if b (trap …) (trap …))`: inference types it `Never`
        // (cdz check passes), and the emit must NOT decline "no machine representation" — both arms end in
        // `unreachable`, so the block yields nothing and gets a UNIT/Empty block type at the `Core::If`
        // emit (mirrors the diverging-fn-body → 0-result signature). Diverges iff BOTH branches do; a
        // one-sided diverge (`(if b (trap) x)`) still yields `x`'s value, so it does NOT.
        Core::If { then_, else_, .. } => body_diverges(db, then_) && body_diverges(db, else_),
        // A `match` whose EVERY reachable arm body diverges produces no value on any path — `Never`, same
        // as the both-diverge `if`. `(match n (0 (trap …)) (_ (trap …)))`: inference types it `Never`, and
        // the emit must NOT decline "match result type has no machine representation". Diverges iff ALL
        // arm bodies diverge (a scalar/list match's `arms`, a sum match's decision-tree leaves). An empty
        // `arms` (a zero-arm match on a `Never` scrutinee) IS diverging — the scrutinee itself never
        // returns; `Iterator::all` on an empty slice is `true`, which is correct here.
        Core::Match { arms, .. } => {
            let bodies: Vec<StructId> = arms.iter().map(|a| a.body).collect();
            bodies.into_iter().all(|b| body_diverges(db, b))
        }
        Core::MatchList { arms, .. } => {
            let bodies: Vec<StructId> = arms.iter().map(|a| a.body).collect();
            bodies.into_iter().all(|b| body_diverges(db, b))
        }
        Core::MatchSum { root, .. } => sumcont_diverges(db, &root),
        _ => false,
    }
}

/// Whether a sum-match decision-tree continuation PROVABLY diverges on every path — the [`SumCont`]
/// companion of [`body_diverges`] for `Core::MatchSum`. A continuation diverges iff EVERY body it can
/// reach diverges: a `Leaf` iff its body does; a `Guarded`/`LitTest` iff BOTH the taken branch (`body`/
/// `then_`) AND the fall-through (`els`) diverge (either may be selected at run time); a `Switch` iff
/// every arm's continuation diverges (each disc, plus the default). Used so an all-diverge sum match
/// (`(match s ((Some _) (trap …)) ((None) (trap …)))`) emits an empty block + trailing `unreachable`
/// rather than declining "sum match result type has no machine representation".
fn sumcont_diverges(db: &mut Db, cont: &crate::core::SumCont) -> bool {
    use crate::core::SumCont;
    match cont {
        SumCont::Leaf(body) => body_diverges(db, *body),
        SumCont::Guarded { body, els, .. } => body_diverges(db, *body) && sumcont_diverges(db, els),
        SumCont::LitTest { then_, els, .. } => {
            sumcont_diverges(db, then_) && sumcont_diverges(db, els)
        }
        SumCont::Switch { arms, .. } => arms.iter().all(|a| sumcont_diverges(db, &a.cont)),
    }
}

/// The flow-refinement frame a branch of `cond` GUARANTEES — a per-variable `[lo, hi]` interval map. A
/// branch condition that compares a variable to a constant narrows that variable's range in the taken
/// branch (and its negation in the other); `and`/`or`/`not` compose the atoms. A shape this does not
/// model contributes NOTHING (returns `base`) — conservative, so no guard is ever wrongly elided. Used
/// by both backends to elide a guard the narrowed range proves dead (a range-guaranteed `+ 1` cannot
/// overflow, etc.). `base` is the enclosing frame this branch refines further.
///
///   • a comparison `(op var C)` / `(op C var)` → the one-sided interval [`refine_from_comparison`];
///   • `(and a b)` in its holding polarity, `(or a b)` in its failing polarity → refine by each operand;
///     the other polarity is a disjunction (no single-variable interval), skip;
///   • `(not a)` → refine `a` with the opposite polarity.
/// Nested `if`s accumulate (each frame merges the parent's). A SIGNED ordering refines directly; an
/// UNSIGNED ordering refines against a NON-NEGATIVE constant (no wraparound — see `refine_from_comparison`);
/// `Eq`'s then-branch pins `[c,c]` and its else yields no single interval. The refinement is a narrowing
/// the branch GUARANTEES, so a guard the narrowed range proves dead is safe to drop.
pub(crate) fn refined_frame_for_branch(
    db: &mut Db,
    cond: StructId,
    then_branch: bool,
    base: crate::fxhash::FxHashMap<StructId, crate::db::ValueFact>,
) -> crate::fxhash::FxHashMap<StructId, crate::db::ValueFact> {
    match core_of(db, cond) {
        Core::Compare { op, lhs, rhs } => {
            refine_from_comparison(db, op, lhs, rhs, then_branch, base)
        }
        // `(and a b)` holds in the THEN branch iff BOTH hold — apply each. `(or a b)` fails in the ELSE
        // branch iff BOTH fail — apply each operand NEGATED (pass `then_branch=false` down). The other
        // polarity (an `and`'s else, an `or`'s then) is a disjunction of the operands' negations/holds,
        // which does not yield a single-variable interval — skip it (returns `base` unchanged).
        Core::And { lhs, rhs, is_and } => {
            let apply_both = (is_and && then_branch) || (!is_and && !then_branch);
            if !apply_both {
                return base;
            }
            // Each operand is itself a condition establishing its own bound in this branch's polarity: an
            // `and`'s THEN wants both operands HELD (then_branch=true), an `or`'s ELSE wants both operands
            // FAILED (then_branch=false). Recurse so a nested `(and …)`/comparison in either operand is
            // handled uniformly.
            let after_lhs = refined_frame_for_branch(db, lhs, is_and, base);
            refined_frame_for_branch(db, rhs, is_and, after_lhs)
        }
        // `(not a)` in this branch's polarity = `a` in the OPPOSITE polarity.
        Core::Not { operand } => refined_frame_for_branch(db, operand, !then_branch, base),
        _ => base,
    }
}

/// Merge into `base` the one-sided bound a single var-vs-const comparison `(op lhs rhs)` guarantees in the
/// given branch polarity — the atom [`refined_frame_for_branch`] composes for `and`/`or`/`not`.
/// `(op var C)` / `(op C var)` (flipped) → in the `then` branch `var op C` holds, in the `else` its
/// negation; the resulting `[lo, hi]` bound is intersected with any existing refinement for `var`. Refines
/// a SIGNED integer var, or an UNSIGNED one against a NON-NEGATIVE constant (an unsigned ordering vs `c ≥ 0`
/// has no wraparound — `x < 8` ⇒ `x ∈ [0, 7]`). A non-comparison, a non-integer var, or an unsigned var vs
/// a NEGATIVE constant (degenerate — the domain edge decides it) contributes nothing (returns `base`).
pub(crate) fn refine_from_comparison(
    db: &mut Db,
    op: Prim,
    lhs: StructId,
    rhs: StructId,
    then_branch: bool,
    base: crate::fxhash::FxHashMap<StructId, ValueFact>,
) -> crate::fxhash::FxHashMap<StructId, ValueFact> {
    let binder_of = |db: &mut Db, id: StructId| -> Option<StructId> {
        match core_of(db, id) {
            Core::Param { binder } | Core::LocalRef { binder } => Some(binder),
            _ => None,
        }
    };
    let const_of = |db: &mut Db, id: StructId| -> Option<i64> {
        match core_of(db, id) {
            Core::ConstInt(v) => v.to_i64(),
            _ => None,
        }
    };
    // The COLLECTION binder behind a `(List.len coll)` occurrence, if `id` is exactly that op applied to a
    // simple collection reference. `resolved_of` is NOT needed here: `ListLen`'s operand is a plain
    // collection occurrence (a param/kept-local ref), so `core_of` on it never inlines/renumbers a payload
    // out of a dup-site the way a match SCRUTINEE's `core_of` did in slice-5 — the operand is a leaf ref.
    let list_len_coll = |db: &mut Db, id: StructId| -> Option<StructId> {
        match core_of(db, id) {
            Core::ListLen { operand } => match core_of(db, operand) {
                Core::Param { binder } | Core::LocalRef { binder } => Some(binder),
                _ => None,
            },
            _ => None,
        }
    };
    // BOUNDED-INDEX (below-len) ESTABLISHER (operator-greenlit bounds facet): recognize `(< i (List.len xs))`
    // and its flipped/negated equivalents, and — in the branch that GUARANTEES `i < len(xs)` — record the
    // relational fact `below_len[i] = xs`. A `List.at xs i` under this fact sheds its own `index < len`
    // check (the guard already proved it). ONLY a STRICT `<` (not `<=`) proves `i < len` — `i <= len` allows
    // `i == len`, an OOB access, so it establishes nothing. Detect the index binder on one side and a
    // `List.len` on the other; normalize to `i CMP len`; negate CMP for the else branch; establish iff the
    // effective relation is strict-less-than. This runs BEFORE the const path (a `List.len` operand is not a
    // `ConstInt`, so the const path would just `return base` and lose the fact); a non-ListLen comparison
    // falls through to it unchanged.
    let idx_len = if let (Some(i), Some(xs)) = (binder_of(db, lhs), list_len_coll(db, rhs)) {
        Some((i, xs, op)) // `(op i (List.len xs))` — i already on the left
    } else if let (Some(xs), Some(i)) = (list_len_coll(db, lhs), binder_of(db, rhs)) {
        // `(op (List.len xs) i)` — flip so the index is on the left (`len op i` ⇒ `i op.flip len`).
        let flipped = match op {
            Prim::Lt => Prim::Gt,
            Prim::Gt => Prim::Lt,
            Prim::Le => Prim::Ge,
            Prim::Ge => Prim::Le,
            other => other,
        };
        Some((i, xs, flipped))
    } else {
        None
    };
    if let Some((i, xs, cmp)) = idx_len {
        // The relation the TAKEN branch establishes: the else branch negates it.
        let effective = if then_branch {
            cmp
        } else {
            match cmp {
                Prim::Lt => Prim::Ge,
                Prim::Le => Prim::Gt,
                Prim::Gt => Prim::Le,
                Prim::Ge => Prim::Lt,
                other => other,
            }
        };
        // Only STRICT `i < len(xs)` licenses the bounds elision. `i <= len` would admit `i == len` (OOB).
        if matches!(effective, Prim::Lt) {
            let mut frame = base;
            // Compose with any existing interval fact for `i` (keep its `int_range`, add `below_len`) so the
            // two facets coexist on the same binder — the consumer reads whichever it needs.
            let entry = frame.entry(i).or_default();
            entry.below_len = Some(xs);
            return frame;
        }
        // A non-strict / non-establishing ListLen comparison contributes no fact (the runtime check stays).
        return base;
    }
    // `(var, op-with-var-on-left, const)`. `(C op var)` flips to `var op.flip C`.
    let (var, cmp, c) = if let (Some(v), Some(k)) = (binder_of(db, lhs), const_of(db, rhs)) {
        (v, op, k)
    } else if let (Some(k), Some(v)) = (const_of(db, lhs), binder_of(db, rhs)) {
        let flipped = match op {
            Prim::Lt => Prim::Gt,
            Prim::Gt => Prim::Lt,
            Prim::Le => Prim::Ge,
            Prim::Ge => Prim::Le,
            other => other,
        };
        (v, flipped, k)
    } else {
        return base;
    };
    // EQUALITY guard: `(if (= x c) THEN ELSE)` — in the THEN branch `x == c`, so pin `x` to the EXACT
    // range `[c, c]` (the `if`-guard analogue of a match arm's exact-value refinement). This lets the body
    // fold a guard/comparison on `x` (`(if (= x 5) (+ x 1) …)` — the `+ 1` under `x == 5` cannot overflow;
    // a range-comparison on `x` decides). The ELSE branch gives only `x != c` — no interval — so skip it.
    // Sound for BOTH signednesses: equality does not depend on the order's wraparound. Intersects with any
    // existing frame bound for `x` (a `[c,c]` is the tightest, so it wins).
    if matches!(cmp, Prim::Eq) {
        if !then_branch {
            return base; // `x != c` — no single interval
        }
        // `c` came from `to_i64()`, so it is already an i64 — no clamp needed.
        let ec = c;
        let mut frame = base;
        // Pin `x` to the exact `[c, c]` — but only when `c` lies WITHIN any prior frame bound for `x`. If it
        // does not, the guard is unsatisfiable (a contradiction the branch never reaches), so leave the
        // prior frame rather than fabricate an inverted `[c,c]` a downstream consumer might misread.
        let (plo, phi) = frame
            .get(&var)
            .and_then(|f| f.int_range)
            .unwrap_or((i64::MIN, Some(i64::MAX)));
        if plo <= ec && phi.is_none_or(|h| ec <= h) {
            frame.insert(var, ValueFact::from_int_range(ec, Some(ec)));
        }
        return frame;
    }
    // An integer variable — signed OR unsigned. A SIGNED ordering refines directly. An UNSIGNED ordering
    // is sound too, but ONLY against a NON-NEGATIVE constant: `x < c` / `x >= c` for `c ≥ 0` narrow `x` to
    // a sub-interval of `[0, 2^w)` with NO wraparound (both `x` and `c` are ≥ 0, so the ordering is the
    // ordinary one) — `x < 8` ⇒ `x ∈ [0, 7]`, eliding a downstream mask/bounds guard exactly as the signed
    // case elides an overflow guard. A NEGATIVE constant against an unsigned var is degenerate (`x < -1` is
    // always false, `x > -1` always true for `x ≥ 0`) — no useful sub-interval, and those verdicts are
    // decided by `fold_comparison_at_type_bound` at the type edge, not here — so skip it (return `base`),
    // never fabricating a refinement from a comparison the domain already settles. The clamp/intersect math
    // below is signedness-neutral once past this guard (it operates on the i64 values the interval carries).
    let Ty::Int(it) = type_of(db, var) else {
        return base; // only an INTEGER variable has an interval refinement
    };
    let signed = it.ground_signed();
    if !signed && c < 0 {
        return base; // unsigned var vs a negative const — degenerate, no sub-interval
    }
    // The bound the taken branch establishes; the `else` branch negates the op.
    let effective = if then_branch {
        cmp
    } else {
        match cmp {
            Prim::Lt => Prim::Ge,
            Prim::Le => Prim::Gt,
            Prim::Gt => Prim::Le,
            Prim::Ge => Prim::Lt,
            other => other,
        }
    };
    let c = c as i128;
    let clamp = |x: i128| -> Option<i64> {
        if x > i64::MAX as i128 || x < i64::MIN as i128 {
            None
        } else {
            Some(x as i64)
        }
    };
    let (new_lo, new_hi): (Option<i64>, Option<i64>) = match effective {
        Prim::Lt => (None, clamp(c - 1)),
        Prim::Le => (None, clamp(c)),
        Prim::Gt => (clamp(c + 1), None),
        Prim::Ge => (clamp(c), None),
        _ => return base, // Eq/Ne/compare — no interval bound
    };
    // The seed interval when `var` has no prior frame bound is its DECLARED-TYPE bounds — NOT a hardcoded
    // `(i64::MIN, i64::MAX)`. For an UNSIGNED-64 var the real upper bound is `2^64 − 1`, which is NOT
    // i64-representable, so its type `hi` is `None` (unbounded above in this i64 domain). Seeding the
    // fabricated `Some(i64::MAX)` there would be UNSOUND: a lower-bound refinement (`x > 8`) would leave
    // `hi = Some(i64::MAX)`, wrongly excluding `[i64::MAX+1, 2^64−1]` — values a UInt64 legitimately holds —
    // so a downstream `(> x i64::MAX)` would fold to false and MISCOMPILE. Seeding the type's own `(0, None)`
    // keeps the lower-bound refinement's `hi` unbounded (sound). `lo`'s `None` (no i64-min, unreachable for a
    // real integer type) falls back to `i64::MIN`. `value_range` still re-intersects with the type bounds.
    let (seed_lo, seed_hi) = crate::lower::resolved_int_bounds(it)
        .map(|(lo, hi)| (lo.unwrap_or(i64::MIN), hi))
        .unwrap_or((i64::MIN, Some(i64::MAX)));
    let mut frame = base;
    let (mut lo, mut hi) = frame
        .get(&var)
        .and_then(|f| f.int_range)
        .unwrap_or((seed_lo, seed_hi));
    if let Some(nl) = new_lo {
        lo = lo.max(nl);
    }
    if let Some(nh) = new_hi {
        hi = Some(match hi {
            Some(h) => h.min(nh),
            None => nh,
        });
    }
    frame.insert(var, ValueFact::from_int_range(lo, hi));
    frame
}
