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
        Resolved::Str(s) => Core::ConstStr(s),
        // A FLOAT literal folds to its exact `Core::ConstFloat` — a `Ty::Float` value. This lets float
        // EQUALITY fold (two constants compared by canonical value). It still cannot cross the boundary
        // as a value or be an arithmetic operand (no f64 machine path yet) — those sites decline where
        // they consume it; the CONSTANT itself is now a real core value.
        Resolved::Float(d) => Core::ConstFloat(d),
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
        // A sum-variant pattern's payload binder — read the scrutinee's payload. If the scrutinee is a
        // CONSTANT sum (`Core::SumNew` with a single payload), FOLD to that payload's core directly — a
        // constant `(match (Some 5) ((Some x) x))` yields the constant `5`, no heap build/read (the sum
        // analogue of a constant tuple projection folding). Otherwise it is a runtime read:
        // `sum-payload(scrutinee)` then unbox by the payload's solved type. The disc is not needed
        // (control is already in the matched arm).
        Resolved::SumPayload {
            scrutinee, steps, ..
        } => {
            // FOLD when the whole path lands in constant `Core::SumNew` payloads — a constant `(match
            // (Some 5) ((Some x) x))` yields `5`, no heap read (extends to nesting: `(Some (Some 5))`
            // through `[Payload, Payload]` folds to `5`). Otherwise emit a runtime `Core::SumPayload`
            // that walks the path.
            if let Some(folded) = fold_sum_path(db, scrutinee, &steps) {
                folded
            } else {
                Core::SumPayload {
                    scrutinee,
                    path: steps.to_vec(),
                }
            }
        }
        // A `let` — A-NORMALIZE its bindings: a binding whose value is a runtime computation used more
        // than once is NAMED (a `Core::Let` binding, computed once, read by `LocalRef`); a single-use
        // or constant binding is copy-propagated / erased (its references follow through to its value).
        // So naming adds no cost and the emitted bytes are unchanged for a program with no multi-use
        // runtime binding (`reference-compiler.md` §The Core Representation Is In A-Normal Form).
        Resolved::Let { bindings, body } => lower_let(db, &bindings, body),
        // A NULLARY VARIANT used as a value (`None`) — its ctor record carries `(meta variant)` and its
        // type is the sum (no payload arrow). It constructs `sum-new(disc, unit)` with no payloads. A
        // PAYLOAD variant record used WITHOUT being applied (`Some` bare) is a function value with no
        // runtime form yet — decline (a variant constructor is applied to construct; a bare partial
        // application needs closures). This is checked before the plain-record arm so a variant is not
        // lowered as a data record of its meta fields.
        Resolved::Record { .. } if crate::eval::variant_disc_of(db, id).is_some() => {
            match crate::infer::type_of(db, id) {
                // Nullary variant value — its type is the sum directly.
                crate::ty::Ty::Sum { .. } => {
                    let disc = crate::eval::variant_disc_of(db, id).unwrap_or(0);
                    Core::SumNew {
                        disc,
                        payloads: Vec::new(),
                    }
                }
                // A payload variant used bare is a partial application (a function value).
                _ => Core::Poison(Reject::decline(
                    "a variant constructor with payloads must be applied to its arguments",
                )),
            }
        }
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
            // The operand did not reduce to a compile-time-visible record. MEMBER-INTO-IF: if it is an
            // `(if c R S)` whose BOTH branches are visible records carrying the field →
            // `(if c R.key S.key)`, pushing the member read into each branch. The record analogue of the
            // tuple `PROJECTION-INTO-IF` (a record built through an `if` was OPAQUE to `member_value`, so
            // it stayed a runtime heap value — `arr-alloc` + per-field box/set + `arr-get`/unbox, purely
            // to read one field back). Reuses the EXISTING field-value occurrences as the branches (no
            // ast synthesis, no re-resolution — each keeps its resolved scope); the un-read sibling
            // fields drop exactly as a visible-record member fold drops them, and `c` is evaluated either
            // way so its trap is preserved. `member_value` on each branch reduces it to its record and
            // projects `key` (by name — order-independent); a branch missing the field, or a kept
            // multi-use `if`-binding (`reduce_to_if` stops there), declines this and falls through to the
            // runtime read below.
            crate::eval::Member::NotRecord => {
                if let Some((cond, then_, else_)) = crate::eval::reduce_to_if(db, operand)
                    && let crate::eval::Member::Field(tf) =
                        crate::eval::member_value(db, then_, &key)
                    && let crate::eval::Member::Field(ef) =
                        crate::eval::member_value(db, else_, &key)
                {
                    trace!(target: "rcdzc::fold", node = id.0, key = %key.name, "member read pushed into an if of records (no heap build)");
                    Core::If {
                        cond,
                        then_: tf,
                        else_: ef,
                    }
                } else {
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
            }
        },
        // A tuple literal — kept as a compound. Like a record, it folds away only when a projection
        // reads a visible element of it; a tuple that survives (constructed from runtime operands, or a
        // constant tuple that escapes) is a `Core::Tuple` the backend builds on the heap.
        Resolved::Tuple { elems } => Core::Tuple {
            elems: elems.to_vec(),
        },
        // A list literal — a `Core::ListNew` the backend builds on the persistent `vec-*` heap. (Unlike a
        // tuple, a list has no projection-fold: `List.len`/`List.at` are operations, not a static index.)
        Resolved::List { elems } => Core::ListNew {
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
                    // PROJECTION-INTO-IF: `(. (if c T E) i)` where BOTH branches are visible tuples of
                    // matching arity → `(if c T[i] E[i])`, pushing the projection into each branch. This
                    // reuses the EXISTING element occurrences as the `if`'s branches (no ast synthesis,
                    // no re-resolution — each keeps its resolved scope), so a tuple built through an `if`
                    // never reaches the heap when it is only projected: the two branch tuples fold away
                    // (their un-projected siblings drop exactly as a plain tuple projection drops them),
                    // leaving one `if` over the two selected elements. `c` is evaluated either way, so any
                    // trap in it is preserved. An out-of-arity index is impossible here (rejected in
                    // `type_errors`); defensively it poisons like the visible-tuple case.
                    if let Some((cond, te, ee)) = crate::eval::reduce_to_if_of_tuples(db, operand) {
                        match (te.get(index), ee.get(index)) {
                            (Some(&then_), Some(&else_)) => {
                                trace!(target: "rcdzc::fold", node = id.0, index, "projection pushed into an if of tuples (no heap build)");
                                Core::If { cond, then_, else_ }
                            }
                            _ => Core::Poison(Reject::coded(
                                Code::Malformed,
                                format!("tuple index {index} is out of range"),
                            )),
                        }
                    } else {
                        trace!(target: "rcdzc::lower", node = id.0, operand = operand.0, index, "tuple projection stays runtime (operand is a runtime tuple)");
                        Core::Proj { operand, index }
                    }
                }
            }
        }
        // An `if`. FOLD when the condition reduces to a compile-time-constant boolean: the branch the
        // condition selects IS the result, so lower it directly and drop the `if`. This is dead-branch
        // elimination on a proven-constant condition — the untaken branch NEVER executes at run time.
        // ⚠ WHAT MAY BE DROPPED from the untaken branch mirrors the reachability model
        // (`compile::collect_reached_poisons`, which does NOT descend an `if`'s branches): a RUNTIME TRAP
        // shielded by an untaken branch is not a build failure, so a `ConstTrap` (CDZ0304) untaken branch
        // folds away (`(if (< 1 2) 7 (% 5 0))` → 7 — the div-by-zero is unreachable). But a NON-TRAP
        // poison — an ill-FORMED untaken branch (an unbound name, a type mismatch, an unsupported
        // literal like a float, whose branch also DISAGREES in type with the taken one, e.g.
        // `(if true 1 3.5)`) — is a static well-formedness fault the program must be REJECTED for,
        // reachability notwithstanding. So keep the `Core::If` when the untaken branch is a non-trap
        // poison, letting that fault surface; fold otherwise. A runtime condition stays a `Core::If`.
        Resolved::If { cond, then_, else_ } => {
            // CONDITIONAL CONSTANT PROPAGATION on a REPEATED condition (runtime `c` only). Within the
            // THEN-branch `c` is known TRUE, within the ELSE-branch FALSE — so a branch that is ITSELF
            // `(if c' A B)` with `c'` EQUIVALENT to `c` (a syntactically-equal PURE condition; with no
            // mutation it re-evaluates identically) is redundant: take `A` in the then-branch, `B` in the
            // else-branch. Rewrite the branch to that inner arm, REUSING its existing occurrence (no
            // synthesis), so the folds below see the simplified branches (`(if c (if c A B) E)` →
            // `(if c A E)`, collapsing further if that leaves identical branches). Only a RUNTIME `c` is
            // rewritten: for a CONSTANT `c` the untaken branch is dead and the `ConstBool` arm's
            // untaken-illformed check must see the ORIGINAL branch (skip the rewrite), and a poison `c`
            // propagates. The inner `if`'s DROPPED arm may hide a runtime trap — unreachable under `c`, so
            // dropping it mirrors the reachability model (as the constant-condition fold drops a
            // `ConstTrap` untaken branch). `core_equiv`'s pure-core matching guarantees `c'` carries no
            // new effect (params/locals/consts/arith/compare/convert only).
            let (then_, else_) =
                if matches!(core_of(db, cond), Core::ConstBool(_) | Core::Poison(_)) {
                    (then_, else_)
                } else {
                    (
                        collapse_repeated_cond(db, cond, then_, true).unwrap_or(then_),
                        collapse_repeated_cond(db, cond, else_, false).unwrap_or(else_),
                    )
                };
            match core_of(db, cond) {
                Core::ConstBool(b) => {
                    let (taken, dropped) = if b { (then_, else_) } else { (else_, then_) };
                    let untaken_is_illformed = matches!(
                        core_of(db, dropped),
                        Core::Poison(r) if r.code != Some(Code::ConstTrap)
                    );
                    if untaken_is_illformed {
                        Core::If { cond, then_, else_ }
                    } else {
                        trace!(target: "rcdzc::lower", node = id.0, taken = b, "if with a constant condition folds to the taken branch");
                        core_of(db, taken)
                    }
                }
                // A condition that is a poison propagates (the ill-formed condition is the fault).
                Core::Poison(r) => Core::Poison(r),
                // A runtime condition. If BOTH branches are the SAME value (`(if c x x)`, or two branches
                // that FOLD to the same core — e.g. `(if c (+ x 0) x)` after the identity fold), the `if`
                // computes that value regardless, so it collapses to the branch — BUT only when the
                // condition is TRAP-FREE: the condition is still evaluated at run time, so if it could trap
                // (a call, a checked op) that trap must be preserved (keep the `if`). A trap-free condition
                // (a param/local, a comparison, a bitwise op) has no observable effect to keep.
                _ if core_equiv(db, then_, else_) && is_trap_free(db, cond) => {
                    trace!(target: "rcdzc::lower", node = id.0, "if with identical branches folds to the branch (trap-free condition)");
                    core_of(db, then_)
                }
                // BOOLEAN COERCION: `(if c true false)` is just `c` — the `if` computes the condition's own
                // value. `c` is a `Bool` (an `if` condition must be), and it is evaluated on BOTH branches of
                // the original, so returning it drops the `if` with no change (including any trap in `c`,
                // which still fires — `c` is unconditionally evaluated here just as it was as the condition).
                _ if matches!(core_of(db, then_), Core::ConstBool(true))
                    && matches!(core_of(db, else_), Core::ConstBool(false)) =>
                {
                    trace!(target: "rcdzc::lower", node = id.0, "if c true false folds to the condition c");
                    core_of(db, cond)
                }
                // BOOLEAN NEGATION: `(if c false true)` is `!c`. `c` is unconditionally evaluated (as the
                // condition), so negating its value drops the `if` with no other change (any trap in `c`
                // still fires). A runtime `c` becomes `Core::Not{c}` (emitted as `i32.eqz`); a constant `c`
                // would already have folded via the `ConstBool` arm above, so here `c` is a runtime bool.
                _ if matches!(core_of(db, then_), Core::ConstBool(false))
                    && matches!(core_of(db, else_), Core::ConstBool(true)) =>
                {
                    trace!(target: "rcdzc::lower", node = id.0, "if c false true folds to the negation !c");
                    Core::Not { operand: cond }
                }
                _ => Core::If { cond, then_, else_ },
            }
        }
        // A SHORT-CIRCUITING connective. Fold on a constant LEFT operand — the short-circuit rule decides
        // the result WITHOUT evaluating `rhs` (so a trapping/ill-formed `rhs` is shielded, exactly as an
        // `if`'s unselected branch): `(and false _)`→false, `(and true rhs)`→rhs; `(or true _)`→true,
        // `(or false rhs)`→rhs. A non-constant `lhs` (or a poison `lhs`, which propagates) emits a
        // `Core::And` the backend lowers to `if lhs then/else <rhs|const>`.
        Resolved::And { lhs, rhs, is_and } => match core_of(db, lhs) {
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
                _ => Core::And { lhs, rhs, is_and },
            },
        },
        // Negation: fold a constant, `(not (not x))` → x (double negation), else `Core::Not` (i32.eqz).
        Resolved::Not { operand } => match core_of(db, operand) {
            Core::ConstBool(b) => Core::ConstBool(!b),
            // Double negation: the operand is itself a `Not` — the two cancel, so the result is the INNER
            // operand's core. `not` is total (no trap, no effect), so cancelling the pair changes nothing.
            Core::Not { operand: inner } => core_of(db, inner),
            Core::Poison(r) => Core::Poison(r),
            _ => Core::Not { operand },
        },
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
            // A PERFORM that reaches lowering directly — no enclosing handler discharged it (a handled
            // perform is REDUCED AWAY by `effects::reduce_handle` before its body is lowered, so it never
            // reaches here) and no host delegation routed it (E2). Whether this is an ERROR depends on
            // CONTEXT: an unhandled perform reached from an ENTRYPOINT escapes ungranted (CDZ0401 — the
            // "no home" check, reported at the export level in `compile.rs`), but a perform in a LIBRARY
            // function's body is fine — its home is whatever handler/delegation encloses its CALLERS (the
            // cross-function inline trigger resolves it there). So here — the standalone lowering of an
            // arbitrary def body — a bare perform is a DECLINE, not a coded reject: a library def that
            // performs an effect stays well-formed, while the entrypoint-level check catches a genuinely
            // ungranted escape. (Reported cleanly rather than leaking the op's `(intrinsic perform)` marker
            // as an "unknown intrinsic".)
            if crate::eval::effect_op_of(db, head).is_some() {
                trace!(target: "rcdzc::lower", node = id.0, head = head.0, "apply: unhandled perform at standalone lowering → decline (entrypoint check reports CDZ0401)");
                return Core::Poison(Reject::decline(
                    "this effect operation is performed with no enclosing handler here; its home is \
                     determined by the handler or delegation enclosing its callers",
                ));
            }
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
                        // The REDUCTION-depth limit was hit — not (necessarily) a recursive callee, just
                        // a call chain nested deeper than the inliner reduces (`REDUCE_DEPTH_LIMIT`). A
                        // finite deep chain is a resource-limit DECLINE, not a miscompile; name it
                        // accurately (the old "recursive function" wording misdescribed a plain deep
                        // nest, which since inlining became linear is now reachable on a well-formed
                        // program). This does NOT route through `lower_recursive_call_or_decline` (that is
                        // only for an `is_recursive`-origin decline), so the wording is free to be exact.
                        trace!(target: "rcdzc::lower", node = id.0, "apply: reduction depth limit hit → decline (resource limit)");
                        return Core::Poison(Reject::decline(
                            "a call chain nested deeper than the inliner reduces (a resource limit was reached)",
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
            // An EMPTY compound-VALUE constructor — `(list)` / `(tuple)` / `(record)` written with the
            // alias name at zero args — BUILDS the empty compound, it is NOT the ctor value. Route it
            // through `reduce_ctor` (which rewrites `(list)` → `("list")` → the symbol form) before the
            // zero-arg identity short-circuit below (which would return the ctor record and then decline
            // it as a bare built-in value). A NON-empty alias application reaches `reduce_ctor` via the
            // `Some(prim)` arm; this is only the nullary case the short-circuit would otherwise capture.
            if args.is_empty()
                && matches!(
                    crate::eval::meta_apply_of(db, head),
                    Some(Prim::TupleNew | Prim::RecordNew | Prim::ListNew)
                )
            {
                let prim = crate::eval::meta_apply_of(db, head).unwrap();
                return match crate::eval::reduce_ctor(db, prim, id, &args) {
                    Ok(built) => core_of(db, built),
                    Err(msg) => Core::Poison(Reject::decline(msg)),
                };
            }
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
                // A sum VARIANT CONSTRUCTOR applied — `(Option.Some 5)`. The discriminant is read off
                // the head's `(meta variant)` channel (the value the shared `sum-new` prim needs); the
                // args are the payloads. Build `Core::SumNew{disc, payloads}` the backend lowers to
                // `sum-new(disc, payload)`.
                Some(Prim::SumNew) => {
                    trace!(target: "rcdzc::lower", node = id.0, "apply: sum variant constructor");
                    lower_sum_new(db, head, &args)
                }
                // `List.len` applied to a list — FOLD when the operand is a compile-time-visible list
                // literal (its length is statically known), else emit `Core::ListLen` (the runtime
                // `vec-len`). One operand: the list.
                Some(Prim::ListLen) if args.len() == 1 => {
                    let operand = args[0];
                    match core_of(db, operand) {
                        Core::ListNew { elems } => {
                            trace!(target: "rcdzc::fold", node = id.0, len = elems.len(), "List.len folds to a constant (visible list literal)");
                            Core::ConstInt(IntValue::from_i64(elems.len() as i64))
                        }
                        Core::Poison(r) => Core::Poison(r),
                        _ => Core::ListLen { operand },
                    }
                }
                // `List.push` / `List.concat` — runtime `vec-push`/`vec-concat`. A poison operand
                // propagates; otherwise emit the runtime op (no constant fold — a persistent push/concat
                // builds a new heap value, not worth folding a constant spine here).
                Some(Prim::ListPush) if args.len() == 2 => {
                    if let Core::Poison(r) = core_of(db, args[0]) {
                        Core::Poison(r)
                    } else if let Core::Poison(r) = core_of(db, args[1]) {
                        Core::Poison(r)
                    } else {
                        Core::ListPush {
                            list: args[0],
                            elem: args[1],
                        }
                    }
                }
                Some(Prim::ListConcat) if args.len() == 2 => {
                    if let Core::Poison(r) = core_of(db, args[0]) {
                        Core::Poison(r)
                    } else if let Core::Poison(r) = core_of(db, args[1]) {
                        Core::Poison(r)
                    } else {
                        Core::ListConcat {
                            lhs: args[0],
                            rhs: args[1],
                        }
                    }
                }
                // `List.update` — replace the element at an index (runtime `vec-update`). Three args:
                // the list, the Int64 index, the replacement element. Any poison operand propagates;
                // otherwise emit the runtime op (no constant fold — a persistent update builds a new
                // heap value, like push/concat).
                Some(Prim::ListUpdate) if args.len() == 3 => {
                    if let Core::Poison(r) = core_of(db, args[0]) {
                        Core::Poison(r)
                    } else if let Core::Poison(r) = core_of(db, args[1]) {
                        Core::Poison(r)
                    } else if let Core::Poison(r) = core_of(db, args[2]) {
                        Core::Poison(r)
                    } else {
                        Core::ListUpdate {
                            list: args[0],
                            index: args[1],
                            elem: args[2],
                        }
                    }
                }
                // `List.at` — the FALLIBLE indexed read `(List a) → Int64 → (Option a)`. FOLD when the
                // list is a compile-time-visible literal AND the index is a constant: an in-range index
                // yields `(Some elem)` (the element's own core), an out-of-range one (negative, or `>=`
                // arity) yields `None` — both built as a `Core::SumNew` of the result Option's variant
                // discriminants, so a constant `List.at` renders through the ordinary sum escape/fold with
                // no heap read. Otherwise emit the runtime `Core::ListAt` (a bounds-checked `vec-get`).
                Some(Prim::ListAt) if args.len() == 2 => lower_list_at(db, id, args[0], args[1]),
                // `Bytes.of` — construct a byte sequence from a list of `Int64` in `0..=255`. When the
                // operand is a compile-time-visible list literal, RANGE-CHECK each element now (a `< 0`
                // or `> 255` value is a compile-time trap, CDZ0304 — matching the runtime `bytes-set`
                // guard) and emit a `Core::BytesOf` carrying the element occurrences (the backend bakes
                // it / builds it on the rope heap). A runtime list source is a later increment (declines
                // cleanly for now — only a visible literal folds). One operand: the list.
                Some(Prim::BytesOf) if args.len() == 1 => lower_bytes_of(db, id, args[0]),
                // `Bytes.len` — FOLD when the operand is a compile-time-visible `Bytes.of` (its byte
                // count is statically known), else emit the runtime `Core::BytesLen` (`bytes-len`). One
                // operand: the bytes. Mirrors `List.len`.
                Some(Prim::BytesLen) if args.len() == 1 => {
                    let operand = args[0];
                    match core_of(db, operand) {
                        Core::BytesOf { elems } => {
                            trace!(target: "rcdzc::fold", node = id.0, len = elems.len(), "Bytes.len folds to a constant (visible Bytes.of literal)");
                            Core::ConstInt(IntValue::from_i64(elems.len() as i64))
                        }
                        Core::Poison(r) => Core::Poison(r),
                        _ => Core::BytesLen { operand },
                    }
                }
                // `String.scalar-len` / `String.byte-len` — FOLD on a constant string to its scalar (char)
                // count / UTF-8 byte count respectively (`collections-and-text.md` §A String Offers Both
                // A Scalar Length And A Byte Length). No escape: the result is an `Int64`. A runtime
                // string declines (the byte-rope length op arrives with the runtime string heap).
                Some(prim @ (Prim::StrScalarLen | Prim::StrByteLen)) if args.len() == 1 => {
                    match core_of(db, args[0]) {
                        Core::ConstStr(s) => {
                            let n = match prim {
                                Prim::StrScalarLen => s.chars().count(),
                                _ => s.len(), // UTF-8 byte length
                            };
                            trace!(target: "rcdzc::fold", node = id.0, ?prim, len = n, "String length folds to a constant");
                            Core::ConstInt(IntValue::from_i64(n as i64))
                        }
                        Core::Poison(r) => Core::Poison(r),
                        _ => Core::Poison(Reject::decline(
                            "a runtime string's length is not yet computed (constant strings only)",
                        )),
                    }
                }
                // `Bytes.at` — the FALLIBLE indexed read `Bytes → Int64 → (Option Int64)`. Mirrors
                // `List.at`: FOLD a visible `Bytes.of` indexed by a constant (in-range → `(Some byte)`,
                // out-of-range/negative → `None`), else emit the runtime `Core::BytesAt`.
                Some(Prim::BytesAt) if args.len() == 2 => lower_bytes_at(db, id, args[0], args[1]),
                // `Bytes.concat` — append two byte sequences. FOLD a constant pair to a single
                // `Core::BytesOf` (its bytes are the concatenation); else emit runtime `Core::BytesConcat`.
                Some(Prim::BytesConcat) if args.len() == 2 => {
                    lower_bytes_concat(db, args[0], args[1])
                }
                // `Bytes.slice` — the FALLIBLE sub-range read. FOLD a constant `Bytes.of` + constant
                // start/len (in range → `(Some (Bytes.of <slice>))`, out → `None`), else `Core::BytesSlice`.
                Some(Prim::BytesSlice) if args.len() == 3 => {
                    lower_bytes_slice(db, id, args[0], args[1], args[2])
                }
                // `Bytes.compact` — content-equal, storage-independent. On a constant it is the identity
                // (same bytes); a runtime value emits `Core::BytesCompact`.
                Some(Prim::BytesCompact) if args.len() == 1 => {
                    let operand = args[0];
                    match core_of(db, operand) {
                        // A constant `Bytes.of` compacts to itself (content-equal); no runtime op.
                        c @ Core::BytesOf { .. } => c,
                        Core::Poison(r) => Core::Poison(r),
                        _ => Core::BytesCompact { operand },
                    }
                }
                // `String.at` — the FALLIBLE scalar-indexed read. FOLD a constant string + constant index
                // to `(Some "<char>")` in range / `None` out (by Unicode SCALAR position, not byte). A
                // runtime string declines (the byte-rope read is a later increment).
                Some(Prim::StrAt) if args.len() == 2 => lower_str_at(db, id, args[0], args[1]),
                // `String.slice` — the FALLIBLE sub-range read by SCALAR offsets `[start, end)`. FOLD a
                // constant string + constant bounds to `(Some "<substr>")` in range / `None` out (reversed,
                // over-long, or negative). A runtime string declines (the byte-rope slice is a later
                // increment).
                Some(Prim::StrSlice) if args.len() == 3 => {
                    lower_str_slice(db, id, args[0], args[1], args[2])
                }
                // `Option.expect` / `Result.expect` — the unwrap-or-trap accessor. `args[0]` is the sum,
                // `args[1]` the message (dropped — the wasm trap is textless). FOLD a constant PRESENT
                // variant to its payload; a runtime sum emits `Core::SumExpect` (disc probe → payload /
                // trap).
                Some(Prim::SumExpect) if args.len() == 2 => lower_sum_expect(db, id, args[0]),
                // `Int64.checked-add` / `checked-mul` — the FALLIBLE arithmetic. FOLD a constant operand
                // pair to `(Some result)` in range / `(None unit)` on overflow; a runtime operand is a
                // later increment (declines cleanly).
                Some(prim @ (Prim::CheckedAdd | Prim::CheckedMul)) if args.len() == 2 => {
                    lower_checked_arith(db, id, prim, args[0], args[1])
                }
                // `Int64.wrapping-add` / `wrapping-mul` — two's-complement wraparound, NEVER trapping. FOLD
                // a constant pair via `wrapping_*`; a runtime operand emits `Core::Arith` (which for a
                // wrapping prim selects the RAW machine op, no overflow guard).
                Some(prim @ (Prim::WrappingAdd | Prim::WrappingMul)) if args.len() == 2 => {
                    lower_wrapping_arith(db, prim, args[0], args[1])
                }
                // `String.concat` — the TOTAL binary join. FOLD two constant strings to their
                // concatenation (the result is another constant `String`). The value form is always NFC,
                // and NFC is NOT closed under concatenation in general (a combining mark starting the RIGHT
                // operand can compose with the base char ending the LEFT one). The reader already NFC-
                // normalizes each `ConstStr`, and concatenation of two ALL-ASCII strings is trivially NFC
                // (ASCII carries no combining marks) — so fold that case, which the compiler's own error-
                // message/name assembly (and every corpus concat case) lives in. A concat where either
                // operand has a non-ASCII scalar DECLINES: re-normalizing the join would need Unicode
                // tables, and the pure compiler core carries no value deps (that arrives with the runtime
                // byte-rope join). A runtime operand likewise declines.
                Some(Prim::StrConcat) if args.len() == 2 => {
                    match (core_of(db, args[0]), core_of(db, args[1])) {
                        (Core::ConstStr(a), Core::ConstStr(b)) if a.is_ascii() && b.is_ascii() => {
                            trace!(target: "rcdzc::fold", node = id.0, "String.concat folds two constant ASCII strings");
                            Core::ConstStr(format!("{a}{b}"))
                        }
                        (Core::Poison(r), _) | (_, Core::Poison(r)) => Core::Poison(r),
                        _ => Core::Poison(Reject::decline(
                            "a string concatenation is only folded for constant ASCII operands (the \
                             normalizing byte-rope join arrives with the runtime string heap)",
                        )),
                    }
                }
                // Every other constructor prim — including the compound-VALUE constructors `TupleNew`/
                // `RecordNew` reached via the shadowable `tuple`/`record` alias names — reduces via
                // `reduce_ctor`, which rewrites `(tuple a b)` → the symbol-headed `((,) a b)` (and
                // `(record …)` → `({} …)`). Lowering the reduced node then goes through the ORDINARY
                // `Resolved::Tuple`/`Record` path — so a constant compound FOLDS (a projection reads the
                // element with no heap) exactly as a symbol-written one does, with no value-ctor special
                // case here. (A type constructor like `(Int 64)` reduces to its module the same way.)
                Some(prim) => {
                    trace!(target: "rcdzc::lower", node = id.0, ?prim, "apply: constructor prim");
                    match crate::eval::reduce_ctor(db, prim, id, &args) {
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
        // A `handle` is REDUCED AWAY (E1c): resolve each enclosed perform to its concrete arm and rewrite
        // the tail-resumptive case to plain code — the perform becomes the arm's resume value, the
        // next-state threads forward (`DESIGN-effects-rcdzc.md` §4.1). `reduce_handle` produces a
        // rewritten BODY occurrence, which we then lower by the ordinary path (so `select` sees only
        // plain `Core`). A case the tail path cannot serve (a non-tail/absent resume, a cross-function or
        // recursive perform) makes `reduce_handle` return `None` → DECLINE (a Todo, never a miscompile).
        Resolved::Handle { init, arms, body } => {
            match crate::effects::reduce_handle(db, init, &arms, body) {
                Some(rewritten) => core_of(db, rewritten),
                None => Core::Poison(Reject::decline(
                    "this handler is not yet reducible by the tail-resumptive fold (cross-function \
                 or non-tail resume arrives in a later increment)",
                )),
            }
        }
        Resolved::Host { .. } => Core::Poison(Reject::decline(
            "a host delegation is not yet lowered (the boundary import arrives in E2)",
        )),
        Resolved::Resume { .. } => Core::Poison(Reject::decline(
            "resume outside a lowered handler arm is not yet realized",
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
    // A COMPOUND scrutinee — a SUM, a TUPLE, or a RECORD — is matched by the DECISION TREE, not the
    // scalar-probe path. A sum dispatches on the discriminant; a tuple has no discriminant, so its match
    // is a chain of `Elem`-path binders / literal tests; a RECORD has neither a discriminant NOR a
    // sanctioned destructuring pattern (a record is read by `(. r field)` projection, not pattern-matched
    // field-by-field — `core-semantics.md §Patterns Compose` lists tuple + constructor patterns, NOT
    // record patterns), so a record match's only patterns are a bare BINDER (binds the whole record) or a
    // WILDCARD — a degenerate match the tree folds to the first covering arm. All go through
    // `lower_match_sum` (the shared decision-tree builder); a scalar scrutinee falls through to the
    // scalar-probe path below.
    if let crate::ty::Ty::Sum { .. } | crate::ty::Ty::Tuple(_) | crate::ty::Ty::Record(_) =
        crate::infer::type_of(db, scrutinee)
    {
        return lower_match_sum(db, scrutinee, arms);
    }
    // Classify each arm into a probe + optional GUARD + body. An arm's pattern may be a GUARDED pattern
    // `(guard <inner-pat> <cond>)` — the inner pattern gives the probe, `<cond>` the guard (a boolean the
    // arm's binder is in scope for, resolve Case 5). A pattern that is not a scalar literal, binder,
    // wildcard, or such a guarded pattern declines the whole match (a compound needs a heap walk).
    let mut probes: Vec<(crate::core::Probe, Option<StructId>, StructId)> = Vec::new();
    for &(pat, body) in arms {
        let (inner_pat, guard) = match db.ast.as_form(pat, "guard") {
            // `(guard <inner-pat> <cond>)` — a guarded pattern.
            Some(g) if g.len() == 2 => (g[0], Some(g[1])),
            Some(_) => {
                return Core::Poison(Reject::coded(
                    Code::Malformed,
                    "a guarded pattern must be (guard <pattern> <cond>)",
                ));
            }
            None => (pat, None),
        };
        match classify_probe(db, inner_pat) {
            Some(p) => probes.push((p, guard, body)),
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
    for (probe, _, _) in &probes {
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
    //      A GUARDED arm does NOT count toward exhaustiveness — its guard may be false, so it covers no
    //      value unconditionally (`core-semantics.md` §Matching Is Exhaustive Or Rejected: "A guard does
    //      NOT count toward exhaustiveness"). So only UNGUARDED arms contribute coverage below.
    let has_wild = probes
        .iter()
        .any(|(p, g, _)| g.is_none() && matches!(p, crate::core::Probe::Wild));
    // A Bool scrutinee's two literals exhaust it. (A definitely-Bool or still-open `Any` scrutinee whose
    // arms are Bool literals — a bare parameter matched with `true`/`false` — is matching over Bool; a
    // definitely-Int scrutinee with a Bool probe already faulted in step (1) and never reaches here.)
    let bool_exhaustive = scrut_ty.agrees_with(&crate::ty::Ty::Bool)
        && probes
            .iter()
            .any(|(p, g, _)| g.is_none() && matches!(p, crate::core::Probe::Bool(true)))
        && probes
            .iter()
            .any(|(p, g, _)| g.is_none() && matches!(p, crate::core::Probe::Bool(false)));
    if !has_wild && !bool_exhaustive {
        return Core::Poison(Reject::coded(
            Code::NonExhaustive,
            "a scalar match must end in a wildcard `_` arm (non-exhaustive)",
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
        _ => None,
    };
    if let Some(sc) = const_scrut {
        let mut foldable = true;
        for (probe, guard, body) in &probes {
            let probe_hit = match &sc {
                GuardFoldScrut::Int(v) => probe_matches_int(probe, v),
                GuardFoldScrut::Bool(b) => probe_matches_bool(probe, *b),
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
    // Runtime scalar scrutinee — it must BE a scalar (a compound needs a heap walk, later).
    if !is_scalar(db, scrutinee) {
        return Core::Poison(Reject::decline(
            "matching a compound value needs a heap walk (not yet built)",
        ));
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
    trace!(target: "rcdzc::lower", scrutinee = scrutinee.0, arms = probes.len(), "match stays runtime (scalar scrutinee → probe chain)");
    Core::Match {
        scrutinee,
        arms: probes
            .into_iter()
            .map(|(probe, guard, body)| crate::core::MatchArm { probe, guard, body })
            .collect(),
    }
}

/// A constant scrutinee value for the guarded-match fold — an integer or a boolean.
enum GuardFoldScrut {
    Int(IntValue),
    Bool(bool),
}

/// Walk a constant-value path from `root` down `steps`, returning the leaf's core if EVERY step lands
/// in a compile-time-constant compound (`Core::SumNew` payloads / `Core::Tuple` elements). This folds a
/// nested payload binder over a constant scrutinee — `(match (Some (Some 5)) ((Some (Some y)) y))`
/// through `[Payload, Payload]` yields the constant `5`, no heap read. `None` if any step hits a runtime
/// value (then the binder emits a runtime `Core::SumPayload` walk).
fn fold_sum_path(db: &mut Db, root: StructId, steps: &[crate::core::PathStep]) -> Option<Core> {
    use crate::core::PathStep;
    let mut cur = root;
    for step in steps {
        cur = match (step, core_of(db, cur)) {
            (PathStep::Payload, Core::SumNew { payloads, .. }) if payloads.len() == 1 => {
                payloads[0]
            }
            (PathStep::Elem(i), Core::Tuple { elems }) => *elems.get(*i)?,
            _ => return None,
        };
    }
    Some(core_of(db, cur))
}

/// Lower a match over a SUM scrutinee to a DECISION TREE (Maranget). Dispatch on the variant
/// DISCRIMINANT at each level; a NESTED pattern shares its outer probe and splits on the inner
/// discriminant, so `(Some (Some x))`, `(Some None)`, `None` test the outer `Some` tag ONCE and only
/// then the inner tag — two tag checks on the deep path, not a linear re-probe per arm
/// (`type-system.md §Patterns Compose`). Exhaustiveness (`type-system.md §A Match Is Exhaustive Against
/// The Sum Type's Variant Set`) is checked at EACH switch: every variant covered OR a default arm; else
/// CDZ0210. A constant sum FOLDS to the selected body (like a scalar match); a runtime sum emits a
/// `Core::MatchSum` tree. A payload binder resolves to a `SumPayload` on its own (resolve Case 6), so an
/// arm carries only its discriminant + continuation.
fn lower_match_sum(db: &mut Db, scrutinee: StructId, arms: &[(StructId, StructId)]) -> Core {
    // The scrutinee must be a COMPOUND the decision tree matches — a SUM (its type gives the root variant
    // set to switch on), a TUPLE (no discriminant; `Elem`-path binders/lit-tests), or a RECORD (no
    // discriminant and no destructure pattern — only a whole-value binder/wildcard arm). A poisoned
    // scrutinee propagates its poison; anything else is a decline (the caller routes only these here).
    let scrut_ty = crate::infer::type_of(db, scrutinee);
    if !matches!(
        scrut_ty,
        crate::ty::Ty::Sum { .. } | crate::ty::Ty::Tuple(_) | crate::ty::Ty::Record(_)
    ) {
        if let Core::Poison(r) = core_of(db, scrutinee) {
            return Core::Poison(r);
        }
        return Core::Poison(Reject::decline(
            "compound match scrutinee is not a sum, tuple, or record",
        ));
    }
    // Build the initial pattern MATRIX: one row per arm, each a `(constraints, body)` where a constraint
    // is `(path, disc)` — "the sub-value at `path` must have discriminant `disc`". A row's constraints
    // start from its top-level pattern (path `[]`) and may nest. A malformed/unsupported pattern declines
    // the whole match (a heap walk / literal-in-sum is a later increment), never a silent match.
    let mut rows: Vec<MatchRow> = Vec::new();
    for &(pat, body) in arms {
        // Peel a `(guard <inner-pattern> <cond>)` wrapper: the arm's discriminant constraints come from
        // the inner pattern, and `<cond>` is carried as the row's guard (gated at the leaf in `build_tree`).
        let (inner_pat, guard) = match db.ast.as_form(pat, "guard") {
            Some(g) if g.len() == 2 => (g[0], Some(g[1])),
            _ => (pat, None),
        };
        // LINEARITY: a pattern is a BINDER POSITION and must bind each name at most once (core-semantics.md
        // §Patterns Compose: "A pattern MUST bind each name at most once … rather than silently shadowing").
        // `(tuple x x)` / `(Some (tuple x x))` binds `x` twice — CDZ0102, the same non-linear-binder error a
        // repeated `def` parameter gets — not a last-wins shadow that makes the first binder's payload
        // unreachable. Checked across the WHOLE arm pattern (nested sub-patterns included).
        if let Err(r) = check_pattern_linear(db, inner_pat) {
            return Core::Poison(r);
        }
        let mut lit_tests = Vec::new();
        match pattern_constraints(db, inner_pat, &scrut_ty, Vec::new(), &mut lit_tests) {
            Ok(constraints) => rows.push(MatchRow {
                constraints,
                lit_tests,
                body,
                guard,
            }),
            Err(r) => return Core::Poison(r),
        }
    }
    // Compile the matrix into a decision tree rooted at the scrutinee (path `[]`, type `scrut_ty`).
    let mut path_types: std::collections::HashMap<Vec<crate::core::PathStep>, crate::ty::Ty> =
        std::collections::HashMap::new();
    path_types.insert(Vec::new(), scrut_ty);
    match build_tree(db, scrutinee, &rows, &path_types) {
        // The whole match reduces to one body (a top-level catch-all, or a fully constant-folded tree).
        Ok(crate::core::SumCont::Leaf(body)) => core_of(db, body),
        // Otherwise the root is a Switch (the usual case) — or a Guarded, when a disc-fold collapsed the
        // root switch to the selected variant's guarded arm. Either way the backend emits it through the
        // uniform `emit_sum_cont`, so carry the root continuation directly.
        Ok(root) => Core::MatchSum {
            scrutinee,
            root: Box::new(root),
        },
        Err(r) => Core::Poison(r),
    }
}

/// One row of the pattern matrix: the discriminant CONSTRAINTS this arm imposes (each a `(path, disc)`),
/// and the arm's body. An empty constraint set is a catch-all (a bare binder / `_` top-level pattern) —
/// it matches regardless of any discriminant. Constraints are ordered outer-to-inner (a shorter path
/// first), which is the order the tree tests them.
#[derive(Clone)]
struct MatchRow {
    constraints: Vec<(Vec<crate::core::PathStep>, u32)>,
    /// LITERAL tests the arm imposes on payload sub-values: each `(path, probe)` requires the scalar at
    /// `path` to equal the literal. A `(Some 0)` pattern adds `([Payload], Int(0))`. Like a guard, a
    /// literal test does NOT count toward exhaustiveness (it may not match — it needs a same-variant
    /// binder/wildcard fall-through), and it is gated once the discriminant constraints are satisfied.
    lit_tests: Vec<(Vec<crate::core::PathStep>, crate::core::Probe)>,
    body: StructId,
    /// A match-arm GUARD `(guard <pattern> <cond>)` — the boolean `<cond>` the arm additionally requires.
    /// `None` for an unguarded arm. Once every discriminant constraint is satisfied (the row reaches a
    /// leaf position in `build_tree`), a guarded row emits `if cond then body else <fall-through>` and
    /// does NOT count toward exhaustiveness; an unguarded row is an unconditional leaf.
    guard: Option<StructId>,
}

/// Reject a match-arm pattern that binds the same name more than once (CDZ0102) — a pattern is a BINDER
/// POSITION and must be LINEAR (`core-semantics.md §Patterns Compose`). Walks the whole pattern collecting
/// BINDER names (a bare non-`_` name that is NOT a variant constructor of a sum in scope, NOR a literal),
/// and faults the second occurrence, anchored there. A `_` binds nothing (may repeat); a variant name
/// (`Some`, `E.Lit`) is a constructor, not a binder; a literal is a value, not a binder. Recurses into
/// tuple/variant sub-patterns and peels a `(guard …)` wrapper. (A non-deduping walk — unlike resolve's
/// binder lookups it must SEE every occurrence to catch the repeat.)
fn check_pattern_linear(db: &mut Db, pat: StructId) -> Result<(), Reject> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    collect_pattern_binders(db, pat, &mut seen)
}

/// The recursive walk behind [`check_pattern_linear`]: insert each binder name into `seen`, faulting a
/// repeat. See that function for the binder-vs-ctor-vs-literal classification.
fn collect_pattern_binders(
    db: &mut Db,
    pat: StructId,
    seen: &mut std::collections::HashSet<String>,
) -> Result<(), Reject> {
    // Peel a guard wrapper — the binder-carrying pattern is the inner one.
    if let Some(g) = db.ast.as_form(pat, "guard")
        && g.len() == 2
    {
        return collect_pattern_binders(db, g[0], seen);
    }
    // A bare atom: a literal binds nothing; a `_` binds nothing; any OTHER bare name is a binder UNLESS it
    // is a nullary variant constructor (`None`, `Sign.Neg`) — a ctor is not a binder. `variant_disc_of`
    // recognizes a ctor value; a name that is not one is a binder.
    if let crate::ast::Struct::Atom(_) = db.ast.get(pat) {
        if matches!(
            crate::resolve::resolved_of(db, pat),
            crate::resolved::Resolved::Int(_) | crate::resolved::Resolved::Bool(_)
        ) {
            return Ok(()); // a literal is not a binder
        }
        if let Some(name) = db.ast.as_name(pat).map(|s| s.to_string()) {
            if name == "_" {
                return Ok(());
            }
            // A bare name that resolves to a variant constructor is a ctor, not a binder.
            if crate::eval::variant_disc_of(db, pat).is_some() {
                return Ok(());
            }
            if !seen.insert(name.clone()) {
                return Err(Reject::coded(
                    Code::NonLinearBinder,
                    format!("pattern binds `{name}` more than once (a pattern must be linear)"),
                )
                .at(pat));
            }
        }
        return Ok(());
    }
    // A compound pattern `(head arg…)` — a variant `(Some p)`, a tuple `(tuple p…)`, or a member `(. S V)`
    // (a nullary ctor, no binders). The head is a ctor/`tuple`/`.` — not a binder; recurse into the args.
    if let crate::ast::Struct::List(children) = db.ast.get(pat) {
        let children = children.clone();
        // A `(. Sum V)` member pattern is a nullary-ctor reference — no binder args.
        if children.first().and_then(|&h| db.ast.as_name(h)) == Some(".") {
            return Ok(());
        }
        // Skip the head (a ctor / `tuple` alias); recurse each argument sub-pattern.
        for &arg in children.iter().skip(1) {
            collect_pattern_binders(db, arg, seen)?;
        }
    }
    Ok(())
}

/// Collect the discriminant constraints a PATTERN imposes on the sub-value at `path` (of type `ty`),
/// appending `(deeper-path, disc)` per variant test. A bare NAME is a binder/wildcard — NO constraint
/// (it matches any value; its binding is resolved independently). A variant pattern `(V arg…)` / bare
/// nullary `V` adds `(path, disc(V))` and recurses into its single payload arg at `path + [Payload]`
/// (a multi-payload variant's payload is a tuple — the arg descends through `Elem` in a later increment).
/// A variant name is distinguished from a binder by RESOLVING it against `ty`'s variant set: `None`
/// against `Option` is the nullary variant (a constraint), `x` is a binder (none). Errs (declines) on a
/// pattern this increment does not compile — a tuple/record destructure, a literal, a wrong-arity ctor.
fn pattern_constraints(
    db: &mut Db,
    pat: StructId,
    ty: &crate::ty::Ty,
    path: Vec<crate::core::PathStep>,
    lit_tests: &mut Vec<(Vec<crate::core::PathStep>, crate::core::Probe)>,
) -> Result<Vec<(Vec<crate::core::PathStep>, u32)>, Reject> {
    // A GUARDED pattern `(guard <inner-pattern> <cond>)` contributes the INNER pattern's discriminant
    // constraints (the guard itself is not a discriminant test — it is carried on the `MatchRow` by
    // `lower_match_sum` and gated at the leaf in `build_tree`). Descend into the inner pattern so a
    // `(guard (Some x) …)` still constrains `[]` to the `Some` disc + binds `x` at `[Payload]`.
    if let Some(g) = db.ast.as_form(pat, "guard") {
        if g.len() != 2 {
            return Err(Reject::coded(
                Code::Malformed,
                "a guarded pattern must be (guard <pattern> <cond>)",
            ));
        }
        return pattern_constraints(db, g[0], ty, path, lit_tests);
    }
    // A LITERAL payload sub-pattern — an integer or boolean atom, NOT a name. `(Some 0)` matches `Some`
    // carrying exactly `0`: the literal refines the match (`core-semantics.md §Pattern Matching`, "nested
    // patterns can combine constructors and literals"). It imposes NO discriminant constraint (a scalar
    // has no variant tag); it adds a LITERAL TEST `(path, probe)` — the sub-value at `path` must EQUAL
    // the literal — gated (like a guard) once the enclosing discriminant is satisfied, with a same-variant
    // fall-through for the non-matching case. The literal's TYPE must AGREE with the sub-value's type at
    // this position: `(tuple true b)` against `(tuple 1 2)` puts a `Bool` literal where the element is
    // `Int64` — a literal-pattern-type mismatch (CDZ0201, core-semantics.md §Equality Is Structural),
    // checked HERE (nested), exactly as the top-level `(match 5 (true 1))` case is, so a nested wrong-type
    // literal does not slip past as a runtime non-match. (`ty` is `Any` for an unsolved position — no
    // check, the not-yet-constrained treatment a projection of `Any` gets.)
    let probe = match crate::resolve::resolved_of(db, pat) {
        crate::resolved::Resolved::Int(v) => {
            Some((crate::core::Probe::Int(v), crate::ty::Ty::int()))
        }
        crate::resolved::Resolved::Bool(b) => {
            Some((crate::core::Probe::Bool(b), crate::ty::Ty::Bool))
        }
        _ => None,
    };
    if let Some((probe, lit_ty)) = probe {
        if !matches!(ty, crate::ty::Ty::Any) && !lit_ty.agrees_with(ty) {
            return Err(Reject::coded(
                Code::Malformed,
                format!(
                    "a {} literal pattern does not match the {} sub-value it is matched against",
                    lit_ty.render_name(),
                    ty.render_name()
                ),
            ));
        }
        lit_tests.push((path, probe));
        return Ok(Vec::new());
    }
    // A bare NAME: either a NULLARY VARIANT of this sum (`None`) or a binder/wildcard. Resolve it against
    // the sum's variant set — a name that IS a variant contributes that discriminant (no payload to
    // recurse into); any other bare name binds and contributes nothing.
    if let Some(name) = db.ast.as_name(pat) {
        let name = name.to_string();
        if name != "_"
            && let Some(disc) = variant_disc_by_name(db, ty, &name)
        {
            return Ok(vec![(path, disc)]);
        }
        return Ok(Vec::new()); // a binder / wildcard — no constraint
    }
    // A TUPLE pattern `(tuple p0 p1…)` at `path` — a variant's tuple PAYLOAD, destructured positionally
    // (core-semantics.md §Patterns Compose: a tagged value carrying a tuple is one nested pattern). A
    // tuple has no discriminant, so it imposes NO constraint of its own; each element sub-pattern
    // descends at `path + [Elem(i)]`, of the tuple element's type. (Reached only inside a variant
    // payload — the top-level scrutinee is a sum, so `pattern_constraints` is entered on a variant.)
    if is_tuple_pattern(db, pat) {
        let elems: Vec<StructId> = db
            .ast
            .as_form(pat, "tuple")
            .or_else(|| db.ast.as_ctor_form(pat, "tuple"))
            .unwrap_or(&[])
            .to_vec();
        // The payload MUST be a tuple, and the pattern's ARITY must match it — a tuple pattern against a
        // non-tuple payload, or one naming the wrong number of elements (`(tuple a b c)` against a
        // 2-tuple), is an ill-typed destructure the compiler REJECTS (CDZ0201), never a silent match on a
        // wrong shape. (type-system.md: two tuples agree only when their arities are identical.)
        let elem_tys: &[crate::ty::Ty] = match ty {
            crate::ty::Ty::Tuple(ts) if ts.len() == elems.len() => ts,
            // `Any` payload (an unsolved/unknown type) can't be arity-checked here — descend permissively
            // (each element `Any`), the same not-yet-constrained treatment a projection of an `Any` gets.
            crate::ty::Ty::Any => {
                let mut out = Vec::new();
                for (i, &elem) in elems.iter().enumerate() {
                    let mut deeper = path.clone();
                    deeper.push(crate::core::PathStep::Elem(i));
                    out.extend(pattern_constraints(
                        db,
                        elem,
                        &crate::ty::Ty::Any,
                        deeper,
                        lit_tests,
                    )?);
                }
                return Ok(out);
            }
            _ => {
                return Err(Reject::coded(
                    Code::Malformed,
                    format!(
                        "a tuple pattern of {} element(s) does not match the payload type {}",
                        elems.len(),
                        ty.render_name()
                    ),
                ));
            }
        };
        let mut out = Vec::new();
        for (i, &elem) in elems.iter().enumerate() {
            let mut deeper = path.clone();
            deeper.push(crate::core::PathStep::Elem(i));
            out.extend(pattern_constraints(
                db,
                elem,
                &elem_tys[i],
                deeper,
                lit_tests,
            )?);
        }
        return Ok(out);
    }
    // A compound pattern. Its head is the variant CONSTRUCTOR — a member `(. Sum V)` or a bare variant
    // name — and the remaining children are payload sub-patterns.
    let (head, args): (StructId, Vec<StructId>) = match db.ast.get(pat) {
        crate::ast::Struct::List(children) => match children.first().copied() {
            // A bare member `(. Sum V)` used as a whole pattern — the ctor, no payload args.
            Some(first) if db.ast.as_name(first) == Some(".") => (pat, Vec::new()),
            Some(first) => (first, children[1..].to_vec()),
            None => return Err(Reject::decline("an empty sum match pattern")),
        },
        crate::ast::Struct::Atom(_) => {
            return Err(Reject::decline("a malformed sum match pattern"));
        }
    };
    let Some(disc) = crate::eval::variant_disc_of(db, head) else {
        return Err(Reject::decline(
            "a sum match pattern head is not a variant constructor",
        ));
    };
    // TYPE-CHECK the pattern's constructor against the SCRUTINEE's sum type: the variant must belong to
    // the sum being matched, not merely be SOME sum's variant with the right name. A `Some`/`U.A` pattern
    // over a `T` scrutinee resolves to a valid discriminant of Option/U, but that variant is not T's — a
    // type confusion that would bind the payload under the wrong type (a wrong VALUE, or an INVALID WASM
    // component when the payload widths differ). Sum identity is by DECLARATION OCCURRENCE (`ty.rs`
    // §Two sums are the SAME type iff their `decl` agree), so compare the pattern ctor's owning `decl`
    // against the scrutinee `ty`'s `decl` — a mismatch is CDZ0203, the same type error `(: 5 Bool)` gets.
    // (A bare nullary-variant name took the `variant_disc_by_name` path above, which is already scoped to
    // this sum's declaration, so only a COMPOUND ctor pattern reaches here needing the check.)
    if let crate::ty::Ty::Sum {
        decl: scrut_decl, ..
    } = ty
        && crate::eval::variant_owner_decl(db, head) != Some(*scrut_decl)
    {
        return Err(Reject::coded(
            Code::TypeMismatch,
            format!(
                "this variant pattern is not a variant of the matched type {}",
                ty.render_name()
            ),
        ));
    }
    let mut out = vec![(path.clone(), disc)];
    // Recurse into the payload. A single-payload variant `(Some p)` descends into `p` at `path +
    // [Payload]`; the payload's TYPE is the variant's payload type at this instantiation, so a nested
    // variant name there resolves against the right sum. A NULLARY variant pattern `(None)`/bare `None`
    // has no payload arg — nothing to recurse.
    match args.len() {
        0 => {}
        1 => {
            let payload_ty = crate::infer::payload_ty_at_instantiation(db, head, ty)
                .unwrap_or(crate::ty::Ty::Any);
            let mut deeper = path;
            deeper.push(crate::core::PathStep::Payload);
            let sub = pattern_constraints(db, args[0], &payload_ty, deeper, lit_tests)?;
            out.extend(sub);
        }
        // A MULTI-PAYLOAD variant pattern `(Cons h t)` is sugar for the single-tuple-payload form `(Cons
        // (tuple h t))`: the payloads are boxed as ONE tuple handle (`lower_sum_new` / the `SumNew`
        // backend), so `payload_ty_at_instantiation` reports the payload as a `Ty::Tuple`, and each arg
        // destructures a tuple ELEMENT at `path + [Payload, Elem(i)]` — exactly the descent the explicit
        // `(tuple …)` payload pattern takes.
        _ => {
            let payload_ty = crate::infer::payload_ty_at_instantiation(db, head, ty)
                .unwrap_or(crate::ty::Ty::Any);
            // The pattern's payload ARITY must match the variant's declared payload count — `(Mk a b c)`
            // against a 2-payload `Mk` names a nonexistent third element (it would read past the payload
            // tuple and bind `c` under an `Any`/wrong type — a wrong value, or invalid wasm). REJECT it
            // (CDZ0201), the same arity check the explicit `(tuple …)` payload pattern enforces above. An
            // `Any` payload (unsolved) can't be arity-checked — descend permissively (each `Any`).
            let elem_tys: Vec<crate::ty::Ty> = match &payload_ty {
                crate::ty::Ty::Tuple(ts) if ts.len() == args.len() => ts.to_vec(),
                crate::ty::Ty::Tuple(ts) => {
                    return Err(Reject::coded(
                        Code::Malformed,
                        format!(
                            "this variant pattern binds {} payload(s), but the variant carries {}",
                            args.len(),
                            ts.len()
                        ),
                    ));
                }
                crate::ty::Ty::Any => vec![crate::ty::Ty::Any; args.len()],
                // A non-tuple payload type under a multi-arg pattern is an arity error too (a single-payload
                // variant matched with several binders).
                _ => {
                    return Err(Reject::coded(
                        Code::Malformed,
                        format!(
                            "this variant pattern binds {} payloads, but the variant's payload is {}",
                            args.len(),
                            payload_ty.render_name()
                        ),
                    ));
                }
            };
            let mut payload_path = path;
            payload_path.push(crate::core::PathStep::Payload);
            for (i, (&arg, elem_ty)) in args.iter().zip(elem_tys.iter()).enumerate() {
                let mut deeper = payload_path.clone();
                deeper.push(crate::core::PathStep::Elem(i));
                let sub = pattern_constraints(db, arg, elem_ty, deeper, lit_tests)?;
                out.extend(sub);
            }
        }
    }
    Ok(out)
}

/// Whether `id` is a tuple PATTERN `(tuple …)` — a `tuple` NAME head (the alias the reader keeps in a
/// pattern) or the `"tuple"` string-literal primitive. Mirrors `resolve::is_tuple_pattern` (kept local
/// so lower does not depend on resolve's private helpers).
fn is_tuple_pattern(db: &Db, id: StructId) -> bool {
    db.ast.as_form(id, "tuple").is_some() || db.ast.head_ctor(id) == Some("tuple")
}

/// The discriminant of the variant named `name` in the sum `ty`, or `None` if `ty` is not a sum or has
/// no such variant. This is what distinguishes a bare NULLARY-VARIANT pattern (`None` against `Option`)
/// from a binder (`x`) — the name is looked up in the scrutinee sum's own declaration (occurrence-keyed,
/// so a same-named variant in another sum does not leak in).
fn variant_disc_by_name(db: &mut Db, ty: &crate::ty::Ty, name: &str) -> Option<u32> {
    let decl = match ty {
        crate::ty::Ty::Sum { decl, .. } => *decl,
        _ => return None,
    };
    let t = db.type_decl_by_occ(decl)?;
    t.variants
        .iter()
        .position(|v| v.name == name)
        .map(|i| i as u32)
}

/// A map from an access PATH to the solved TYPE of the sub-value there — populated as the tree descends
/// (the root `[]` maps to the scrutinee type; entering a variant arm at `switch_path` extends it with
/// that variant's payload type at `switch_path + [Payload]`). Keyed per-branch (not global), because the
/// SAME path under different parent variants has different types (`Result`'s `[Payload]` is `a` in the
/// `Ok` arm, `e` in the `Err` arm) — a global map would collide; a branch-local one is always consistent.
type PathTypes = std::collections::HashMap<Vec<crate::core::PathStep>, crate::ty::Ty>;

/// Compile a pattern MATRIX (`rows`) into a decision-tree CONTINUATION for the value at `scrutinee`. If
/// the FIRST row is a catch-all (no constraints), it matches unconditionally → its body is the leaf (later
/// rows unreachable). Otherwise switch on the discriminant at the SHALLOWEST path any row constrains:
/// gather the discs tested there in source order, and for each build a specialized sub-matrix — rows
/// constraining that path with this disc (constraint removed) PLUS rows not constraining it (they match
/// any disc, flowing into every arm) — then recurse. A default arm (`disc: None`) covers the rows that
/// don't constrain the switch path. Exhaustiveness is checked at EACH switch (every variant tested, or a
/// default). A constant sub-value FOLDS to the matching arm's continuation (no runtime switch).
fn build_tree(
    db: &mut Db,
    scrutinee: StructId,
    rows: &[MatchRow],
    path_types: &PathTypes,
) -> Result<crate::core::SumCont, Reject> {
    // The FIRST row whose discriminant constraints are all satisfied (empty) is at a LEAF position. If it
    // is UNGUARDED it matches unconditionally → its body is the leaf (later rows unreachable). If it is
    // GUARDED, it fires only when its guard holds; on a false guard control FALLS THROUGH to the rest of
    // this sub-matrix (`build_tree` of the remaining rows) — the per-variant fall-through a guarded arm
    // needs. A guarded leaf does NOT terminate the matrix, so the fall-through must independently be
    // exhaustive (an unguarded arm of the same variant, or the default, below it).
    match rows.first() {
        None => {
            return Err(Reject::coded(
                Code::NonExhaustive,
                "a sum match must cover every variant or end in a wildcard `_` (non-exhaustive)",
            ));
        }
        // A row whose discriminant constraints are all satisfied but that still carries LITERAL TESTS is
        // at a leaf gated by those tests: `(Some 0)` reaches here (after the `Some` switch) with a pending
        // `([Payload], Int(0))`. Emit a `LitTest` — test the sub-value at `path` against the literal; on a
        // match, CONTINUE with that test dropped (further lit-tests / the guard / the body); on a MISMATCH,
        // FALL THROUGH to the remaining rows (the same-variant binding arm `(Some k)`), exactly as a guard
        // threads its `else`. A literal test does NOT count toward exhaustiveness — the fall-through must
        // cover the variant. FOLD when the tested sub-value is a compile-time constant (a constant
        // scrutinee): a matching literal drops the test, a non-matching one skips to the fall-through
        // WITHOUT emitting the body — the constant-match half of corpus "nested patterns with literals".
        Some(row) if row.constraints.is_empty() && !row.lit_tests.is_empty() => {
            let (lit_path, probe) = row.lit_tests[0].clone();
            // The row with this first literal test consumed (its other tests / guard / body remain).
            let mut matched_row = row.clone();
            matched_row.lit_tests.remove(0);
            let mut matched_rows = vec![matched_row];
            matched_rows.extend_from_slice(&rows[1..]);
            // FOLD against a constant sub-value.
            if let Some(c) = const_at_path(db, scrutinee, &lit_path) {
                let hit = match (&probe, &c) {
                    (crate::core::Probe::Int(v), Core::ConstInt(cv)) => v.eq_value(cv),
                    (crate::core::Probe::Bool(b), Core::ConstBool(cb)) => b == cb,
                    // A non-constant / type-mismatched sub-value can't fold — emit the runtime test.
                    _ => {
                        return build_lit_test(
                            db,
                            scrutinee,
                            lit_path,
                            probe,
                            &matched_rows,
                            &rows[1..],
                            path_types,
                        );
                    }
                };
                if hit {
                    return build_tree(db, scrutinee, &matched_rows, path_types);
                } else {
                    return build_tree(db, scrutinee, &rows[1..], path_types);
                }
            }
            return build_lit_test(
                db,
                scrutinee,
                lit_path,
                probe,
                &matched_rows,
                &rows[1..],
                path_types,
            );
        }
        Some(row) if row.constraints.is_empty() && row.guard.is_none() => {
            return Ok(crate::core::SumCont::Leaf(row.body));
        }
        Some(row) if row.constraints.is_empty() => {
            // A GUARDED leaf: `if guard then body else <fall-through over the remaining rows>`.
            let cond = row.guard.expect("matched the guarded arm");
            let body = row.body;
            // FOLD the guard when it is a compile-time-constant bool (a constant scrutinee makes its
            // payload binders constant, so `(> x 0)` over `x = 0` folds to `false`). A true guard SELECTS
            // the body directly; a false guard SKIPS to the fall-through tree — WITHOUT lowering the body.
            // This shields a body that would TRAP when folded (`(/ 10 x)` at `x = 0` → CDZ0304) from being
            // evaluated when its guard is false: the guard short-circuits the body exactly as `and`/`or`
            // and `if` shield an untaken branch (core-semantics.md §Boolean Connectives Short-Circuit).
            // Without this fold, a false-guarded arm's trapping body raised a SPURIOUS CDZ0304 for an arm
            // that never runs. A guard reading a RUNTIME value does not fold → the runtime `Guarded` cont.
            match core_of(db, cond) {
                Core::ConstBool(true) => return Ok(crate::core::SumCont::Leaf(body)),
                Core::ConstBool(false) => return build_tree(db, scrutinee, &rows[1..], path_types),
                _ => {}
            }
            let els = build_tree(db, scrutinee, &rows[1..], path_types)?;
            return Ok(crate::core::SumCont::Guarded {
                cond,
                body,
                els: Box::new(els),
            });
        }
        _ => {}
    }
    // Pick the SWITCH path — the shallowest path any row constrains (outer patterns first, so the outer
    // probe is shared). Its TYPE gives the variant set for exhaustiveness + recursion. Read from
    // `path_types` (populated as sum-variant arms descend), else COMPUTE it by walking the path from the
    // scrutinee's own type — a `Ty::Tuple` element indexes at `Elem(i)`, so a sum nested in a TUPLE element
    // (`(match (tuple a b) ((tuple (E.Lit x) …)…))`, switch path `[Elem(0)]`) resolves even though no
    // sum-payload descent seeded it. (`path_types` still wins where present — a variant payload's
    // instantiated type is more precise than a raw type-walk.)
    let switch_path = shallowest_path(rows);
    let sub_ty = match path_types.get(&switch_path) {
        Some(t) => t.clone(),
        None => match type_at_path(db, scrutinee, &switch_path) {
            Some(t) => t,
            None => {
                return Err(Reject::decline(
                    "compound match switch path has no solved type",
                ));
            }
        },
    };
    let (decl, variant_count) = match &sub_ty {
        crate::ty::Ty::Sum { decl, .. } => match db.type_decl_by_occ(*decl) {
            Some(t) => (*decl, t.variants.len()),
            None => return Err(Reject::decline("sum match sub-value has no declaration")),
        },
        _ => {
            return Err(Reject::decline(
                "sum match dispatches on a non-sum sub-value",
            ));
        }
    };
    // Partition the matrix by the disc each row tests at `switch_path` in ONE pass (was one O(N) scan per
    // arm via `specialize` → O(N²) over N arms; the `tested.contains` loop was O(N²) too). Each row either
    // tests `switch_path` with some disc `d` (it belongs ONLY to arm `d`, with that now-satisfied
    // constraint dropped) or does NOT test it (a DEFAULT row — it flows into EVERY arm AND the default
    // arm, unchanged). Rows keep their source index so an arm's sub-matrix preserves source order (arm
    // priority = first-matching-row) when disc rows and default rows interleave.
    let mut tested: Vec<u32> = Vec::new();
    let mut disc_rows: crate::fxhash::FxHashMap<u32, Vec<(usize, MatchRow)>> = Default::default();
    let mut default_rows: Vec<(usize, MatchRow)> = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        match row.constraints.iter().find(|(p, _)| *p == switch_path) {
            Some((_, d)) => {
                let d = *d;
                let bucket = disc_rows.entry(d).or_insert_with(|| {
                    tested.push(d);
                    Vec::new()
                });
                bucket.push((
                    i,
                    MatchRow {
                        // Drop the now-satisfied `switch_path` constraint (control is in this arm).
                        constraints: row
                            .constraints
                            .iter()
                            .filter(|(p, _)| *p != switch_path)
                            .cloned()
                            .collect(),
                        lit_tests: row.lit_tests.clone(),
                        body: row.body,
                        guard: row.guard,
                    },
                ));
            }
            None => default_rows.push((
                i,
                MatchRow {
                    constraints: row.constraints.clone(),
                    lit_tests: row.lit_tests.clone(),
                    body: row.body,
                    guard: row.guard,
                },
            )),
        }
    }
    // The switched sub-value's STATICALLY-KNOWN discriminant, if any — a `SumNew` core at `switch_path`
    // has a fixed disc EVEN when its payload is a runtime value (`(Some n)` is `SumNew{Some, [n]}`: the
    // `Some` tag is known, only `n` is runtime). It drives the FOLD below (pick the known arm, no runtime
    // switch). It does NOT relax exhaustiveness: `core-semantics.md §Matching Is Exhaustive Or Rejected`
    // (corpus 02 "a sum match missing a variant is non-exhaustive EVEN when the scrutinee is the covered
    // one") makes exhaustiveness a property of the ARM SET against the TYPE's variant set, never of which
    // variant the scrutinee holds — a value-driven shortcut that skips the check because the constant hit
    // a present arm is exactly what that case forbids.
    let known_disc = match const_at_path(db, scrutinee, &switch_path) {
        Some(Core::SumNew { disc, .. }) => Some(disc),
        _ => None,
    };
    // Exhaustiveness: every variant tested, or a default (wildcard/binder) present — else CDZ0210. Against
    // the TYPE's variant set, independent of `known_disc` (see above).
    let has_default = !default_rows.is_empty();
    if !has_default && tested.len() < variant_count {
        return Err(Reject::coded(
            Code::NonExhaustive,
            "a sum match must cover every variant or end in a wildcard `_` (non-exhaustive)",
        ));
    }
    // One arm per tested discriminant, then the default arm (if any). Each arm's sub-matrix merges its
    // disc rows with the default rows by source index (both already ascending), recursing under a
    // `path_types` extended with THIS variant's payload type at `switch_path+[Payload]`.
    let mut sum_arms: Vec<crate::core::SumArm> = Vec::new();
    for &d in &tested {
        let own = disc_rows.remove(&d).unwrap_or_default();
        let sub_rows = merge_rows(own, &default_rows);
        let child_types = extend_path_types(db, path_types, &switch_path, &sub_ty, decl, d);
        let cont = build_tree(db, scrutinee, &sub_rows, &child_types)?;
        sum_arms.push(crate::core::SumArm {
            disc: Some(d),
            cont,
        });
    }
    if has_default {
        // The default arm switches on nothing new at `switch_path` — its rows only reach paths they
        // already constrain (all in `path_types`), so no extension is needed.
        let sub_rows: Vec<MatchRow> = default_rows.into_iter().map(|(_, r)| r).collect();
        let cont = build_tree(db, scrutinee, &sub_rows, path_types)?;
        sum_arms.push(crate::core::SumArm { disc: None, cont });
    }
    // FOLD when the switched sub-value's discriminant is STATICALLY KNOWN (a `SumNew` core — its tag is
    // fixed even if its payload is runtime): pick the matching arm's continuation directly, no runtime
    // disc switch. `(match (Some n) …)` folds to the `Some` arm (whose body may still test the runtime
    // payload `n` via a `LitTest`). A scrutinee whose disc is NOT known keeps the runtime `Switch`.
    if let Some(disc) = known_disc {
        for arm in &sum_arms {
            if arm.disc.is_none() || arm.disc == Some(disc) {
                trace!(target: "rcdzc::fold", "sum match folds to a selected arm (known discriminant)");
                return Ok(arm.cont.clone());
            }
        }
    }
    trace!(target: "rcdzc::lower", scrutinee = scrutinee.0, depth = switch_path.len(), arms = sum_arms.len(), "sum switch (decision-tree node)");
    Ok(crate::core::SumCont::Switch {
        path: switch_path,
        arms: sum_arms,
    })
}

/// Build a runtime `SumCont::LitTest` node: test the sub-value at `lit_path` against `probe`; on a match
/// continue with `matched_rows` (this arm with the test consumed, then the rest of the sub-matrix), on a
/// mismatch fall through to `else_rows`. Both sub-trees are compiled by `build_tree`. Split out of
/// `build_tree` so the constant-fold path (a matching/non-matching constant sub-value) and the runtime
/// path share one construction; the `then_`/`els` recursion is what lets several literal tests on one arm
/// nest and a fall-through reach the same-variant binding arm.
fn build_lit_test(
    db: &mut Db,
    scrutinee: StructId,
    lit_path: Vec<crate::core::PathStep>,
    probe: crate::core::Probe,
    matched_rows: &[MatchRow],
    else_rows: &[MatchRow],
    path_types: &PathTypes,
) -> Result<crate::core::SumCont, Reject> {
    let then_ = build_tree(db, scrutinee, matched_rows, path_types)?;
    let els = build_tree(db, scrutinee, else_rows, path_types)?;
    Ok(crate::core::SumCont::LitTest {
        path: lit_path,
        probe,
        then_: Box::new(then_),
        els: Box::new(els),
    })
}

/// The solved TYPE of the sub-value at `path` from `scrutinee`, computed by walking the scrutinee's own
/// type: an `Elem(i)` step indexes a `Ty::Tuple`'s i-th element; a `Payload` step descends a sum
/// variant's payload (via the head recorded... but a raw type-walk cannot know WHICH variant a `Payload`
/// step refers to, so `Payload` is only resolvable through `extend_path_types`' instantiation — this
/// fallback handles the `Elem`-only paths a TUPLE-scrutinee match produces, where `path_types` was not
/// seeded). Returns `None` for a `Payload` step (deferred to `path_types`) or an out-of-range/ill-typed
/// index. Used as the fallback when `path_types` has no entry — a sum nested in a tuple element.
fn type_at_path(
    db: &mut Db,
    scrutinee: StructId,
    path: &[crate::core::PathStep],
) -> Option<crate::ty::Ty> {
    let mut cur = crate::infer::type_of(db, scrutinee);
    for step in path {
        cur = match step {
            crate::core::PathStep::Elem(i) => match &cur {
                crate::ty::Ty::Tuple(elems) => elems.get(*i)?.clone(),
                _ => return None,
            },
            // A `Payload` step's target type needs the variant's instantiation (`extend_path_types`);
            // a raw type-walk cannot supply it, so this fallback does not resolve a `Payload`-bearing path
            // (those paths are always seeded in `path_types` by the sum-variant descent).
            crate::core::PathStep::Payload => return None,
        };
    }
    Some(cur)
}

/// Extend `path_types` for the arm switching on variant `disc` at `switch_path` (a sum of type `sub_ty`,
/// declaration `decl`): the sub-value at `switch_path + [Payload]` has the type of THAT variant's payload
/// at `sub_ty`'s instantiation. Read via the variant's constructor record (its `(meta t)` scheme unified
/// against `sub_ty`), so a generic sum's payload is instantiated (`Ok`'s payload in `Result Int Str` is
/// `Int`). A nullary variant has no payload — no extension. The map is CLONED so sibling arms don't share.
fn extend_path_types(
    db: &mut Db,
    path_types: &PathTypes,
    switch_path: &[crate::core::PathStep],
    sub_ty: &crate::ty::Ty,
    decl: StructId,
    disc: u32,
) -> PathTypes {
    let mut out = path_types.clone();
    // The variant's constructor occurrence — via the synthesized sum record's variant field, which
    // carries the `(meta t)` scheme `payload_ty_at_instantiation` reads. (The declaration name occurrence
    // does not resolve to a scheme; the synthesized ctor field does.)
    // The variant's constructor occurrence — cached on the variant at synthesis time (O(1)), rather than
    // re-scanning the sum record's variant fields by name per arm (that was O(V) per arm → O(V²) overall).
    let ctor = db
        .type_decl_by_occ(decl)
        .and_then(|t| t.variants.get(disc as usize))
        .and_then(|v| v.ctor);
    if let Some(ctor) = ctor
        && let Some(payload_ty) = crate::infer::payload_ty_at_instantiation(db, ctor, sub_ty)
    {
        let mut child = switch_path.to_vec();
        child.push(crate::core::PathStep::Payload);
        // A MULTI-payload variant's payload is a `Ty::Tuple` (its payloads boxed as one tuple handle);
        // also register each tuple ELEMENT's type at `switch_path + [Payload, Elem(i)]` so a nested switch
        // (a variant pattern in a payload position — `(Cons h (Cons h2 rest))`) resolves its sub-value's
        // type. A single-payload variant's payload is registered at `[Payload]` alone, unchanged.
        if let crate::ty::Ty::Tuple(elems) = &payload_ty {
            for (i, elem_ty) in elems.iter().enumerate() {
                let mut elem_path = child.clone();
                elem_path.push(crate::core::PathStep::Elem(i));
                out.insert(elem_path, elem_ty.clone());
            }
        }
        out.insert(child, payload_ty);
    }
    out
}

/// The shallowest (shortest, then by `path_cmp`) path any row constrains — the switch site.
fn shallowest_path(rows: &[MatchRow]) -> Vec<crate::core::PathStep> {
    rows.iter()
        .flat_map(|r| r.constraints.iter().map(|(p, _)| p.clone()))
        .min_by(|a, b| a.len().cmp(&b.len()).then_with(|| path_cmp(a, b)))
        .unwrap_or_default()
}

/// A total order on paths for a deterministic switch choice (Payload < Elem, Elem by index).
fn path_cmp(a: &[crate::core::PathStep], b: &[crate::core::PathStep]) -> std::cmp::Ordering {
    use crate::core::PathStep::{Elem, Payload};
    for (x, y) in a.iter().zip(b.iter()) {
        let o = match (x, y) {
            (Payload, Payload) => std::cmp::Ordering::Equal,
            (Payload, Elem(_)) => std::cmp::Ordering::Less,
            (Elem(_), Payload) => std::cmp::Ordering::Greater,
            (Elem(i), Elem(j)) => i.cmp(j),
        };
        if o != std::cmp::Ordering::Equal {
            return o;
        }
    }
    a.len().cmp(&b.len())
}

/// Merge an arm's OWN disc rows with the shared DEFAULT rows into one sub-matrix, preserving SOURCE order
/// (arm priority = first-matching-row). Both inputs are `(source_index, row)` already ascending by index
/// (the partition in `build_tree` pushed them in row order), so this is a linear two-way merge — no sort.
/// A default row is cloned into each arm it flows into; `own` rows are moved (each belongs to one arm).
fn merge_rows(own: Vec<(usize, MatchRow)>, defaults: &[(usize, MatchRow)]) -> Vec<MatchRow> {
    let mut out = Vec::with_capacity(own.len() + defaults.len());
    let mut oi = own.into_iter().peekable();
    let mut di = defaults.iter().peekable();
    loop {
        match (oi.peek(), di.peek()) {
            (Some((oidx, _)), Some((didx, _))) => {
                if oidx <= didx {
                    out.push(oi.next().unwrap().1);
                } else {
                    out.push(di.next().unwrap().1.clone());
                }
            }
            (Some(_), None) => out.push(oi.next().unwrap().1),
            (None, Some(_)) => out.push(di.next().unwrap().1.clone()),
            (None, None) => break,
        }
    }
    out
}

/// The compile-time-constant `Core` at `path` from `scrutinee`, if every step lands in a constant
/// compound (`SumNew` payload / `Tuple` element) — else `None` (a runtime step). Drives the constant fold
/// at each switch. Mirrors `fold_sum_path` but starts from an occurrence and returns the leaf core.
fn const_at_path(db: &mut Db, scrutinee: StructId, path: &[crate::core::PathStep]) -> Option<Core> {
    use crate::core::PathStep;
    let mut cur = scrutinee;
    for step in path {
        cur = match (step, core_of(db, cur)) {
            (PathStep::Payload, Core::SumNew { payloads, .. }) if payloads.len() == 1 => {
                payloads[0]
            }
            (PathStep::Elem(i), Core::Tuple { elems }) => *elems.get(*i)?,
            _ => return None,
        };
    }
    Some(core_of(db, cur))
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
    // A COMPOUND (tuple/record) binding that is ONLY ever PROJECTED — never used as a whole value —
    // need not be built on the heap at all: each projection folds straight through to the element's own
    // computation (a param `local.get`, a nested op, …), which is far cheaper than an `arr-alloc` +
    // per-field `box`/`arr-set` + `arr-get`/`get` + `drop` round-trip. Keeping it would build a heap
    // value only to read it back (or, when the projections fold, to drop it dead). So a projection-only
    // compound is NOT kept — it folds. A compound that ESCAPES as a whole (returned, passed as an arg,
    // nested into another compound) genuinely needs materialization and IS kept. (A non-compound
    // runtime binding — a shared scalar computation — keeps the multi-use rule below: naming avoids a
    // recompute.)
    if is_compound_value(db, init) && !binding_escapes_whole(db, init, scope) {
        return false;
    }
    // Count references to this binding across its scope. Naming is worth it only at >= 2 uses.
    let mut n = 0;
    for &region in scope {
        n += uses_in(db, region, init);
    }
    n >= 2
}

/// Whether the node at `init` lowers to a COMPOUND heap value — a tuple or a record. These are the
/// values whose only-projected form folds through rather than being built on the heap.
fn is_compound_value(db: &mut Db, init: StructId) -> bool {
    matches!(core_of(db, init), Core::Tuple { .. } | Core::Record { .. })
}

/// Whether the binding `init` is used as a WHOLE VALUE anywhere in `scope` — i.e. referenced in any
/// position OTHER than as the operand of a projection (`(. c i)` / `(. c field)`). A whole-value use
/// (returned as the body's result, passed as a call argument, nested as an element of another compound,
/// annotated, …) means the compound must actually exist at run time, so it is materialized on the heap.
/// If every reference is a projection, the compound never needs to exist — each field read folds to the
/// element directly — so this returns `false` and the binding is not kept. Mirrors the value-flow
/// discipline `binding_escapes` uses in selection for Perceus drops, at the resolved layer.
fn binding_escapes_whole(db: &mut Db, init: StructId, scope: &[StructId]) -> bool {
    scope
        .iter()
        .any(|&region| ref_escapes_whole(db, region, init))
}

/// Whether a reference to `init` appears as a WHOLE-VALUE use within `node` (not merely as a projection
/// operand). Recurses every sub-position; at a projection `(. operand i)`, a reference that IS the
/// `operand` is a projection (does not escape), but the operand is still recursed in case it nests a
/// whole-value use deeper (e.g. `(. (f c) 0)` uses `c` wholly as `f`'s argument).
fn ref_escapes_whole(db: &mut Db, node: StructId, init: StructId) -> bool {
    match resolved_of(db, node) {
        // A bare reference to the binding, in a non-projection position → a whole-value use.
        Resolved::Ref { value } => value == init,
        // A projection: if its operand is a DIRECT ref to `init`, that is a projection use (does not
        // escape). Otherwise recurse the operand (it may nest a whole-value use).
        Resolved::Proj { operand, .. } | Resolved::Member { operand, .. } => {
            if matches!(resolved_of(db, operand), Resolved::Ref { value } if value == init) {
                false
            } else {
                ref_escapes_whole(db, operand, init)
            }
        }
        Resolved::If { cond, then_, else_ } => {
            ref_escapes_whole(db, cond, init)
                || ref_escapes_whole(db, then_, init)
                || ref_escapes_whole(db, else_, init)
        }
        Resolved::And { lhs, rhs, .. } => {
            ref_escapes_whole(db, lhs, init) || ref_escapes_whole(db, rhs, init)
        }
        Resolved::Not { operand } => ref_escapes_whole(db, operand, init),
        Resolved::Let { bindings, body } => {
            bindings
                .iter()
                .any(|(_, v)| ref_escapes_whole(db, *v, init))
                || ref_escapes_whole(db, body, init)
        }
        Resolved::Record { fields } => fields.values().any(|&v| ref_escapes_whole(db, v, init)),
        Resolved::Tuple { elems } | Resolved::List { elems } => {
            elems.iter().any(|&e| ref_escapes_whole(db, e, init))
        }
        Resolved::Annot { expr, .. } => ref_escapes_whole(db, expr, init),
        Resolved::Apply { head, args } => {
            ref_escapes_whole(db, head, init)
                || args.iter().any(|&a| ref_escapes_whole(db, a, init))
        }
        Resolved::Match { scrutinee, arms } => {
            ref_escapes_whole(db, scrutinee, init)
                || arms.iter().any(|(_, b)| ref_escapes_whole(db, *b, init))
        }
        // Effect control forms: a reference to `init` as a whole value can appear in a handler's init,
        // any arm body, a resumption's value/next-state, or the handled/delegated body — recurse each.
        Resolved::Handle {
            init: seed,
            arms,
            body,
        } => {
            ref_escapes_whole(db, seed, init)
                || arms.iter().any(|a| ref_escapes_whole(db, a.body, init))
                || ref_escapes_whole(db, body, init)
        }
        Resolved::Resume { value, next_state } => {
            ref_escapes_whole(db, value, init) || ref_escapes_whole(db, next_state, init)
        }
        Resolved::Host { body, .. } => ref_escapes_whole(db, body, init),
        // A `SumPayload` reads a PIECE of the scrutinee (`sum-payload`), not the whole value — like a
        // projection operand, it is not a whole-value escape of `init`.
        Resolved::SumPayload { .. }
        | Resolved::Int(_)
        | Resolved::Bool(_)
        | Resolved::Str(_)
        | Resolved::Float(_)
        | Resolved::Unit
        | Resolved::Prim(_)
        | Resolved::Param { .. }
        | Resolved::TypeVal(_)
        | Resolved::Lambda { .. }
        | Resolved::Poison(_) => false,
    }
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
    /// Whether the walk starts by recovering the SUM PAYLOAD: when this leaf lives inside a sum
    /// variant's payload, the walker first calls `sum-payload(rep)` to get the payload handle, THEN
    /// applies `path`. A single-payload variant leaves `path` empty (the payload handle IS the boxed
    /// leaf); a multi-payload variant's `path` indexes into the payload tuple. `false` for a plain
    /// tuple/record leaf (the walk starts at the root handle directly). Set on the per-variant templates
    /// a [`SumFormTemplate`] holds.
    pub via_sum_payload: bool,
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

/// The two STATIC halves of a runtime `Bytes` value form, for the looping `encode()` walker (L2b).
/// The value form of `(: <bytes> Bytes)` is `PREFIX · <LEB len> · <n raw bytes> · SUFFIX`, where ONLY
/// the leaf's length-LEB and payload are runtime — the prefix (header … the `KIND_BYTES` tag) and the
/// suffix (the `Bytes` type-name leaf + the whole struct table + root) are byte-identical regardless of
/// `n` (verified across n = 0 / 3 / 130). So the walker writes `prefix`, then the runtime LEB of
/// `bytes-len`, then copies the bytes, then `suffix` — no fixed-size template. `DESIGN-runtime-bytes-
/// escape-walker.md`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RuntimeBytesForm {
    /// Bytes to write verbatim BEFORE the runtime length+payload — the header through the KIND_BYTES tag.
    pub prefix: Vec<u8>,
    /// Bytes to write verbatim AFTER the runtime payload — the type-name leaf + struct table + root.
    pub suffix: Vec<u8>,
}

/// Compute the [`RuntimeBytesForm`] for `Ty::Bytes` — build the ZERO-length Bytes value form (`…0b 00
/// <suffix>`) and split it at the leaf's length byte: `prefix` = everything up to and INCLUDING the
/// `KIND_BYTES` tag, `suffix` = everything AFTER the `00` length byte. A runtime walker fills the gap
/// with `<LEB n><n bytes>`. `None` if the encoded form does not have the expected `0b 00` shape (a
/// codec change) — the escape then declines rather than emit a wrong walker.
pub fn runtime_bytes_form(db: &mut Db) -> Option<RuntimeBytesForm> {
    // Build `(: b"" Bytes)` — an empty Bytes leaf — and encode it. Its leaf pool holds `":"`, the empty
    // bytes leaf (`0b 00`), and `"Bytes"`; the struct table + root follow.
    let _ = db; // (kept for signature symmetry with the other form builders; not needed here)
    let mut b = crate::ast::Builder::new();
    let colon = b.name(":");
    let empty = b.atom_leaf(crate::ast::Leaf::Bytes(Vec::new()));
    let bytes_ty = b.name("Bytes");
    let root = b.list(vec![colon, empty, bytes_ty]);
    let arenas = b.finish(root);
    let encoded = crate::codec::encode(&arenas);
    // Find the `KIND_BYTES`(0x0b) tag IMMEDIATELY followed by its `0x00` length byte (the empty leaf).
    // `":"` is a NAME leaf (`0x0a 01 3a`) and `"Bytes"` a NAME leaf too, so the only `0b 00` pair is the
    // empty bytes leaf's tag+length. Split there.
    const KIND_BYTES: u8 = 11;
    let pos = encoded.windows(2).position(|w| w == [KIND_BYTES, 0x00])?;
    // prefix = header … the KIND_BYTES tag (inclusive); the byte at `pos+1` is the `00` length we replace.
    let prefix = encoded[..=pos].to_vec();
    // suffix = everything AFTER the `00` length byte (an empty payload contributes no bytes).
    let suffix = encoded[pos + 2..].to_vec();
    Some(RuntimeBytesForm { prefix, suffix })
}

/// A value-form template: the byte buffer (placeholders in the leaf values) + the runtime holes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ValueFormTemplate {
    pub bytes: Vec<u8>,
    pub leaves: Vec<RuntimeLeaf>,
}

/// The escape template for a SUM result — one complete value-form template per variant (its rendered
/// `(: (VariantName payload…) SumType)` bytes + holes), indexed by DISCRIMINANT. Unlike a tuple/record
/// (one static shape, one template), a sum renders DIFFERENTLY per variant (`(Some 5)` vs `(None unit)`
/// — different name, different payload), so the walker must switch on the runtime discriminant
/// (`sum-disc`) and emit the matching variant's template. Each variant's payload leaves carry
/// `via_sum_payload` (they are reached through `sum-payload` first). `type-system.md §A Match Is
/// Exhaustive Against The Sum Type's Variant Set` — the variant set is closed, so the switch is total.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SumFormTemplate {
    /// One template per variant, in DISCRIMINANT (declaration) order — `variants[disc]` renders the
    /// value with that discriminant.
    pub variants: Vec<ValueFormTemplate>,
}

/// Build the [`SumFormTemplate`] for a `Ty::Sum` result: one value-form template per variant. Each
/// variant's template renders `(: (VariantName payload…) SumType)` with the payload leaves left as
/// holes reached through `sum-payload`. A NULLARY variant renders `(VariantName unit)` (no holes) — the
/// corpus form (`(None unit)`). A SINGLE-payload variant renders `(VariantName <scalar-hole>)`, the
/// hole reached directly off the payload handle (`via_sum_payload`, empty `path`). A MULTI-payload
/// variant renders `(VariantName p0 p1 …)`, the holes reached by `arr-get` into the payload tuple. A
/// payload whose type has no value-form surface (a function/nested-sum for now) makes the whole thing
/// `None` — the escape declines. Needs `db` to read the variant names + payload types from
/// `db.type_decls` (found by the sum's `decl` occurrence).
pub fn sum_form_template(db: &mut Db, ty: &crate::ty::Ty) -> Option<SumFormTemplate> {
    let crate::ty::Ty::Sum { decl, args, .. } = ty else {
        return None;
    };
    // Recover the variant set + the declaration's type PARAMS from the declaration occurrence. A generic
    // sum's payload occurrences mention the params (a lowercase `a`); the instantiation's `args` are the
    // concrete types to substitute for them, positionally.
    let decl_ref = db.type_decl_by_occ(*decl)?;
    let params = decl_ref.params.clone();
    // Clone the shape out so we can reduce payload types with `&mut db` below.
    let variants: Vec<(String, Vec<StructId>)> = decl_ref
        .variants
        .iter()
        .map(|v| (v.name.clone(), v.payloads.clone()))
        .collect();
    let mut out = Vec::with_capacity(variants.len());
    for (disc, (_, payload_occs)) in variants.iter().enumerate() {
        // Reduce each payload TYPE occurrence to a `Ty` AT THE INSTANTIATION: a payload that IS a type
        // parameter (a bare name in `params`) becomes the corresponding concrete `arg`; any other
        // payload reduces normally (`typeval_of`). This is what makes a generic `Option Int64` escape
        // with its `Some` payload templated as `Int64` rather than the unresolvable param `a`.
        let mut payload_tys = Vec::with_capacity(payload_occs.len());
        for &p in payload_occs {
            let pty = match db.ast.as_name(p) {
                Some(n) if params.iter().any(|q| q == n) => {
                    let idx = params.iter().position(|q| q == n).unwrap();
                    args.get(idx).cloned()?
                }
                _ => crate::eval::typeval_of(db, p)?,
            };
            payload_tys.push(pty);
        }
        // The variant HEAD — QUALIFIED `(. IntList Cons)` for a user sum, a BARE `Some` name for a
        // built-in prelude sum — so the runtime walker writes the same head the constant bake does.
        out.push(variant_form_template(
            db,
            *decl,
            disc as u32,
            &payload_tys,
            ty,
        )?);
    }
    Some(SumFormTemplate { variants: out })
}

/// One variant's value-form template: `(: <variant-head> payload…) SumType)`, payload leaves as holes
/// reached via `sum-payload`. Arity shapes the value + the hole paths (see [`sum_form_template`]). The
/// variant HEAD is built by [`variant_head_ast`] (qualified `(. Type Variant)` for a user sum, a bare
/// name for a built-in), so the runtime template writes the identical head the constant bake does.
fn variant_form_template(
    db: &Db,
    decl: StructId,
    disc: u32,
    payloads: &[crate::ty::Ty],
    sum_ty: &crate::ty::Ty,
) -> Option<ValueFormTemplate> {
    let mut b = crate::ast::Builder::new();
    let colon = b.name(":");
    let mut leaves: Vec<PendingLeaf> = Vec::new();
    // The VALUE: `(<variant-head> payload…)`.
    let value = {
        let head = variant_head_ast(db, &mut b, decl, disc)?;
        let mut children = vec![head];
        match payloads.len() {
            // Nullary: `(VariantName unit)` — the corpus form (`(None unit)`), no holes.
            0 => {
                children.push(b.name("unit"));
            }
            // Single payload: reached DIRECTLY off the payload handle — `via_sum_payload`, empty path.
            1 => {
                let mut path = Vec::new();
                children.push(template_value_ast_flagged(
                    &mut b,
                    &payloads[0],
                    &mut path,
                    &mut leaves,
                    true,
                )?);
            }
            // Multiple payloads: the payload is a tuple handle — `arr-get(i)` into it, `via_sum_payload`.
            _ => {
                for (i, pty) in payloads.iter().enumerate() {
                    let mut path = vec![i as u32];
                    children.push(template_value_ast_flagged(
                        &mut b,
                        pty,
                        &mut path,
                        &mut leaves,
                        true,
                    )?);
                }
            }
        }
        b.list(children)
    };
    // The TYPE node — the sum's full type surface: a bare `Sign` for a monomorphic sum, `(Option
    // Int64)` for a generic instantiation (`type_ast`'s `Ty::Sum` arm renders both from the solved
    // type). So `(: (Some 5) (Option Int64))` — the corpus parameterized form.
    let type_node = type_ast(&mut b, sum_ty)?;
    let root = b.list(vec![colon, value, type_node]);
    let arenas = b.finish(root);
    let bytes = crate::codec::encode(&arenas);
    let holes = resolve_leaf_offsets(&bytes, &arenas, &leaves)?;
    Some(ValueFormTemplate {
        bytes,
        leaves: holes,
    })
}

/// A leaf recorded during template construction, before its byte offset is resolved: its arena `LeafId`
/// (to locate it in the encoded pool) plus the runtime info the hole carries.
struct PendingLeaf {
    leaf_id: crate::ast::LeafId,
    path: Vec<u32>,
    kind: LeafFill,
    /// Whether this leaf is reached through `sum-payload` first (a sum variant payload leaf) — carried
    /// onto the resolved [`RuntimeLeaf`]. `false` for a plain tuple/record leaf.
    via_sum_payload: bool,
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
    template_value_ast_flagged(b, ty, path, out, false)
}

/// The core of [`template_value_ast`] with the `via_sum_payload` flag threaded onto each recorded leaf
/// — set when building a sum VARIANT PAYLOAD's sub-template (the leaves are reached through
/// `sum-payload` first). The flat tuple/record path passes `false`.
fn template_value_ast_flagged(
    b: &mut crate::ast::Builder,
    ty: &crate::ty::Ty,
    path: &mut Vec<u32>,
    out: &mut Vec<PendingLeaf>,
    via_sum_payload: bool,
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
                via_sum_payload,
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
                via_sum_payload,
            });
            Some(atom)
        }
        Ty::Tuple(elems) => {
            let head = b.name("tuple");
            let mut children = vec![head];
            for (i, e) in elems.iter().enumerate() {
                path.push(i as u32);
                children.push(template_value_ast_flagged(
                    b,
                    e,
                    path,
                    out,
                    via_sum_payload,
                )?);
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
                let fval = template_value_ast_flagged(b, t, path, out, via_sum_payload)?;
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
            // A bytes leaf is a fully-baked constant (no runtime hole) — advance past it like a Str
            // (kind byte + len LEB + the raw bytes).
            crate::ast::Leaf::Bytes(bs) => {
                off += 1 + leb_len(bs.len() as u64) + bs.len();
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
            via_sum_payload: p.via_sum_payload,
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

/// Build the variant HEAD s-expression for variant `disc` of the sum declared at `decl`, as it appears
/// in an observed value's canonical form: the variant's BARE NAME atom — `Some`, `Sm`, `Cons`, `Pos`. A
/// variant renders the SAME whether its sum is BUILT-IN (Option/Result) or USER-declared: the value form
/// of a variant does not depend on where its sum was declared (the built-in-vs-user split that rendered a
/// user variant as the member-access `(. Type Variant)` while a built-in rendered bare was an
/// inconsistency — a rendered VALUE should be a variant name, not a projection expression). The rendered
/// value is always annotated with its sum type (`(: (Sm 42) Opt)`), which disambiguates a bare variant
/// name shared across sums (sum identity is by declaration occurrence, carried by the annotation). `None`
/// if the disc is out of range (a compiler bug). Shared by the constant-escape bake and the
/// runtime-escape template so both write the identical head.
fn variant_head_ast(
    db: &Db,
    b: &mut crate::ast::Builder,
    decl: StructId,
    disc: u32,
) -> Option<StructId> {
    let t = db.type_decl_by_occ(decl)?;
    let vname = t.variants.get(disc as usize)?.name.clone();
    Some(b.name(vname))
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
        // A constant string bakes as its `"…"` leaf — the codec encodes it (KIND_STR: len + UTF-8
        // bytes), and the host reader lifts it back to a string value.
        Core::ConstStr(s) => Some(b.atom_leaf(Leaf::Str(s))),
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
        // A CONSTANT list literal renders `(list e1 e2 …)` — its length is statically known (unlike a
        // grown/runtime list), so its bytes bake exactly like a constant tuple's. Each element is a
        // constant in turn (a non-constant element makes the whole value non-constant, so `core_of` would
        // not be a `ListNew` of constants and this returns `None`, declining the escape).
        Core::ListNew { elems } => {
            let head = b.name("list");
            let mut children = vec![head];
            for e in elems {
                children.push(const_value_ast(db, b, e)?);
            }
            Some(b.list(children))
        }
        // A CONSTANT sum value — `(Some 5)`, `(None unit)`, `(Some (Some 5))`. Its canonical form is
        // `(VariantName payload…)` with the variant TAG present (`deterministic-value-form.md`;
        // core-semantics.md §A Constructor Applied To An Argument Is A Sum Value). This holds regardless
        // of what the payload IS — a scalar, a tuple, or ANOTHER sum value — so a NESTED constant sum
        // (`(Some (Some 5))`) bakes recursively, both variant tags present. This is the constant-escape
        // (R1) companion of `sum_form_template`'s runtime walker: a fully-constant sum crosses by baked
        // bytes here, so it never needs the per-variant runtime template (which cannot express a nested
        // sum's variable-length inner shape). The variant NAME is recovered from the disc against this
        // node's solved sum type (its declaration's variant set); a nullary variant carries `unit`.
        Core::SumNew { disc, payloads } => {
            let ty = crate::infer::type_of(db, id);
            let crate::ty::Ty::Sum { decl, .. } = ty else {
                return None; // a SumNew whose solved type is not a sum is a compiler bug — decline
            };
            let head = variant_head_ast(db, b, decl, disc)?;
            let mut children = vec![head];
            match payloads.len() {
                // Nullary variant: `(VariantName unit)` — the corpus form (`(None unit)`).
                0 => children.push(b.name("unit")),
                // Single payload (the canonical variant shape — one payload type, a scalar / tuple /
                // nested sum): render it recursively.
                1 => children.push(const_value_ast(db, b, payloads[0])?),
                // Multiple application arguments (a `(V.Both a b)` multi-arg surface) — not a canonical
                // single-payload form; the escape declines rather than guess a rendering.
                _ => return None,
            }
            Some(b.list(children))
        }
        // A constant `Bytes.of` → a `Leaf::Bytes` value node (rendered `b"…"` by the host). Each element
        // is a constant Int in `0..=255` (range-checked at `lower_bytes_of`); collect the raw bytes. A
        // non-constant element would have declined at `lower_bytes_of` (no `Core::BytesOf` built), so
        // every element here folds to a `ConstInt` in range.
        Core::BytesOf { elems } => {
            let mut raw = Vec::with_capacity(elems.len());
            for e in elems {
                match core_of(db, e) {
                    Core::ConstInt(v) => {
                        raw.push(v.to_i64().filter(|n| (0..=255).contains(n))? as u8)
                    }
                    _ => return None,
                }
            }
            Some(b.atom_leaf(Leaf::Bytes(raw)))
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
        // A scalar's type surface is its name atom. `String` is a monomorphic named type too, so its
        // surface is the bare `String` atom (`render_name`).
        Ty::Int(_) | Ty::Bool | Ty::Unit | Ty::String => Some(b.name(ty.render_name())),
        // A sum's type surface: the bare NAME for a monomorphic sum (`(: (Neg unit) Sign)`), or the
        // STRUCTURED application `(Option Int64)` for a generic instantiation — a `(NAME arg…)` list, so
        // the args round-trip as separate nodes (not one spaced-out name atom). Matches `render_name`'s
        // surface but built as real structure so the codec + host reader see the parameterized type.
        Ty::Sum { name, args, .. } => {
            if args.is_empty() {
                Some(b.name(name.clone()))
            } else {
                let head = b.name(name.clone());
                let mut children = vec![head];
                for a in args {
                    children.push(type_ast(b, a)?);
                }
                Some(b.list(children))
            }
        }
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
        // A list's type surface is `(List Elem)` — matches `render_name`.
        Ty::List(elem) => {
            let head = b.name("List");
            let ety = type_ast(b, elem)?;
            Some(b.list(vec![head, ety]))
        }
        // A bytes value's type surface is the bare name `Bytes` (a leaf, like a scalar) — matches
        // `render_name`; its VALUE renders `b"…"` (built in `const_value_ast` / the escape walker).
        Ty::Bytes => Some(b.name("Bytes".to_string())),
        // A still-free type variable in an escaping value's type has NO defined serialization — a bare
        // `(None)` : `Option ?0` or an empty `(list)` : `List ?0` whose payload/element nothing pins. It
        // is NOT rendered (no honest concrete surface exists): returning `None` here makes
        // `constant_value_form`/`sum_form_template` decline, so the escape falls through to the
        // AMBIGUOUS-TYPE guard in `backend/wasm/mod.rs` (`has_free_var` → CDZ0203, "annotate it") rather
        // than crossing with an invented type. type-system.md §An Escaping Value MUST Have A Fully
        // Determined Type; corpus 07 "an escaped value with an unresolved payload type is rejected".
        // A float has no boundary value form yet (no float value runs / crosses), so no type surface —
        // like a function/type-value. A float program declines before reaching the escape anyway.
        Ty::Fn(_, _) | Ty::Type | Ty::Var(_) | Ty::Any | Ty::Float => None,
    }
}

/// Conditional-constant-propagation helper: if `branch` reduces to an inner `(if c' A B)` whose
/// condition `c'` is EQUIVALENT to the enclosing `cond` (via `core_equiv` — a pure-core structural
/// match), return the occurrence of the arm the enclosing branch's known truth of `cond` selects — `A`
/// when `cond_is_true` (the then-branch, where `cond` holds), `B` otherwise (the else-branch, where it
/// does not). `None` if `branch` is not such a nested `if` (leave it unchanged). The returned occurrence
/// is REUSED as-is (no synthesis); it was resolved in the same scope, so lowering it in the branch's
/// place is sound. `reduce_to_if` chases refs/annotations and stops at a kept multi-use binding, so a
/// `let`-named inner `if` is not peeled (its value lives in a slot). Only the DIRECT nested `if` is
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
        Some(if cond_is_true { inner_then } else { inner_else })
    } else {
        None
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
        Resolved::And { lhs, rhs, .. } => uses_in(db, lhs, init) + uses_in(db, rhs, init),
        Resolved::Not { operand } => uses_in(db, operand, init),
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
        Resolved::Tuple { elems } | Resolved::List { elems } => {
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
            for a in args.iter() {
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
        // A `SumPayload` reads the scrutinee at run time (`sum-payload`); if the scrutinee is `init`,
        // that is a use of the binding.
        Resolved::SumPayload { scrutinee, .. } => usize::from(scrutinee == init) as u32,
        // Effect control forms: the binding may be referenced in a handler's init, any arm body, a
        // resumption's value/next-state, or the handled/delegated body — count each position.
        Resolved::Handle {
            init: seed,
            arms,
            body,
        } => {
            let mut n = uses_in(db, seed, init);
            for arm in &arms {
                n += uses_in(db, arm.body, init);
            }
            n + uses_in(db, body, init)
        }
        Resolved::Resume { value, next_state } => {
            uses_in(db, value, init) + uses_in(db, next_state, init)
        }
        Resolved::Host { body, .. } => uses_in(db, body, init),
        // Leaves and non-referencing forms contribute nothing.
        Resolved::Int(_)
        | Resolved::Bool(_)
        | Resolved::Str(_)
        | Resolved::Float(_)
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
        // ALGEBRAIC IDENTITY: one operand is a constant whose value makes the op a NO-OP or a constant
        // result — the whole checked operation (and its overflow guard) is eliminated at lowering. Only
        // the identities that are SAFE at every width and never trap are applied (see `arith_identity`);
        // the RESULT keeps the op's solved type because the runtime operand shares it (binary-op
        // unification), and a `0`/`1` constant grounds to that width at selection.
        (lc, rc) => {
            if let Some(simplified) = arith_identity(db, op, args[0], &lc, args[1], &rc) {
                trace!(target: "rcdzc::lower", op = intrinsic_name(op), "arithmetic identity simplified (op elided)");
                return simplified;
            }
            trace!(target: "rcdzc::lower", op = intrinsic_name(op), "arithmetic stays runtime (operand not constant)");
            Core::Arith {
                op,
                lhs: args[0],
                rhs: args[1],
            }
        }
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
        // `x | 0` / `0 | x` / `x ^ 0` / `0 ^ x` → x.
        Prim::BitOr | Prim::BitXor if is(rc, 0) => Some(lc.clone()),
        Prim::BitOr | Prim::BitXor if is(lc, 0) => Some(rc.clone()),
        // `x & 0` / `0 & x` → 0 — DISCARDS x, so only when x cannot trap.
        Prim::BitAnd if is(rc, 0) && is_trap_free(db, lhs) => Some(zero()),
        Prim::BitAnd if is(lc, 0) && is_trap_free(db, rhs) => Some(zero()),
        // `x << 0` / `x >> 0` → x (a zero shift COUNT is a no-op; count is the right operand).
        Prim::Shl | Prim::Shr if is(rc, 0) => Some(lc.clone()),
        // `x / 1` → x (division by one is the identity; keeps x, so its own traps stay).
        Prim::Div if is(rc, 1) => Some(lc.clone()),
        // `x % 1` → 0 (every integer is divisible by 1) — DISCARDS x, so only when x cannot trap.
        Prim::Rem if is(rc, 1) && is_trap_free(db, lhs) => Some(zero()),

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

/// Whether the node at `id` lowers to a core that CANNOT TRAP at run time — so discarding it (an
/// annihilator identity like `x * 0 → 0`) loses no defined trap. CONSERVATIVE: only a value with no
/// checked operation anywhere inside it. Trap-free = a leaf (constant/param/local/unit), a wrap
/// (total), or a bitwise op / conversion / projection over trap-free operands. NOT trap-free = `+`/
/// `-`/`*`/`<<`/`>>` (overflow/count guards), `/`/`%` (÷0, MIN/-1), a call (its body may trap), an
/// `if`/`match` (a branch may trap), a sum/tuple/record construct (may allocate/box — treated as
/// possibly-effecting here). Reads the operand's already-lowered core recursively.
fn is_trap_free(db: &mut Db, id: StructId) -> bool {
    match core_of(db, id) {
        Core::ConstInt(_)
        | Core::ConstBool(_)
        | Core::Unit
        | Core::Param { .. }
        | Core::LocalRef { .. } => true,
        // Bitwise ops are total; a comparison never traps — trap-free if their operands are.
        Core::Arith {
            op: Prim::BitAnd | Prim::BitOr | Prim::BitXor,
            lhs,
            rhs,
        }
        | Core::Compare { lhs, rhs, .. } => is_trap_free(db, lhs) && is_trap_free(db, rhs),
        // `wrap` is total (never traps) — trap-free if its operand is.
        Core::Convert {
            op: Prim::Wrap,
            operand,
        } => is_trap_free(db, operand),
        // Everything else — checked arithmetic (+/-/*/shifts), div/rem, calls, control flow, heap
        // constructs, poison — is conservatively treated as possibly-trapping.
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
        ) => ox == oy && core_equiv(db, lx, ly) && core_equiv(db, rx, ry),
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
        | Prim::UnitTy
        | Prim::SumNew
        | Prim::SumCtor
        | Prim::TupleNew
        | Prim::RecordNew
        | Prim::ListNew
        | Prim::ListLen
        | Prim::ListPush
        | Prim::ListConcat
        | Prim::ListUpdate
        | Prim::ListAt
        | Prim::ListCtor
        | Prim::BytesOf
        | Prim::BytesLen
        | Prim::BytesTy
        | Prim::StrScalarLen
        | Prim::StrByteLen
        | Prim::StrAt
        | Prim::StrConcat
        | Prim::StrSlice
        | Prim::SumExpect
        | Prim::CheckedAdd
        | Prim::CheckedMul
        | Prim::WrappingAdd
        | Prim::WrappingMul
        | Prim::StringTy
        | Prim::BytesAt
        | Prim::BytesConcat
        | Prim::BytesSlice
        | Prim::BytesCompact => {
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
        // Two CONSTANT strings compare by their text (lexicographic by Unicode scalar values — the byte
        // order of NFC UTF-8, which the reader already normalized to). `(= "a" "a")` → true; ordering
        // comparisons (`<`) order by text. A constant fold, no heap: the string equality the compiler
        // needs for tag/name dispatch.
        (Core::ConstStr(a), Core::ConstStr(b)) => {
            let r = compare_ord(op, a.cmp(&b));
            trace!(target: "rcdzc::fold", op = intrinsic_name(op), result = r, "folded constant string comparison");
            Core::ConstBool(r)
        }
        // Two CONSTANT floats compare by their canonical Float64 value (contracts/deterministic-value-
        // form.md #Numeric Values Serialize Deterministically — floats equal under structural equality
        // share a canonical form, distinct floats have distinct forms). EQUALITY (`=`) is by RAW BITS, so
        // `-0.0 ≠ 0.0` (distinct bit patterns → the canonical form distinguishes them) and a NaN is
        // unequal to itself. `1e19` and `1e20` round to different doubles → unequal. Ordering (`<`/`>`)
        // uses the IEEE partial order (`f64::partial_cmp`); an unordered pair (NaN) declines rather than
        // inventing a total order. Only the fold — no float runtime is needed for a Bool result.
        (Core::ConstFloat(a), Core::ConstFloat(b)) => {
            let (ba, bb) = (a.to_f64_bits(), b.to_f64_bits());
            if matches!(op, Prim::Eq) {
                let r = ba == bb;
                trace!(target: "rcdzc::fold", op = intrinsic_name(op), result = r, "folded constant float equality (by canonical bits)");
                Core::ConstBool(r)
            } else {
                match f64::from_bits(ba).partial_cmp(&f64::from_bits(bb)) {
                    Some(ord) => {
                        let r = compare_ord(op, ord);
                        trace!(target: "rcdzc::fold", op = intrinsic_name(op), result = r, "folded constant float comparison");
                        Core::ConstBool(r)
                    }
                    // An unordered pair (a NaN operand) has no defined `<`/`>` result — decline.
                    None => Core::Poison(Reject::decline(
                        "an ordering comparison with a NaN operand has no defined result",
                    )),
                }
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
            // CONSTANT COMPOUND EQUALITY folds STRUCTURALLY (`core-semantics.md §Equality Is Structural`:
            // two values are equal when they have the same type and their contents are equal
            // component-wise). Only for `=` (a total ordering `<`/`>` over compounds is a later stage);
            // only when BOTH operands are compile-time-visible constant compounds (a `SumNew`/`Tuple`/
            // `Record`/`ListNew`, recursively) — a runtime operand still needs the heap walk (deferred).
            // `(= (Some 1) (Some 1))` → true, `(= (Some 1) (Some 2))` → false, `(= None None)` → true,
            // `(= (tuple 1 2) (tuple 1 2))` → true. A nested compound compares recursively (a payload/
            // element that is itself a compound). Returns `None` when either side is not a constant
            // compound → falls through to the scalar-runtime / decline below.
            if matches!(op, Prim::Eq)
                && let Some(eq) = const_compound_eq(db, args[0], args[1])
            {
                trace!(target: "rcdzc::fold", result = eq, "folded constant compound equality (structural)");
                return Core::ConstBool(eq);
            }
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

/// Structurally compare two CONSTANT compound values at `a`/`b`, returning `Some(true/false)` if BOTH are
/// compile-time-visible constants (a `SumNew`/`Tuple`/`Record`/`ListNew`, or a scalar leaf), else `None`
/// (a runtime operand — the caller declines, deferring to the heap walk). Equality is STRUCTURAL
/// (`core-semantics.md §Equality Is Structural`): two values are equal iff same shape + component-wise
/// equal. A `SumNew` compares its discriminant then its payloads pairwise; a `Tuple`/`ListNew` its
/// elements pairwise (unequal length → not equal); a `Record` its fields (the field SET is fixed by the
/// type, so same-typed records share keys — compare each). Scalar leaves compare by value. Two DIFFERENT
/// compound KINDS (a tuple vs a sum) never fold here — the type checker rejects a cross-shape `=` before
/// lowering, so a kind mismatch reaching here is a compiler bug → `None` (decline).
fn const_compound_eq(db: &mut Db, a: StructId, b: StructId) -> Option<bool> {
    match (core_of(db, a), core_of(db, b)) {
        (Core::ConstInt(x), Core::ConstInt(y)) => Some(x.eq_value(&y)),
        (Core::ConstBool(x), Core::ConstBool(y)) => Some(x == y),
        (Core::ConstStr(x), Core::ConstStr(y)) => Some(x == y),
        // Two floats: equal iff their canonical Float64 BITS match — so a nested `-0.0` is distinct from
        // `0.0` (`(= (tuple -0.0) (tuple 0.0))` → false) and a nested NaN equals a nested NaN (identical
        // bits under the canonical byte form; contracts/deterministic-value-form.md). By-bits, NOT
        // `f64` `==`, precisely so `-0.0`/`0.0` differ and NaN self-equals — the structural byte-form rule.
        (Core::ConstFloat(x), Core::ConstFloat(y)) => Some(x.to_f64_bits() == y.to_f64_bits()),
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
        (Core::Tuple { elems: ea }, Core::Tuple { elems: eb })
        | (Core::ListNew { elems: ea }, Core::ListNew { elems: eb }) => {
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
        // Any other pairing includes a runtime operand (not a constant compound) — decline the fold.
        _ => None,
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
fn lower_sum_new(db: &mut Db, head: StructId, args: &[StructId]) -> Core {
    let Some(disc) = crate::eval::variant_disc_of(db, head) else {
        return Core::Poison(Reject::decline(
            "a sum constructor has no discriminant metadata",
        ));
    };
    // A NULLARY variant is CONSTRUCTED by applying it to the unit value — `(None unit)` / `(Nil ())` —
    // the canonical form (core-semantics.md §Construction MUST Be Via Application: "(None unit)"; a
    // nullary variant carries unit). Its ctor `(meta t)` is the bare sum (no arrow → `variant_payload_type`
    // is `None`), so the single `unit` argument is NOT a payload — the payload of a nullary variant IS the
    // unit value, built as an empty array by the backend (`SumNew` with no payloads). Drop the unit arg so
    // it is not boxed as a spurious payload. (A bare `None` used as a value takes the no-arg path directly.)
    if crate::eval::variant_payload_type(db, head).is_none() && args.len() == 1 {
        // The argument must BE the unit value — a nullary variant applied to a non-unit is an arity error
        // the type-checker reports; here, lower it as the nullary construction (the type fault surfaces in
        // `type_errors`, and an over-payloaded nullary is caught there, not silently given a payload).
        return Core::SumNew {
            disc,
            payloads: Vec::new(),
        };
    }
    Core::SumNew {
        disc,
        payloads: args.to_vec(),
    }
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

/// Lower `(List.at list index)` — the fallible indexed read. FOLD when the `list` operand is a
/// compile-time-visible list literal AND the `index` folds to a constant: an in-range index (`0 <= i <
/// arity`) yields `(Some elem)` — a `Core::SumNew` of the element's core at the `Some` discriminant —
/// and an out-of-range index (negative or `>= arity`) yields `None` (`Core::SumNew` with no payloads at
/// the `None` discriminant). Both fold to the ordinary sum construction, so a constant `List.at` renders
/// through the sum escape/fold with no heap. Otherwise emit the runtime `Core::ListAt` (a bounds-checked
/// `vec-get`). A poison list/index propagates.
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
    // FOLD a constant list literal indexed by a constant integer.
    if let (Core::ListNew { elems }, Core::ConstInt(i)) = (core_of(db, list), core_of(db, index)) {
        // The index is a signed Int64; a negative value or one `>= arity` is out of bounds → `None`.
        match i.to_i64() {
            Some(n) if n >= 0 && (n as usize) < elems.len() => {
                trace!(target: "rcdzc::fold", node = id.0, index = n, "List.at folds to Some (in-bounds constant index)");
                return Core::SumNew {
                    disc: disc_some,
                    payloads: vec![elems[n as usize]],
                };
            }
            _ => {
                trace!(target: "rcdzc::fold", node = id.0, "List.at folds to None (out-of-bounds constant index)");
                return Core::SumNew {
                    disc: disc_none,
                    payloads: Vec::new(),
                };
            }
        }
    }
    // A runtime list or runtime index — emit the bounds-checked runtime read.
    Core::ListAt {
        list,
        index,
        disc_some,
        disc_none,
    }
}

/// Lower `(String.at string index)` — the fallible SCALAR-indexed read. FOLD when both operands are
/// constant: index the string by UNICODE SCALAR position (`chars().nth`, NOT byte offset —
/// collections-and-text.md #A String Is A Sequence Of Unicode Scalar Values), yielding `(Some
/// "<char>")` in range (the ONE-scalar string at that position, a fresh `Core::ConstStr` synthesized
/// into the arena) and `None` out (negative, or `>=` the scalar length). Builds a `Core::SumNew` at the
/// result Option's Some/None discriminants, so it rides the ordinary sum fold/escape/match — no string
/// heap. A runtime string declines (the byte-rope indexed read is a later increment). A poison
/// operand propagates.
fn lower_str_at(db: &mut Db, id: StructId, string: StructId, index: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, string) {
        return Core::Poison(r);
    }
    if let Core::Poison(r) = core_of(db, index) {
        return Core::Poison(r);
    }
    let Some((disc_some, disc_none)) = option_discs(db, id) else {
        return Core::Poison(Reject::decline(
            "String.at result is not the built-in Option sum",
        ));
    };
    match (core_of(db, string), core_of(db, index)) {
        (Core::ConstStr(s), Core::ConstInt(i)) => {
            // Index by scalar value; a negative index or one at/beyond the scalar length is out of range.
            let scalar = i.to_i64().and_then(|n| {
                if n >= 0 {
                    s.chars().nth(n as usize)
                } else {
                    None
                }
            });
            match scalar {
                Some(c) => {
                    // The one-scalar string at that position — a fresh `Leaf::Str` node whose `core_of`
                    // is `Core::ConstStr`, used as the `Some` payload (the same shape `List.at` uses,
                    // but the element is synthesized here since a string has no element sub-nodes).
                    trace!(target: "rcdzc::fold", node = id.0, "String.at folds to Some (in-bounds constant scalar index)");
                    let payload = db.push_atom(crate::ast::Leaf::Str(c.to_string()));
                    Core::SumNew {
                        disc: disc_some,
                        payloads: vec![payload],
                    }
                }
                None => {
                    trace!(target: "rcdzc::fold", node = id.0, "String.at folds to None (out-of-range constant index)");
                    Core::SumNew {
                        disc: disc_none,
                        payloads: Vec::new(),
                    }
                }
            }
        }
        // A runtime string or runtime index — the byte-rope indexed read is a later increment.
        _ => Core::Poison(Reject::decline(
            "String.at on a runtime string is not yet computed (constant strings only)",
        )),
    }
}

/// Lower `(String.slice string start end)` — the fallible SCALAR sub-range read, half-open `[start,
/// end)`. FOLD when all three operands are constant: cut the string by UNICODE SCALAR position (`chars`,
/// NOT byte offset — collections-and-text.md #A String Is A Sequence Of Unicode Scalar Values). The
/// range is well-defined only when `0 <= start <= end <= scalar-len`: then `(Some "<substr>")` (a fresh
/// `Core::ConstStr` of the selected scalars — `start == end` yields the empty string, present not None);
/// any bound outside that (reversed `end < start`, over-long `end > len`, or negative) yields `(None
/// unit)`. Builds a `Core::SumNew` at the result Option's discriminants, riding the ordinary sum
/// fold/escape/match — no string heap. A runtime string declines; a poison operand propagates.
fn lower_str_slice(
    db: &mut Db,
    id: StructId,
    string: StructId,
    start: StructId,
    end: StructId,
) -> Core {
    for operand in [string, start, end] {
        if let Core::Poison(r) = core_of(db, operand) {
            return Core::Poison(r);
        }
    }
    let Some((disc_some, disc_none)) = option_discs(db, id) else {
        return Core::Poison(Reject::decline(
            "String.slice result is not the built-in Option sum",
        ));
    };
    match (core_of(db, string), core_of(db, start), core_of(db, end)) {
        (Core::ConstStr(s), Core::ConstInt(a), Core::ConstInt(b)) => {
            let scalars: Vec<char> = s.chars().collect();
            let len = scalars.len() as i64;
            // The range is valid iff `0 <= start <= end <= scalar-len` (signed — a negative bound is out
            // of range, NOT wrapped to a large unsigned offset). `start == end` is an in-range empty slice.
            match (a.to_i64(), b.to_i64()) {
                (Some(a), Some(b)) if a >= 0 && a <= b && b <= len => {
                    let sub: String = scalars[a as usize..b as usize].iter().collect();
                    trace!(target: "rcdzc::fold", node = id.0, "String.slice folds to Some (in-range constant bounds)");
                    let payload = db.push_atom(crate::ast::Leaf::Str(sub));
                    Core::SumNew {
                        disc: disc_some,
                        payloads: vec![payload],
                    }
                }
                _ => {
                    trace!(target: "rcdzc::fold", node = id.0, "String.slice folds to None (out-of-range constant bounds)");
                    Core::SumNew {
                        disc: disc_none,
                        payloads: Vec::new(),
                    }
                }
            }
        }
        // A runtime string or runtime bound — the byte-rope slice is a later increment.
        _ => Core::Poison(Reject::decline(
            "String.slice on a runtime string is not yet computed (constant strings only)",
        )),
    }
}

/// Lower `(Option.expect sum message)` / `(Result.expect sum message)` — the unwrap-or-trap accessor. The
/// PRESENT variant is discriminant 0 (`Some`/`Ok`, the sum's FIRST variant — the shape the `expect` field
/// is added for). FOLD a compile-time-visible PRESENT variant (`Core::SumNew{disc:0, payloads:[p]}`) to
/// its payload `p` (the message is discarded). A constant ABSENT variant is a PROVABLE trap; not folded
/// yet (declines cleanly — no corpus case exercises a constant absent expect, and a codeless decline
/// grades Todo, never a miscompile). A runtime sum emits `Core::SumExpect` (disc probe → payload / trap).
/// A poison sum propagates. `message` is not lowered — the wasm trap carries no text.
fn lower_sum_expect(db: &mut Db, id: StructId, sum: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, sum) {
        return Core::Poison(r);
    }
    // The present variant is discriminant 0 (the sum's first variant). Confirm the scrutinee IS a sum.
    let crate::ty::Ty::Sum { .. } = crate::infer::type_of(db, sum) else {
        return Core::Poison(Reject::decline(
            "expect applies to an Option/Result sum value",
        ));
    };
    const DISC_PRESENT: u32 = 0;
    // FOLD a compile-time-visible present variant to its single payload.
    if let Core::SumNew { disc, payloads } = core_of(db, sum) {
        if disc == DISC_PRESENT && payloads.len() == 1 {
            trace!(target: "rcdzc::fold", node = id.0, "expect folds a constant present variant to its payload");
            return core_of(db, payloads[0]);
        }
        if disc != DISC_PRESENT {
            // A provably-absent constant expect — a compile-time trap. Not folded this increment.
            return Core::Poison(Reject::decline(
                "expect on a constant absent variant (a provable trap) is not yet folded",
            ));
        }
    }
    // A runtime sum — probe the discriminant at run time, unwrap the payload or trap.
    Core::SumExpect {
        scrutinee: sum,
        disc_present: DISC_PRESENT,
    }
}

/// Lower `(Int64.checked-add a b)` / `(Int64.checked-mul a b)` — the FALLIBLE arithmetic companions of
/// the trapping `+`/`*`, returning `(Option T)`: `Some result` when it fits the width / `None` on
/// overflow (numeric-model.md §Overflow Is Defined). FOLD a constant operand pair via `i64` checked
/// arithmetic (the SAME `checked_add`/`checked_mul` `fold_arith` uses to prove the trapping op's overflow
/// — but here overflow yields `None`, not a build error): in range → `Core::SumNew{disc_some, [result]}`
/// (the result a fresh `Core::ConstInt` synthesized into the arena, the `Some` payload — the shape
/// `List.at`/`String.at` use); overflow → `Core::SumNew{disc_none, []}`. Both fold to the ordinary Option
/// construction, riding the sum fold/escape/match. A runtime operand is a later increment (declines
/// cleanly); a poison operand propagates.
fn lower_checked_arith(
    db: &mut Db,
    id: StructId,
    prim: Prim,
    lhs: StructId,
    rhs: StructId,
) -> Core {
    if let Core::Poison(r) = core_of(db, lhs) {
        return Core::Poison(r);
    }
    if let Core::Poison(r) = core_of(db, rhs) {
        return Core::Poison(r);
    }
    let Some((disc_some, disc_none)) = option_discs(db, id) else {
        return Core::Poison(Reject::decline(
            "checked-arithmetic result is not the built-in Option sum",
        ));
    };
    match (core_of(db, lhs), core_of(db, rhs)) {
        (Core::ConstInt(a), Core::ConstInt(b)) => {
            // Evaluate over `i64` (the Stage default width) — the same range the trapping fold uses. A
            // later width stage generalizes the overflow test to the solved width.
            let (Some(x), Some(y)) = (a.to_i64(), b.to_i64()) else {
                // An operand beyond the machine range — a later width stage handles it; decline for now.
                return Core::Poison(Reject::decline(
                    "checked arithmetic on an operand beyond the evaluated width is not yet folded",
                ));
            };
            let checked = match prim {
                Prim::CheckedAdd => x.checked_add(y),
                _ => x.checked_mul(y),
            };
            match checked {
                Some(n) => {
                    trace!(target: "rcdzc::fold", node = id.0, ?prim, result = n, "checked arithmetic folds to Some (in range)");
                    let payload = db.push_atom(crate::ast::Leaf::Int {
                        value: IntValue::from_i64(n),
                        radix: crate::ast::Radix::Dec,
                    });
                    Core::SumNew {
                        disc: disc_some,
                        payloads: vec![payload],
                    }
                }
                None => {
                    trace!(target: "rcdzc::fold", node = id.0, ?prim, "checked arithmetic folds to None (overflow)");
                    Core::SumNew {
                        disc: disc_none,
                        payloads: Vec::new(),
                    }
                }
            }
        }
        // A runtime operand — the overflow-detecting Some/None build is a later increment.
        _ => Core::Poison(Reject::decline(
            "checked arithmetic on a runtime operand is not yet computed (constant operands only)",
        )),
    }
}

/// Lower `(Int64.wrapping-add a b)` / `(Int64.wrapping-mul a b)` — two's-complement wraparound, NEVER
/// trapping (numeric-model.md §Overflow Is Defined — the modular value outcome). FOLD a constant operand
/// pair via `i64` `wrapping_add`/`wrapping_mul` (evaluated at the Stage default width; a later width stage
/// masks to the solved width). A runtime operand becomes a `Core::Arith` carrying the WRAPPING prim — the
/// backend selects the RAW machine `i64.add`/`i64.mul` (which already wraps), NOT the checked/trapping
/// path the `+`/`*` prims take. A poison operand propagates.
fn lower_wrapping_arith(db: &mut Db, prim: Prim, lhs: StructId, rhs: StructId) -> Core {
    let a = core_of(db, lhs);
    let b = core_of(db, rhs);
    match (a, b) {
        (Core::ConstInt(x), Core::ConstInt(y)) => {
            let (Some(x), Some(y)) = (x.to_i64(), y.to_i64()) else {
                return Core::Poison(Reject::decline(
                    "wrapping arithmetic on an operand beyond the evaluated width is not yet folded",
                ));
            };
            let n = match prim {
                Prim::WrappingAdd => x.wrapping_add(y),
                _ => x.wrapping_mul(y),
            };
            trace!(target: "rcdzc::fold", ?prim, result = n, "wrapping arithmetic folds to a constant");
            Core::ConstInt(IntValue::from_i64(n))
        }
        (Core::Poison(r), _) | (_, Core::Poison(r)) => Core::Poison(r),
        // A runtime operand — the RAW (non-trapping) machine op, selected in the backend from this prim.
        _ => Core::Arith { op: prim, lhs, rhs },
    }
}

/// Lower `(Bytes.of list)` — construct a byte sequence from a list of `Int64` in `0..=255`. Folds only
/// a compile-time-visible `Core::ListNew` operand (a runtime list source is a later increment → declines
/// cleanly). Each element must fold to a constant in range: a value `< 0` or `> 255` is a compile-time
/// trap (CDZ0304, matching the runtime `bytes-set` guard — `numeric-model.md` §A Constant Operation With
/// No Value Is Rejected At Compile Time); a non-constant element declines (its `Bytes.of` can't be baked
/// yet). On success produces `Core::BytesOf { elems }` carrying the element occurrences — the backend
/// bakes/builds the sequence. A poison list propagates.
fn lower_bytes_of(db: &mut Db, id: StructId, list: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, list) {
        return Core::Poison(r);
    }
    let Core::ListNew { elems } = core_of(db, list) else {
        // A runtime list (a parameter, a push-built list) is a later increment — decline cleanly.
        return Core::Poison(Reject::decline(
            "Bytes.of of a runtime list is not yet supported (only a visible list literal)",
        ));
    };
    // Each element is a `UInt8` (the `Bytes.of : (List UInt8) → Bytes` scheme). A CONSTANT element
    // outside `0..=255` is not a UInt8 — reject it as an OUT-OF-RANGE WIDTH literal (CDZ0302), NOT a
    // runtime trap: under the UInt8 model an ill-typed byte cannot be constructed at all, and to truncate
    // a wider value into a byte the program writes `(UInt8.wrap n)` explicitly. (The list-element
    // width-check does not yet flow the UInt8 bound through `(list …)` unification on its own, so the
    // constant bound is enforced here — with the width code, matching the type story.) A RUNTIME element
    // (a `UInt8` param, or `(UInt8.wrap n)`) is IN RANGE BY ITS TYPE and passes through — `select` emits
    // its i32 value into `bytes-set`, so `(Bytes.of (list (UInt8.wrap n)))` builds a byte from a runtime
    // value (the LEB128 encoder). The `Core::BytesOf` is built either way; a CONSTANT one bakes at escape
    // (R1), a RUNTIME one builds on the rope heap + escapes via the looping walker (L2b).
    for &e in &elems {
        match core_of(db, e) {
            Core::Poison(r) => return Core::Poison(r),
            Core::ConstInt(v) => match v.to_i64() {
                Some(n) if (0..=255).contains(&n) => {}
                _ => {
                    trace!(target: "rcdzc::fold", node = id.0, "Bytes.of element is not a UInt8 → CDZ0302");
                    return Core::Poison(Reject::coded(
                        Code::IntOutOfRange,
                        "a byte must be a UInt8 (0..=255); truncate a wider value with UInt8.wrap",
                    ));
                }
            },
            // A runtime UInt8 element — in range by its type; `select` emits its value into `bytes-set`.
            _ => {}
        }
    }
    trace!(target: "rcdzc::lower", node = id.0, len = elems.len(), "Bytes.of → Core::BytesOf");
    Core::BytesOf { elems }
}

/// Lower `(Bytes.at bytes index)` — the fallible indexed byte read. FOLD when `bytes` is a visible
/// `Core::BytesOf` AND `index` folds to a constant: an in-range index (`0 <= i < len`) yields `(Some
/// byte)` — a `Core::SumNew` at the `Some` disc carrying the byte as a constant `Int64` — and an
/// out-of-range index (negative or `>= len`) yields `None`. Otherwise emit the runtime `Core::BytesAt`
/// (a bounds-checked `bytes-get`). Mirrors `lower_list_at`, but the element is always a byte → `Int64`.
fn lower_bytes_at(db: &mut Db, id: StructId, bytes: StructId, index: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, bytes) {
        return Core::Poison(r);
    }
    if let Core::Poison(r) = core_of(db, index) {
        return Core::Poison(r);
    }
    let Some((disc_some, disc_none)) = option_discs(db, id) else {
        return Core::Poison(Reject::decline(
            "Bytes.at result is not the built-in Option sum",
        ));
    };
    // FOLD a constant `Bytes.of` indexed by a constant integer.
    if let (Core::BytesOf { elems }, Core::ConstInt(i)) = (core_of(db, bytes), core_of(db, index)) {
        match i.to_i64() {
            Some(n) if n >= 0 && (n as usize) < elems.len() => {
                // The byte at `n` is a constant `Int64` element occurrence — its own core is the payload.
                trace!(target: "rcdzc::fold", node = id.0, index = n, "Bytes.at folds to Some (in-bounds constant index)");
                return Core::SumNew {
                    disc: disc_some,
                    payloads: vec![elems[n as usize]],
                };
            }
            _ => {
                trace!(target: "rcdzc::fold", node = id.0, "Bytes.at folds to None (out-of-bounds constant index)");
                return Core::SumNew {
                    disc: disc_none,
                    payloads: Vec::new(),
                };
            }
        }
    }
    // A runtime bytes or runtime index — emit the bounds-checked runtime read.
    Core::BytesAt {
        bytes,
        index,
        disc_some,
        disc_none,
    }
}

/// Lower `(Bytes.concat a b)`. FOLD when BOTH operands are visible `Core::BytesOf` literals: the result
/// is a single `Core::BytesOf` whose elements are `a`'s then `b`'s (each already a range-checked constant
/// byte occurrence), so a constant concat bakes with no runtime op. Otherwise emit `Core::BytesConcat`. A
/// poison operand propagates.
fn lower_bytes_concat(db: &mut Db, lhs: StructId, rhs: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, lhs) {
        return Core::Poison(r);
    }
    if let Core::Poison(r) = core_of(db, rhs) {
        return Core::Poison(r);
    }
    if let (Core::BytesOf { elems: a }, Core::BytesOf { elems: b }) =
        (core_of(db, lhs), core_of(db, rhs))
    {
        let mut elems = a;
        elems.extend(b);
        trace!(target: "rcdzc::fold", len = elems.len(), "Bytes.concat folds two constant sequences");
        return Core::BytesOf { elems };
    }
    Core::BytesConcat { lhs, rhs }
}

/// Lower `(Bytes.slice bytes start len)` — the fallible sub-range read. Emits the runtime
/// `Core::BytesSlice`, which bounds-checks (`start >= 0`, `len >= 0`, `start + len <= bytes-len`) and
/// yields `Some(bytes-slice)` in range / `None` out — never trapping (the runtime `bytes-slice` traps on
/// OOB, so the emit guards first). A provably-out-of-range CONSTANT slice folds to `None` here (a cheap
/// safe fold); an in-range constant slice does NOT fold to a baked `Some(Bytes)` — its payload is a
/// sub-sequence, which would need a synthesized `Core::BytesOf` payload occurrence — so it takes the
/// runtime path (correct, just imports the runtime). The `Some(Bytes)` payload is a Bytes HANDLE, used
/// directly (no box). Mirrors `lower_bytes_at`'s shape; the constant-Some fold is a later refinement.
fn lower_bytes_slice(
    db: &mut Db,
    id: StructId,
    bytes: StructId,
    start: StructId,
    len: StructId,
) -> Core {
    for op in [bytes, start, len] {
        if let Core::Poison(r) = core_of(db, op) {
            return Core::Poison(r);
        }
    }
    let Some((disc_some, disc_none)) = option_discs(db, id) else {
        return Core::Poison(Reject::decline(
            "Bytes.slice result is not the built-in Option sum",
        ));
    };
    // A provably-out-of-range CONSTANT slice folds to `None` (safe, no synthesized payload needed).
    if let (Core::BytesOf { elems }, Core::ConstInt(s), Core::ConstInt(l)) =
        (core_of(db, bytes), core_of(db, start), core_of(db, len))
    {
        let n = elems.len() as i128;
        let in_range = match (s.to_i64(), l.to_i64()) {
            (Some(s), Some(l)) if s >= 0 && l >= 0 => (s as i128) + (l as i128) <= n,
            _ => false,
        };
        if !in_range {
            trace!(target: "rcdzc::fold", node = id.0, "Bytes.slice folds to None (out-of-range constant)");
            return Core::SumNew {
                disc: disc_none,
                payloads: Vec::new(),
            };
        }
    }
    Core::BytesSlice {
        bytes,
        start,
        len,
        disc_some,
        disc_none,
    }
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
        Prim::StringTy => "String",
        Prim::SumNew => "sum-new",
        Prim::SumCtor => "sum-ctor",
        Prim::TupleNew => "tuple-new",
        Prim::RecordNew => "record-new",
        Prim::ListNew => "list-new",
        Prim::ListLen => "list-len",
        Prim::ListPush => "list-push",
        Prim::ListConcat => "list-concat",
        Prim::ListUpdate => "list-update",
        Prim::ListAt => "list-at",
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
        Prim::StrConcat => "str-concat",
        Prim::StrSlice => "str-slice",
        Prim::SumExpect => "sum-expect",
        Prim::CheckedAdd => "checked-add",
        Prim::CheckedMul => "checked-mul",
        Prim::WrappingAdd => "wrapping-add",
        Prim::WrappingMul => "wrapping-mul",
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
}
