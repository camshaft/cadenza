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
use crate::ast::{IntValue, StructId};
use crate::core::Core;
use crate::db::Db;
use crate::diag::{Code, Reject};
use crate::resolve::resolved_of;
use crate::resolved::{Prim, Resolved};
use tracing::trace;

/// The core (A-normal) form of the node at `id`, filling the column on demand (memoized). Reads the
/// resolved form; children stay ids, lowered on their own demand.
pub fn core_of(db: &mut Db, id: StructId) -> Core {
    if let Slot::Filled(c) = db.core.get(id) {
        trace!(target: "rcdzc::lower", node = id.0, "memo hit");
        return c.clone();
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

/// Lower one node's resolved form to its core form. Records fold: a bare name is its bound value's
/// core, a `let` is its body's core, and a member projection is the FIELD'S core read directly — so a
/// record used only to read a field leaves no runtime trace (it folds to the projected scalar). A
/// record used as a runtime value survives as `Core::Record` (which declines at select until the
/// value heap exists). This is the one compile-time reduction tier acting through lowering
/// (`reference-compiler.md` §A Construct Whose Value Is Fully Determined At Compile Time).
fn compute(db: &mut Db, id: StructId) -> Core {
    match resolved_of(db, id) {
        Resolved::Int(v) => Core::ConstInt(v),
        Resolved::Bool(b) => Core::ConstBool(b),
        Resolved::Unit => Core::Unit,
        // A name is its bound value's core. If that value is a KEPT `let` binding (a multi-use runtime
        // computation the enclosing `let` named once — see `lower_let`), this reference reads the
        // shared slot: `Core::LocalRef`. Otherwise the binding was copy-propagated / erased, so the
        // name IS its value's core — follow the ref (the ordinary case; a single-use or constant
        // binding leaves no `Let`).
        Resolved::Ref { value } => {
            if db.kept_bindings.contains(&value) {
                trace!(target: "rcdzc::lower", node = id.0, binder = value.0, "ref → local (kept multi-use binding)");
                Core::LocalRef { binder: value }
            } else {
                core_of(db, value)
            }
        }
        // A type annotation ERASES to its expression's core — `(: e T)` runs exactly as `e` (the
        // annotation's force is entirely on inference; it has no runtime trace).
        Resolved::Annot { expr, .. } => core_of(db, expr),
        // A `let` — A-NORMALIZE its bindings: a binding whose value is a runtime computation used more
        // than once is NAMED (a `Core::Let` binding, computed once, read by `LocalRef`); a single-use
        // or constant binding is copy-propagated / erased (its references follow through to its value).
        // So naming adds no cost and the emitted bytes are unchanged for a program with no multi-use
        // runtime binding (`reference-compiler.md` §The Core Representation Is In A-Normal Form).
        Resolved::Let { bindings, body } => lower_let(db, &bindings, body),
        // A record value — kept as a compound; folds away only when a member reads a field of it.
        // (Materialize the shared `Arc` map into the `Core::Record` owned map — a single O(fields)
        // copy per record NODE, not per access, so it does not reintroduce the O(N²) the Arc removed.)
        Resolved::Record { fields } => Core::Record {
            fields: (*fields).clone(),
        },
        // Member access FOLDS: reduce the operand to a record (following refs, reducing a ctor
        // application) and lower the field's value directly, so `(. (record (x 1)) x)` and `(. (Int
        // 64) max)` both fold to the field's value with no record built. The one projection, via the
        // evaluator. A non-record operand or an absent field is a poison so a mis-projection never
        // emits a wrong value.
        Resolved::Member { operand, key } => match crate::eval::member_value(db, operand, &key) {
            crate::eval::Member::Field(value) => core_of(db, value),
            crate::eval::Member::NoField => Core::Poison(Reject::coded(
                Code::Malformed,
                format!("record has no field `{}`", key.name),
            )),
            // The operand did not reduce to a compile-time-visible record. If it is a RUNTIME record (a
            // call result, an `if` selection) carrying the field, read it off the heap array: a record
            // at run time IS a positional array in sorted-key order, so the field read is a `Core::Proj`
            // at the field's sorted index — the SAME `arr-get` a tuple projection uses. Otherwise it is
            // a genuine non-record (or a poison operand, whose own root cause propagates).
            crate::eval::Member::NotRecord => {
                match crate::eval::runtime_member_index(db, operand, &key) {
                    Some(index) => {
                        trace!(target: "rcdzc::lower", node = id.0, operand = operand.0, key = %key.name, index, "member access on a runtime record → arr-get at the field's sorted index");
                        Core::Proj { operand, index }
                    }
                    None => match core_of(db, operand) {
                        Core::Poison(r) => Core::Poison(r),
                        _ => Core::Poison(Reject::coded(
                            Code::Malformed,
                            "member access requires a record",
                        )),
                    },
                }
            }
        },
        // A tuple literal — kept as a compound. Like a record, it folds away only when a projection
        // reads a visible element of it; a tuple that survives (constructed from runtime operands, or a
        // constant tuple that escapes) is a `Core::Tuple` the backend builds on the heap.
        Resolved::Tuple { elems } => Core::Tuple {
            elems: elems.to_vec(),
        },
        // A tuple PROJECTION `(. t N)`. FOLD when the operand reduces to a compile-time-visible tuple:
        // lower the element's core directly (no heap, like a record member fold). Otherwise the operand
        // is a RUNTIME tuple (a parameter, a kept `let` binding) — emit a `Core::Proj` the backend lowers
        // to `arr-get`. An out-of-arity index is impossible here (rejected in `type_errors` before
        // selection); defensively, a projection past a visible tuple's arity poisons.
        Resolved::Proj { operand, index } => {
            match crate::eval::reduce_to_tuple_elems(db, operand) {
                Some(elems) => match elems.get(index) {
                    Some(&elem) => {
                        trace!(target: "rcdzc::fold", node = id.0, index, "tuple projection folds to a visible element");
                        core_of(db, elem)
                    }
                    None => Core::Poison(Reject::coded(
                        Code::Malformed,
                        format!("tuple index {index} is out of range"),
                    )),
                },
                None => {
                    trace!(target: "rcdzc::lower", node = id.0, operand = operand.0, index, "tuple projection stays runtime (operand is a runtime tuple)");
                    Core::Proj { operand, index }
                }
            }
        }
        Resolved::If { cond, then_, else_ } => Core::If { cond, then_, else_ },
        // A match over a scalar scrutinee — FOLD when the scrutinee is a constant (select the arm whose
        // probe it satisfies), else emit a `Core::Match` the backend lowers to a probe chain.
        Resolved::Match { scrutinee, arms } => lower_match(db, scrutinee, &arms),
        // A bare built-in operation value that is not applied has no runtime form yet (no closures) —
        // it declines. Applying it is what lowers.
        Resolved::Prim(_) => Core::Poison(Reject::decline(
            "a built-in operation used as a value needs runtime closures (not yet built)",
        )),
        // Application — the ONE path, dispatched by the head value's `(meta apply)` primitive. An
        // arithmetic prim folds (below); a type-constructor prim reduces via the evaluator to a built
        // value (a module / type-value), which is then lowered — a member projection off it folds, a
        // bare type/module used at runtime declines at the erasure fence.
        Resolved::Apply { head, args } => {
            // A LAMBDA head β-reduces (substitute args for params) and the reduced body lowers — this
            // is how a user function call folds/monomorphizes: `((fn (x) (+ x 1)) 5)` reduces to
            // `(+ 5 1)` → `6`, with no function value emitted. The reduction runs UNDER a guard keyed
            // by the lambda's body, so a recursive call (which re-enters the same body while lowering
            // the reduced result) is detected and DECLINES rather than inlining without end.
            if crate::eval::lambda_body(db, head).is_some() {
                trace!(target: "rcdzc::lower", node = id.0, head = head.0, "apply: β-reduce lambda head and lower the result");
                // Reduce and lower under a depth guard: a terminating fold bottoms out; a recursive
                // function inlines past the bound and DECLINES rather than diverging.
                match db.enter_reduction() {
                    Some(mut guard) => {
                        let g = guard.db();
                        return match crate::eval::apply_lambda(g, head, &args) {
                            Ok(Some(reduced)) => core_of(g, reduced),
                            Ok(None) => unreachable!("lambda_body implies a lambda head"),
                            // The reduction declined. If it declined because the callee is RECURSIVE
                            // (can't inline to a normal form), emit a real `Core::Call` to it instead —
                            // provided the callee is a top-level def whose signature is DETERMINED
                            // (`def_scheme` — an annotated recursive def types by absorption, no fixpoint
                            // needed). An unannotated/undetermined callee still declines (its signature
                            // needs the connected solve, a later step). Any other decline propagates.
                            Err(msg) => lower_recursive_call_or_decline(g, head, &args, msg),
                        };
                    }
                    None => {
                        trace!(target: "rcdzc::lower", node = id.0, "apply: reduction depth limit hit → decline (recursive)");
                        return Core::Poison(Reject::decline(
                            "a recursive function needs runtime specialization (not yet built)",
                        ));
                    }
                }
            }
            // A ZERO-ARGUMENT application `(g)` whose head is not a lambda. Applying a value to no
            // arguments is the identity — the application IS the head value. This is how a NULLARY def
            // is called: `(def (g) 7)` resolves `g` to its body value (so a bare `g` is 7), and `(g)`
            // is that same value. (A nullary LAMBDA `((fn () 7))` took the β-reduce branch above, so it
            // is already handled; only a non-lambda head reaches here.) Without this, `(g)` fell through
            // to `meta_apply_of` — which, finding no `(meta apply)` on the scalar 7, rejected it as
            // "value is not applyable", breaking every nullary-function call.
            if args.is_empty() {
                // A RECURSIVE nullary call (`(def (f) (f))`) cannot fold to a normal form — following
                // the head would re-enter the same body without end. Decline it exactly as a recursive
                // parameterized call declines (a nullary def has no runtime-function form yet, so there
                // is no `Core::Call` to emit — it declines rather than diverging). `is_recursive` reads
                // the callee body reached through the nullary def's `Ref` (see `eval::callee_body`).
                if let Some(body) = crate::eval::lambda_body_of_nullary(db, head)
                    && crate::eval::is_recursive(db, body)
                {
                    trace!(target: "rcdzc::lower", node = id.0, head = head.0, "apply: recursive nullary call → decline");
                    return Core::Poison(Reject::decline(
                        "a recursive function needs runtime specialization (not yet built)",
                    ));
                }
                trace!(target: "rcdzc::lower", node = id.0, head = head.0, "apply: zero-argument application is its head value");
                return core_of(db, head);
            }
            match crate::eval::meta_apply_of(db, head) {
                Some(prim) if prim.is_arith() => {
                    trace!(target: "rcdzc::lower", node = id.0, ?prim, "apply: arithmetic prim");
                    lower_arith(db, prim, &args)
                }
                Some(prim) if prim.is_comparison() => {
                    trace!(target: "rcdzc::lower", node = id.0, ?prim, "apply: comparison prim");
                    lower_comparison(db, prim, &args)
                }
                Some(prim) if prim.is_conversion() => {
                    trace!(target: "rcdzc::lower", node = id.0, ?prim, "apply: conversion prim");
                    lower_conversion(db, id, prim, &args)
                }
                Some(prim) => {
                    trace!(target: "rcdzc::lower", node = id.0, ?prim, "apply: type-constructor prim");
                    match crate::eval::reduce_ctor(db, prim, &args) {
                        Ok(built) => core_of(db, built),
                        Err(msg) => {
                            trace!(target: "rcdzc::lower", node = id.0, %msg, "apply: constructor declined");
                            Core::Poison(Reject::decline(msg))
                        }
                    }
                }
                // Not applyable. If the head itself is a poison (e.g. an unbound name), propagate THAT
                // root cause — an unbound head is a scope error, not merely "not applyable".
                None => match core_of(db, head) {
                    Core::Poison(r) => Core::Poison(r),
                    _ => {
                        trace!(target: "rcdzc::lower", node = id.0, head = head.0, "apply: head is not applyable (decline)");
                        Core::Poison(Reject::decline("value is not applyable"))
                    }
                },
            }
        }
        Resolved::Poison(r) => Core::Poison(r),
        // A parameter reference is a RUNTIME value — its value is unknown at compile time, so it lowers
        // to a `Core::Param` the backend reads as a `local.get` of the parameter's slot. (A parameter
        // only reaches lowering when its function body is emitted STANDALONE — an exported function; at
        // a constant call site the param is substituted by the fold and never lowered as a param.)
        Resolved::Param { binder } => Core::Param { binder },
        // A type value or a compile-time lambda is compile-time-only — no runtime core form (the
        // erasure fence forbids one reaching runtime), so lowering it as a runtime value declines.
        Resolved::TypeVal(_) | Resolved::Lambda { .. } => Core::Poison(Reject::decline(
            "a type value or compile-time lambda has no runtime form",
        )),
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
fn lower_let(db: &mut Db, bindings: &[(StructId, StructId)], body: StructId) -> Core {
    // The `(binder-name-occ, init-occ)` pairs; a reference resolves to the INIT occurrence, so that is
    // what the body's `Ref`s point at and what a kept binding is keyed by.
    let mut kept: Vec<(StructId, StructId)> = Vec::new();
    for (k, &(_name_occ, init)) in bindings.iter().enumerate() {
        // A binding's SCOPE (the positions its name is visible in) is the LATER sibling initializers
        // plus the body — `let*` sequential scoping. Count uses across that whole continuation so a
        // binding referenced only by a later initializer is still named.
        let mut continuation: Vec<StructId> = bindings[k + 1..].iter().map(|&(_, v)| v).collect();
        continuation.push(body);
        if should_keep_binding(db, init, &continuation) {
            // Record the keep BEFORE lowering the body/later inits — their references to this init read
            // `db.kept_bindings` to decide `LocalRef` vs follow-through.
            db.kept_bindings.insert(init);
            kept.push((init, init));
        }
    }
    // The body's core (its references to kept bindings now lower to `LocalRef`).
    if kept.is_empty() {
        // Nothing named — the ordinary erase: the `let`'s value is its body's value.
        return core_of(db, body);
    }
    trace!(target: "rcdzc::lower", body = body.0, kept = kept.len(), "let: A-normalized (named multi-use runtime bindings)");
    Core::Let {
        bindings: kept,
        body,
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
/// A wildcard/binder tail covers the rest, and for an OPEN type (an integer) it is the only cover — no
/// finite literal set exhausts the integers. A FINITE type is exhausted by its literals instead: a Bool
/// scrutinee covered by both a `true` arm and a `false` arm needs no wildcard. A match that covers
/// neither way is rejected (CDZ0210), not compiled to a fallthrough with no defined value.
fn lower_match(db: &mut Db, scrutinee: StructId, arms: &[(StructId, StructId)]) -> Core {
    // Classify each arm's pattern into a probe + keep its body. A pattern that is not a scalar literal,
    // binder, or wildcard declines the whole match (a compound pattern needs a heap walk).
    let mut probes: Vec<(crate::core::Probe, StructId)> = Vec::new();
    for &(pat, body) in arms {
        match classify_probe(db, pat) {
            Some(p) => probes.push((p, body)),
            None => {
                return Core::Poison(Reject::decline(
                    "a match pattern that is not a scalar literal or `_` is not yet supported",
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
    for (probe, _) in &probes {
        let pat_ty = match probe {
            crate::core::Probe::Int(_) => Some(crate::ty::Ty::int()),
            crate::core::Probe::Bool(_) => Some(crate::ty::Ty::Bool),
            crate::core::Probe::Wild => None,
        };
        if let Some(pt) = pat_ty
            && !pt.agrees_with(&scrut_ty)
        {
            return Core::Poison(Reject::coded(
                Code::Malformed,
                format!(
                    "match pattern type {} does not match scrutinee type {}",
                    pt.render_name(),
                    scrut_ty.render_name()
                ),
            ));
        }
    }
    //  (2) exhaustiveness: a scalar match must cover every value of the scrutinee's type. A wildcard
    //      tail covers the rest, and for an OPEN type (Int64) that is the ONLY way — no finite literal
    //      set exhausts the integers. But a FINITE type is exhausted by its literals: a Bool scrutinee
    //      covered by BOTH a `true` arm and a `false` arm needs no wildcard (the two values are the
    //      whole type — `core-semantics.md` §Matching Is Exhaustive Or Rejected). This holds EVEN when
    //      the scrutinee is a constant that hits an arm — well-formedness is independent of the value.
    let has_wild = probes
        .iter()
        .any(|(p, _)| matches!(p, crate::core::Probe::Wild));
    // A Bool scrutinee's two literals exhaust it. (A definitely-Bool or still-open `Any` scrutinee whose
    // arms are Bool literals — a bare parameter matched with `true`/`false` — is matching over Bool; a
    // definitely-Int scrutinee with a Bool probe already faulted in step (1) and never reaches here.)
    let bool_exhaustive = scrut_ty.agrees_with(&crate::ty::Ty::Bool)
        && probes
            .iter()
            .any(|(p, _)| matches!(p, crate::core::Probe::Bool(true)))
        && probes
            .iter()
            .any(|(p, _)| matches!(p, crate::core::Probe::Bool(false)));
    if !has_wild && !bool_exhaustive {
        return Core::Poison(Reject::coded(
            Code::NonExhaustive,
            "a scalar match must end in a wildcard `_` arm (non-exhaustive)",
        ));
    }

    // Well-formed. FOLD if the scrutinee is a compile-time constant: select the first arm whose probe
    // it satisfies and lower THAT body (no runtime match, like the const `if` fold).
    match core_of(db, scrutinee) {
        Core::ConstInt(v) => {
            for (probe, body) in &probes {
                if probe_matches_int(probe, &v) {
                    trace!(target: "rcdzc::fold", "match folds to a selected arm (constant Int scrutinee)");
                    return core_of(db, *body);
                }
            }
            // Unreachable: a wildcard is present (checked above), so some arm always matches.
            return Core::Poison(Reject::decline(
                "match: no arm matched a constant (unreachable)",
            ));
        }
        Core::ConstBool(b) => {
            for (probe, body) in &probes {
                if probe_matches_bool(probe, b) {
                    trace!(target: "rcdzc::fold", "match folds to a selected arm (constant Bool scrutinee)");
                    return core_of(db, *body);
                }
            }
            return Core::Poison(Reject::decline(
                "match: no arm matched a constant (unreachable)",
            ));
        }
        Core::Poison(r) => return Core::Poison(r),
        _ => {}
    }
    // Runtime scalar scrutinee — it must BE a scalar (a compound needs a heap walk, later).
    if !is_scalar(db, scrutinee) {
        return Core::Poison(Reject::decline(
            "matching a compound value needs a heap walk (not yet built)",
        ));
    }
    trace!(target: "rcdzc::lower", scrutinee = scrutinee.0, arms = probes.len(), "match stays runtime (scalar scrutinee → probe chain)");
    Core::Match {
        scrutinee,
        arms: probes,
    }
}

/// Classify a match PATTERN occurrence into a [`Probe`], or `None` if it is not a Stage-3 scalar
/// pattern. An integer/boolean literal is a literal probe; a bare NAME (the wildcard `_`, or a BINDER
/// like `k`) always matches — a `Wild` probe. A binder differs from `_` only in scope: a reference to
/// it in the arm body resolves to the scrutinee (handled by `resolve`'s Case 5), so the PROBE is
/// identical (always matches, exhaustive tail). (A constructor / tuple / record pattern is a later
/// increment — it returns `None` here; with no sums yet, every bare name in a scalar match is a binder.)
fn classify_probe(db: &mut Db, pat: StructId) -> Option<crate::core::Probe> {
    // A bare name — the wildcard `_` OR a binder — always matches. Detected structurally (before
    // resolving, which would look the name up / poison it); the binding is a scope concern, not a probe.
    if db.ast.as_name(pat).is_some() {
        return Some(crate::core::Probe::Wild);
    }
    match resolved_of(db, pat) {
        Resolved::Int(v) => Some(crate::core::Probe::Int(v)),
        Resolved::Bool(b) => Some(crate::core::Probe::Bool(b)),
        _ => None,
    }
}

/// Whether a probe matches a constant integer scrutinee (for the fold). A `Wild` matches anything. The
/// literal comparison is BY VALUE (`eq_value`) — a folded `0` (empty magnitude) and a literal `0`
/// (`[0]`) denote the same integer, so struct `==` would wrongly miss (the parity-dispatch bug).
fn probe_matches_int(probe: &crate::core::Probe, v: &IntValue) -> bool {
    match probe {
        crate::core::Probe::Int(p) => p.eq_value(v),
        crate::core::Probe::Wild => true,
        crate::core::Probe::Bool(_) => false,
    }
}

/// Whether a probe matches a constant boolean scrutinee (for the fold). A `Wild` matches anything.
fn probe_matches_bool(probe: &crate::core::Probe, b: bool) -> bool {
    match probe {
        crate::core::Probe::Bool(p) => *p == b,
        crate::core::Probe::Wild => true,
        crate::core::Probe::Int(_) => false,
    }
}

/// A lambda application whose β-reduction DECLINED with `msg`: emit a runtime `Core::Call` if it
/// declined because the callee is a RECURSIVE top-level def with a DETERMINED signature; otherwise
/// propagate the decline. This is the ONE place a recursive call becomes a real wasm call instead of an
/// unbounded inline. A non-recursive decline (a partial application, a bad head) is NOT a call — its
/// message is passed through unchanged.
fn lower_recursive_call_or_decline(
    db: &mut Db,
    head: StructId,
    args: &[StructId],
    msg: String,
) -> Core {
    // Only a RECURSION decline becomes a call; every other decline (partial application, over-arity)
    // propagates as-is. The recursion decline is the one `apply_lambda` raises via `is_recursive`.
    let is_recursion_decline = msg.contains("recursive function needs runtime specialization");
    if !is_recursion_decline {
        return Core::Poison(Reject::decline(msg));
    }
    // Resolve the head to the top-level def it names. Only a NAMED top-level def can be emitted as a
    // standalone wasm function (its index is stable in the layout); a computed/anonymous recursive head
    // has no such identity, so it still declines.
    let callee = match callee_def_index(db, head) {
        Some(d) => d,
        None => return Core::Poison(Reject::decline(msg)),
    };
    // The callee must have a DETERMINED signature to be emitted (its params need machine valtypes). An
    // annotated recursive def qualifies (types by absorption); an unannotated one is solved by the
    // connected parameter solve (`solve_recursive_params`, A2) — it stays undetermined only when no use
    // in the body constrains a parameter (it grounds to `Any`), in which case the call still declines.
    if crate::infer::def_scheme(db, callee).is_none() {
        trace!(target: "rcdzc::lower", head = head.0, callee, "recursive call: callee signature undetermined → decline (A2)");
        return Core::Poison(Reject::decline(
            "a recursive function with an unannotated parameter is not yet inferred (annotate its parameters)",
        ));
    }
    trace!(target: "rcdzc::lower", head = head.0, callee, args = args.len(), "recursive call → Core::Call");
    Core::Call {
        callee,
        args: args.to_vec(),
    }
}

/// The `db.defs` index of the top-level def an application head names, if any — following a `Ref` to a
/// `Lambda` whose body matches a def's body occurrence. Returns `None` for a head that is not a named
/// top-level def (a `let`-bound lambda, a computed head).
fn callee_def_index(db: &mut Db, head: StructId) -> Option<usize> {
    // The head resolves to a `Lambda { body, .. }` for a top-level def (resolve maps a def name to its
    // lambda). Match that body occurrence back to the def index.
    let body = match resolved_of(db, head) {
        Resolved::Lambda { body, .. } => body,
        Resolved::Ref { value } => return callee_def_index(db, value),
        _ => return None,
    };
    db.def_index_by_body(body)
}

/// Whether a `let` binding whose initializer is `init` should be KEPT as a named `Core::Let` binding
/// rather than copy-propagated. Kept iff (1) its value is a RUNTIME computation — its core is not a
/// constant/atom that folds away, so following it through would re-emit the computation — AND (2) its
/// name is used MORE THAN ONCE across `scope` (the later sibling initializers and the body — naming
/// pays for itself only when it avoids a recompute). A constant, a single-use binding, or a poison is
/// propagated (byte-neutral).
fn should_keep_binding(db: &mut Db, init: StructId, scope: &[StructId]) -> bool {
    // A value that folds to a constant / atom leaves no computation to share — always propagate.
    if !is_runtime_computation(db, init) {
        return false;
    }
    // Count references to this binding across its scope. Naming is worth it only at >= 2 uses.
    let mut n = 0;
    for &region in scope {
        n += uses_in(db, region, init);
    }
    n >= 2
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
        _ => false,
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
    let type_ast = type_ast(&mut b, &ty)?;
    let root = b.list(vec![colon, value, type_ast]);
    Some(crate::codec::encode(&b.finish(root)))
}

/// A RUNTIME leaf hole in a value-form byte template: the byte OFFSET in the template where the leaf's
/// runtime value is written, the WALK PATH of `arr-get` indices from the root heap handle to the leaf,
/// and its KIND (how many bytes / which encoding). The template bakes everything static (structure,
/// names, type nodes, kind/len framing); at run time `encode()` walks each hole's path and writes the
/// value. (`DESIGN-value-heap-rcdzc.md` §3a R2 — the runtime compound escape.)
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RuntimeLeaf {
    /// Byte offset in the template where the runtime value is written.
    pub offset: usize,
    /// `arr-get` indices from the root handle down to this leaf (empty = the root is itself the leaf).
    pub path: Vec<u32>,
    /// How the leaf's runtime value fills its hole.
    pub kind: LeafFill,
}

/// How a runtime leaf's value fills its template hole.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LeafFill {
    /// A boxed integer: read `get-int` (s64), write 8 big-endian magnitude bytes at `offset` (the
    /// template reserves an 8-byte magnitude with `len=8`; a non-minimal magnitude decodes fine because
    /// `BigInt::from_bytes_be` normalizes leading zeros). A NEGATIVE value also flips the kind byte at
    /// `offset - 2` from `INT_POS_DEC` to `INT_NEG_DEC` and writes the ABSOLUTE magnitude.
    Int,
    /// A boxed boolean: read `get-bool`, write the kind byte at `offset` — `9` (true) or `8` (false).
    Bool,
}

/// The value-form byte TEMPLATE for a runtime compound of type `ty`: the codec bytes with every leaf's
/// value left as a placeholder, plus the [`RuntimeLeaf`] holes to fill at run time. Everything static —
/// the `(: value type)` structure, the `tuple`/`record` heads + field names, the whole TYPE node, and
/// each leaf's kind/len framing — is baked; only the leaf VALUES are holes. `encode()` copies this
/// template into linear memory, walks each hole's heap path, and writes the value (R2). `None` if the
/// type has no value-form surface (a function/type-value). Every leaf is treated as a runtime hole
/// (walked from the handle), so a mixed const/runtime compound needs no special-casing — a constant
/// element still sits boxed on the heap and reads back the same.
pub fn runtime_value_form_template(ty: &crate::ty::Ty) -> Option<ValueFormTemplate> {
    let mut b = crate::ast::Builder::new();
    let colon = b.name(":");
    // Build the value AST with PLACEHOLDER leaves, recording each leaf's walk path + kind as we go.
    let mut leaves: Vec<PendingLeaf> = Vec::new();
    let value = template_value_ast(&mut b, ty, &mut Vec::new(), &mut leaves)?;
    let type_ast = type_ast(&mut b, ty)?;
    let root = b.list(vec![colon, value, type_ast]);
    let arenas = b.finish(root);
    let bytes = crate::codec::encode(&arenas);
    // Locate each placeholder leaf's byte offset in the encoded LEAF POOL (leaves are encoded in order
    // right after the 8-byte header + leaf-count LEB). Walk the pool, tracking offsets; a leaf that was
    // recorded as runtime (by its LeafId) gets its hole offset resolved here.
    let holes = resolve_leaf_offsets(&bytes, &arenas, &leaves)?;
    Some(ValueFormTemplate {
        bytes,
        leaves: holes,
    })
}

/// A value-form template: the byte buffer (placeholders in the leaf values) + the runtime holes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ValueFormTemplate {
    pub bytes: Vec<u8>,
    pub leaves: Vec<RuntimeLeaf>,
}

/// A leaf recorded during template construction, before its byte offset is resolved: its arena `LeafId`
/// (to locate it in the encoded pool) plus the runtime info the hole carries.
struct PendingLeaf {
    leaf_id: crate::ast::LeafId,
    path: Vec<u32>,
    kind: LeafFill,
}

/// Build the VALUE s-expression for a type with PLACEHOLDER leaves, recording each scalar leaf's walk
/// `path` (the `arr-get` indices to reach it) and kind. A tuple/record recurses, pushing the positional
/// index onto the path; a scalar emits a placeholder atom and records a `PendingLeaf`. `None` for a type
/// with no value surface.
fn template_value_ast(
    b: &mut crate::ast::Builder,
    ty: &crate::ty::Ty,
    path: &mut Vec<u32>,
    out: &mut Vec<PendingLeaf>,
) -> Option<StructId> {
    use crate::ast::{Leaf, Radix};
    use crate::ty::Ty;
    match ty {
        Ty::Int(_) => {
            // Placeholder: a positive zero with a FIXED 8-byte magnitude, so the template reserves an
            // 8-byte hole (len=8) the runtime overwrites with the leaf's big-endian magnitude (a
            // non-minimal magnitude decodes fine — `BigInt::from_bytes_be` drops leading zeros). Pushed
            // NON-deduped (`leaf_unique`) so this occurrence has its OWN pool entry and hence its own
            // byte offset — two equal placeholders must not collapse to one hole.
            let leaf_id = b.leaf_unique(Leaf::Int {
                value: crate::ast::IntValue {
                    negative: false,
                    magnitude: vec![0u8; 8],
                },
                radix: Radix::Dec,
            });
            let atom = b.atom(leaf_id);
            out.push(PendingLeaf {
                leaf_id,
                path: path.clone(),
                kind: LeafFill::Int,
            });
            Some(atom)
        }
        Ty::Bool => {
            // Placeholder `false`; the runtime overwrites the kind byte (8=false / 9=true). Pushed
            // NON-deduped so each bool occurrence has its own pool entry + offset.
            let leaf_id = b.leaf_unique(Leaf::Bool(false));
            let atom = b.atom(leaf_id);
            out.push(PendingLeaf {
                leaf_id,
                path: path.clone(),
                kind: LeafFill::Bool,
            });
            Some(atom)
        }
        Ty::Tuple(elems) => {
            let head = b.name("tuple");
            let mut children = vec![head];
            for (i, e) in elems.iter().enumerate() {
                path.push(i as u32);
                children.push(template_value_ast(b, e, path, out)?);
                path.pop();
            }
            Some(b.list(children))
        }
        Ty::Record(fields) => {
            let head = b.name("record");
            let mut children = vec![head];
            // A record is a positional heap array in canonical (sorted) field order — the same order the
            // BTreeMap iterates, so the `arr-get` index is the field's position in that order.
            for (i, (name, t)) in fields.iter().enumerate() {
                let fname = b.name(&name.name);
                path.push(i as u32);
                let fval = template_value_ast(b, t, path, out)?;
                path.pop();
                children.push(b.list(vec![fname, fval]));
            }
            Some(b.list(children))
        }
        _ => None,
    }
}

/// Resolve each pending leaf's BYTE OFFSET in the encoded template. Re-encodes the leaf pool the same
/// way `codec::encode` does (header + count, then each leaf), tracking the running offset; when a leaf's
/// `LeafId` matches a pending runtime leaf, its hole offset is the magnitude position (Int: after the
/// kind + len bytes) or the kind-byte position (Bool). Returns the resolved holes in the pending order.
fn resolve_leaf_offsets(
    bytes: &[u8],
    arenas: &crate::ast::Arenas,
    pending: &[PendingLeaf],
) -> Option<Vec<RuntimeLeaf>> {
    // Offset walk mirrors `codec::encode`: 8-byte header, then a LEB128 leaf-count, then each leaf.
    let mut off = 8usize;
    off += leb_len(arenas.leaves.len() as u64);
    // Map each LeafId → (magnitude offset for Int, kind-byte offset for Bool).
    let mut leaf_off: std::collections::HashMap<u32, (usize, LeafFill)> =
        std::collections::HashMap::new();
    for (i, leaf) in arenas.leaves.iter().enumerate() {
        let kind_off = off;
        match leaf {
            crate::ast::Leaf::Int { value, .. } => {
                // kind byte (1) + len LEB + magnitude.
                let len = value.magnitude.len();
                let mag_off = off + 1 + leb_len(len as u64);
                leaf_off.insert(i as u32, (mag_off, LeafFill::Int));
                off = mag_off + len;
            }
            crate::ast::Leaf::Bool(_) => {
                leaf_off.insert(i as u32, (kind_off, LeafFill::Bool));
                off += 1;
            }
            crate::ast::Leaf::Name(n) => {
                off += 1 + leb_len(n.len() as u64) + n.len();
            }
            crate::ast::Leaf::Str(s) => {
                off += 1 + leb_len(s.len() as u64) + s.len();
            }
            crate::ast::Leaf::Float(_) => return None, // floats not yet in the runtime escape
        }
    }
    let _ = bytes;
    let mut holes = Vec::with_capacity(pending.len());
    for p in pending {
        let (offset, _) = leaf_off.get(&p.leaf_id.0)?;
        holes.push(RuntimeLeaf {
            offset: *offset,
            path: p.path.clone(),
            kind: p.kind,
        });
    }
    Some(holes)
}

/// The number of bytes the unsigned LEB128 encoding of `n` occupies (matches `encode::uleb128`).
fn leb_len(mut n: u64) -> usize {
    let mut c = 1;
    while n >= 0x80 {
        n >>= 7;
        c += 1;
    }
    c
}

/// Reconstruct the VALUE s-expression of a constant node into `b`: a scalar → its literal atom; a
/// `Core::Tuple` → `(tuple <elem>…)`; a `Core::Record` → `(record (<name> <value>)…)` in canonical field
/// order. `None` if the node is not a constant the escape path can bake.
fn const_value_ast(db: &mut Db, b: &mut crate::ast::Builder, id: StructId) -> Option<StructId> {
    use crate::ast::{Leaf, Radix};
    match core_of(db, id) {
        Core::ConstInt(v) => Some(b.atom_leaf(Leaf::Int {
            value: v,
            radix: Radix::Dec,
        })),
        Core::ConstBool(x) => Some(b.atom_leaf(Leaf::Bool(x))),
        Core::Unit => Some(b.name("unit")),
        Core::Tuple { elems } => {
            let head = b.name("tuple");
            let mut children = vec![head];
            for e in elems {
                children.push(const_value_ast(db, b, e)?);
            }
            Some(b.list(children))
        }
        Core::Record { fields } => {
            let head = b.name("record");
            let mut children = vec![head];
            // Canonical (sorted) field order — a `BTreeMap` iterates sorted, matching the type render.
            for (name, &v) in &fields {
                let fname = b.name(name.name.clone());
                let fval = const_value_ast(db, b, v)?;
                children.push(b.list(vec![fname, fval]));
            }
            Some(b.list(children))
        }
        _ => None,
    }
}

/// Reconstruct a TYPE s-expression into `b`, matching `Ty::render_name`'s surface exactly so the host
/// prints the recorded type: `Int64`/`UInt8`/… as a name atom, `Bool`/`Unit` likewise, a tuple as
/// `(Tuple T…)`, a record as `(record (name T)…)`. `None` for a type with no value-form surface (a
/// function/type-value/unsolved variable can never be a runtime value crossing the boundary).
fn type_ast(b: &mut crate::ast::Builder, ty: &crate::ty::Ty) -> Option<StructId> {
    use crate::ty::Ty;
    match ty {
        // A sum's type surface is its nominal name (`(: (Some 5) Option)` annotates `Option`), like a
        // scalar's. (No sum value can cross the boundary until construction lands in a later tick; the
        // value-form surface — `(disc payload)` from the heap walk — is that tick's work.)
        Ty::Int(_) | Ty::Bool | Ty::Unit | Ty::Sum { .. } => Some(b.name(ty.render_name())),
        Ty::Tuple(elems) => {
            let head = b.name("Tuple");
            let mut children = vec![head];
            for t in elems.iter() {
                children.push(type_ast(b, t)?);
            }
            Some(b.list(children))
        }
        Ty::Record(fields) => {
            // The TYPE head is capitalized `Record` (like `Tuple`); the VALUE head is lowercase `record`
            // (see `const_value_ast`). The corpus writes `(Record (a Int64) …)` for the type.
            let head = b.name("Record");
            let mut children = vec![head];
            for (name, t) in fields.iter() {
                let fname = b.name(name.name.clone());
                let fty = type_ast(b, t)?;
                children.push(b.list(vec![fname, fty]));
            }
            Some(b.list(children))
        }
        Ty::Fn(_, _) | Ty::Type | Ty::Var(_) | Ty::Any => None,
    }
}

/// The number of times the binding whose initializer is `init` is REFERENCED within the resolved
/// subtree rooted at `node` — a use is a `Resolved::Ref { value: init }` (the identity a reference to
/// the binding resolves to). Walks the resolved tree structurally without lowering; a nested `let`
/// that SHADOWS the name rebinds references below it to a different init, so those do not count (they
/// resolve to the inner binding's occurrence, not `init`). Bounded by the subtree size.
fn uses_in(db: &mut Db, node: StructId, init: StructId) -> u32 {
    match resolved_of(db, node) {
        Resolved::Ref { value } => {
            if value == init {
                1
            } else {
                // A ref to ANOTHER binding — but its value may itself reference `init` (e.g. a later
                // `let` binding's initializer). Do not descend through the ref target here: the walk
                // over the enclosing structure already visits every initializer/body position, so
                // counting the ref itself (0 for a different binding) avoids double-counting.
                0
            }
        }
        Resolved::If { cond, then_, else_ } => {
            uses_in(db, cond, init) + uses_in(db, then_, init) + uses_in(db, else_, init)
        }
        Resolved::Let { bindings, body } => {
            let mut n = 0;
            for (_, value) in &bindings {
                n += uses_in(db, *value, init);
            }
            n + uses_in(db, body, init)
        }
        Resolved::Record { fields } => {
            let mut n = 0;
            for value in fields.values() {
                n += uses_in(db, *value, init);
            }
            n
        }
        Resolved::Member { operand, .. } => uses_in(db, operand, init),
        Resolved::Tuple { elems } => {
            let mut n = 0;
            for &e in elems.iter() {
                n += uses_in(db, e, init);
            }
            n
        }
        Resolved::Proj { operand, .. } => uses_in(db, operand, init),
        Resolved::Annot { expr, .. } => uses_in(db, expr, init),
        Resolved::Apply { head, args } => {
            let mut n = uses_in(db, head, init);
            for a in &args {
                n += uses_in(db, *a, init);
            }
            n
        }
        // A match: the scrutinee and every arm body may reference the binding. (A literal pattern is a
        // constant, not a reference.) The scrutinee runs once; each arm body is a distinct use position.
        Resolved::Match { scrutinee, arms } => {
            let mut n = uses_in(db, scrutinee, init);
            for (_, body) in &arms {
                n += uses_in(db, *body, init);
            }
            n
        }
        // Leaves and non-referencing forms contribute nothing.
        Resolved::Int(_)
        | Resolved::Bool(_)
        | Resolved::Unit
        | Resolved::Prim(_)
        | Resolved::Param { .. }
        | Resolved::TypeVal(_)
        | Resolved::Lambda { .. }
        | Resolved::Poison(_) => 0,
    }
}

/// Lower an ARITHMETIC application: FOLD it when its operands fold to constants — evaluate at compile
/// time with a CHECKED operation, so a provable overflow is a build error (CDZ0304 poison) rather than
/// a shipped runtime trap (`reference-compiler.md` §A Compile-Provable Trap Fails The Build). An
/// operand that is not a constant stays a runtime `Arith` (its wasm op selected from the solved width
/// at selection); a poison operand propagates.
fn lower_arith(db: &mut Db, op: Prim, args: &[StructId]) -> Core {
    if args.len() != 2 {
        return Core::Poison(Reject::coded(
            Code::Malformed,
            format!("{} takes exactly 2 operands", intrinsic_name(op)),
        ));
    }
    let lhs = core_of(db, args[0]);
    let rhs = core_of(db, args[1]);
    match (lhs, rhs) {
        (Core::ConstInt(a), Core::ConstInt(b)) => fold_arith(op, a, b),
        (Core::Poison(r), _) | (_, Core::Poison(r)) => Core::Poison(r),
        _ => {
            trace!(target: "rcdzc::lower", op = intrinsic_name(op), "arithmetic stays runtime (operand not constant)");
            Core::Arith {
                op,
                lhs: args[0],
                rhs: args[1],
            }
        }
    }
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
        | Prim::Wrap
        | Prim::IntCtor
        | Prim::UIntCtor
        | Prim::FnCtor
        | Prim::TupleCtor
        | Prim::RecordCtor
        | Prim::BoolTy
        | Prim::UnitTy => {
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
                    "constant {} has no defined value (overflow, divide-by-zero, or out-of-range shift)",
                    intrinsic_name(op)
                ),
            ))
        }
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
fn lower_comparison(db: &mut Db, op: Prim, args: &[StructId]) -> Core {
    if args.len() != 2 {
        return Core::Poison(Reject::coded(
            Code::Malformed,
            format!("{} takes exactly 2 operands", intrinsic_name(op)),
        ));
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
            // An operand beyond the fold's machine range — decline (a wider fold arrives with widths).
            _ => Core::Poison(Reject::decline(
                "comparison of an integer beyond the machine width is not yet folded",
            )),
        },
        (Core::ConstBool(a), Core::ConstBool(b)) => {
            let r = compare_ord(op, a.cmp(&b));
            trace!(target: "rcdzc::fold", op = intrinsic_name(op), result = r, "folded constant boolean comparison");
            Core::ConstBool(r)
        }
        (Core::Poison(r), _) | (_, Core::Poison(r)) => Core::Poison(r),
        // A non-constant operand: a runtime comparison IF both operands are scalars (integers or
        // booleans, which have a machine representation the backend can compare); a compound operand
        // still declines (heap-walk equality is a later stage).
        _ => {
            if is_scalar(db, args[0]) && is_scalar(db, args[1]) {
                trace!(target: "rcdzc::lower", op = intrinsic_name(op), "comparison stays runtime (scalar operands)");
                Core::Compare {
                    op,
                    lhs: args[0],
                    rhs: args[1],
                }
            } else {
                trace!(target: "rcdzc::lower", op = intrinsic_name(op), "decline: comparison of a compound value needs a heap walk");
                Core::Poison(Reject::decline(
                    "comparison of a compound value needs a heap walk (not yet built)",
                ))
            }
        }
    }
}

/// Lower a CONVERSION application (`T.wrap`). Truncating: keeps the low `N` bits of the operand at the
/// TARGET width/signedness (`Prim::Wrap`), NEVER traps. The target type is the CONVERSION NODE's own
/// solved type (`type_of(db, id)`), read here so the fold and the runtime path agree on the width. A
/// constant operand FOLDS via `IntValue::wrap_to` to a `ConstInt` already at the target width; a runtime
/// operand becomes a `Core::Convert` the backend emits as a mask-and-reinterpret. A poison propagates.
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
        Core::ConstInt(v) => {
            // Fold: truncate to the target width at arbitrary precision (total — never traps).
            let wrapped = v.wrap_to(signed, width);
            trace!(target: "rcdzc::fold", op = intrinsic_name(op), signed, width, "folded constant wrap");
            Core::ConstInt(wrapped)
        }
        Core::Poison(r) => Core::Poison(r),
        // A runtime operand: emit the mask-and-reinterpret at selection (the target is read off this
        // node's solved type there, the same `type_of(id)` used here).
        _ => {
            if is_scalar(db, args[0]) {
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

/// Whether the node at `id` has a SCALAR solved type — an integer or a boolean, which occupies a
/// machine slot the backend can compare or compute on directly (as opposed to a compound/heap value).
fn is_scalar(db: &mut Db, id: StructId) -> bool {
    matches!(
        crate::infer::type_of(db, id),
        crate::ty::Ty::Int(_) | crate::ty::Ty::Bool
    )
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
        Prim::Wrap => "wrap",
        Prim::IntCtor => "Int",
        Prim::UIntCtor => "UInt",
        Prim::FnCtor => "->",
        Prim::TupleCtor => "Tuple",
        Prim::RecordCtor => "Record",
        Prim::BoolTy => "Bool",
        Prim::UnitTy => "Unit",
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
        let tpl = runtime_value_form_template(ty).expect("template");
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
        let nested =
            crate::ty::Ty::Tuple(vec![t_int(), crate::ty::Ty::Tuple(vec![t_int(), t_int()].into())].into());
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
            "(: (record (a 3) (b 1)) (Record (a Int64) (b Int64)))"
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
    fn lowers_an_if_referencing_its_child_ids() {
        let (ast, if_node) = if_program();
        let mut db = Db::load(ast);
        match core_of(&mut db, if_node) {
            Core::If { cond, then_, else_ } => {
                assert_eq!(core_of(&mut db, cond), Core::ConstBool(false));
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
}
