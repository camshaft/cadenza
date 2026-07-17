//! VERIFICATION-ANNOTATION ENFORCEMENT (Inc-b (D), test-tier) — turn a PLAIN `@requires`/`@ensures` into a
//! runtime check the def actually performs, so a violated pre/post-condition TRAPS rather than silently
//! producing a value. This is the (D) short-term path the operator confirmed (2026-07-17): the annotations
//! verify AT RUN TIME now; proof-guided ELISION (compiling the check away when a proof discharges it) waits
//! for the bounded compile-time kernel interpreter (A). A dormant-correct stub, not a fake — the check runs.
//!
//! ## What this pass does
//!
//! A PLAIN `@requires` on a def — `(@ (requires PRE) (def SIG BODY))`, NOT stacked under `@test` — is
//! rewritten so the def body checks `PRE` at BODY-ENTRY and traps when it is false:
//!
//! ```text
//! (@ (requires PRE) (def SIG BODY))  ->  (@ (requires PRE) (def SIG (if PRE BODY (trap "…"))))
//! ```
//!
//! The `(@ (requires PRE) …)` WRAPPER is left in place — `strip_annotations` (which runs LATER in the load
//! sequence) still records the predicate into `db.requires` for the verification layer. This pass only
//! rewrites the def's BODY; it does not consume the annotation.
//!
//! `@requires` enforces at BODY-ENTRY (once, when the function runs), matching the Hoare-logic reading
//! `{PRE} body {POST}` — NOT at each call site (which is both harder and wrong for higher-order / indirect
//! calls, where there is no syntactic call to instrument). A precondition references only the def's
//! PARAMETERS (and prelude/global names), all in scope at body entry, so `(if PRE BODY (trap))` type-checks
//! and resolves exactly as hand-written code.
//!
//! ## Ordering (load sequence, `Db::load`)
//!
//! Runs BEFORE `proptest_gen::synthesize` and BEFORE `strip_annotations`:
//! - before `synthesize` so a `@test` over a `@requires`-constrained param sees the already-checked body
//!   (the seam agreed with v-property-testing: enforcement first, generator-wrap second);
//! - before `strip_annotations` so the `(@ (requires …) (def …))` wrapper is still present to detect.
//!
//! ## Scope of THIS increment
//!
//! BOTH `@requires` and `@ensures`, universally (plain AND stacked under `@test`/other annotations):
//! - `@requires(PRE)` → body `(if PRE BODY (trap "@requires precondition failed"))` — precondition at entry;
//! - `@ensures(Q)` → body `(let ((it BODY)) (if Q it (trap "@ensures postcondition failed")))` — postcondition
//!   at exit, VALUE-TRANSPARENT (the pass arm returns `it`, the def's own value, not `unit`).
//!
//! Both wrap the def's CURRENT body and re-wrap at each annotation's own index, so stacked/mixed contracts
//! COMPOSE regardless of order: `@requires(P) @ensures(Q)` over `(def SIG BODY)` becomes (after both fire)
//! `(if P (let ((it BODY)) (if Q it trap)) trap)` or the symmetric nesting — every contract enforced.
//!
//! `@ensures` binds the result to `it`; if a PARAM is literally named `it`, the injected `(let ((it …)) …)`
//! would SHADOW it — such a def is SKIPPED for `@ensures` (the capture guard, mirroring `proptest_gen`).
//!
//! LOCKSTEP with v-property-testing (INTERIM — no-overlap): v-property-testing's `rewrite_ensures_stacked_tests`
//! still owns the `@test`-stacked `@ensures` form — `(@ test (@ (ensures Q) (def …)))` — on trunk today. To
//! avoid DOUBLE-INJECTION (both passes wrapping the same body → "expression nests too deeply"), this pass
//! DELIBERATELY SKIPS an `@ensures` whose node is the DIRECT inner of a `@test`/`@exhaustive` wrapper (their
//! exact match shape) and enforces only BARE `@ensures` (no enclosing `@test`). `@requires` has no such
//! overlap (their pass never touches it), so it is enforced universally including under `@test`. When they
//! DELETE `rewrite_ensures_stacked_tests` in lockstep (after this lands), the `@test`-stacked skip here is
//! lifted in a follow-up so this pass enforces `@ensures` fully universally — one owner. Until then: bare
//! `@ensures` = mine, `@test @ensures` = theirs, ZERO overlap.

use crate::ast::{Arenas, Leaf, Struct, StructId};
use crate::prelude::{push_atom, push_list};

/// Append a bare-`Name` atom occurrence (the `Arenas`-level shorthand, twin of `proptest_gen::name`).
fn name(ast: &mut Arenas, n: &str) -> StructId {
    push_atom(ast, Leaf::Name(n.to_string()))
}

/// True if `param` is (or is a typed `(: it Ty)` whose name is) literally `it` — the `@ensures` capture guard.
/// Mirrors `proptest_gen::param_is_named_it` (kept local so this pass has no cross-module coupling).
fn param_is_named_it(ast: &Arenas, param: StructId) -> bool {
    if ast.as_name(param) == Some("it") {
        return true;
    }
    ast.as_form(param, ":")
        .and_then(|t| t.first())
        .and_then(|&nm| ast.as_name(nm))
        == Some("it")
}

/// True if the def SIG `[NAME, p0, p1, …]` has any parameter literally named `it` (bare or typed).
fn sig_binds_it(ast: &Arenas, sig: StructId) -> bool {
    match ast.get(sig) {
        Struct::List(items) => items.iter().skip(1).any(|&p| param_is_named_it(ast, p)),
        _ => false,
    }
}

/// Enforce every `@requires`/`@ensures` contract at run time by rewriting the annotated def's body. A no-op
/// for a program with no verification annotation. See the module docs for the shapes, composition, and the
/// `@ensures` `it`-capture guard.
pub(crate) fn enforce(ast: &mut Arenas) {
    // Only ORIGINAL nodes can be a source annotation; the rewrite APPENDS (if/let/trap/name nodes), so bound
    // the scan to the pre-pass length — appended nodes are never themselves a `(@ (requires|ensures …) def)`.
    let original_len = ast.structure.len() as u32;
    // PRE-SCAN (interim no-overlap with v-property-testing): collect every `@ensures`-head node that is the
    // DIRECT inner of a `@test`/`@exhaustive` wrapper — `(@ test (@ (ensures Q) …))`. Their
    // `rewrite_ensures_stacked_tests` owns exactly this shape today, so this pass must NOT also rewrite it
    // (double injection → over-deep nesting). We skip these `@ensures` nodes below; bare `@ensures` (no
    // enclosing test) is ours. `@requires` is never in this set (their pass ignores it).
    let mut test_stacked_ensures: crate::fxhash::FxHashSet<StructId> =
        crate::fxhash::FxHashSet::default();
    for i in 0..original_len {
        let outer = StructId(i);
        let Some(t) = ast.as_form(outer, "@") else {
            continue;
        };
        let (Some(&outer_name), Some(&inner)) = (t.first(), t.get(1)) else {
            continue;
        };
        if !matches!(ast.as_name(outer_name), Some("test") | Some("exhaustive")) {
            continue;
        }
        // `inner` is the annotated form directly under `@test`/`@exhaustive`. If it is an `(@ (ensures …) …)`
        // head, mark that node — their pass will rewrite it, so we skip it.
        if ast
            .as_form(inner, "@")
            .and_then(|it| it.first())
            .and_then(|&head| ast.as_form(head, "ensures"))
            .is_some()
        {
            test_stacked_ensures.insert(inner);
        }
    }
    for i in 0..original_len {
        let id = StructId(i);
        // `(@ NAME INNER)` — the annotation head. A verification annotation is `(@ (requires PRE) …)` or
        // `(@ (ensures Q) …)`, so NAME is the application `(requires PRE)` / `(ensures Q)` (a list), INNER the
        // annotated form.
        let Some(tail) = ast.as_form(id, "@") else {
            continue;
        };
        let (Some(&name_occ), Some(&inner)) = (tail.first(), tail.get(1)) else {
            continue;
        };
        // Which verification annotation is this, and its single PREDICATE argument? A malformed
        // `@requires()`/`@ensures(a b)` is left for `strip_annotations`' arity check (b4a2, CDZ0201) — this
        // pass only rewrites the well-formed single-predicate form. `is_ensures` selects the wrap shape below.
        let requires_pred = ast.as_form(name_occ, "requires").and_then(|t| match t {
            [only] => Some(*only),
            _ => None,
        });
        let ensures_pred = ast.as_form(name_occ, "ensures").and_then(|t| match t {
            [only] => Some(*only),
            _ => None,
        });
        let (pred, is_ensures) = match (requires_pred, ensures_pred) {
            (Some(p), _) => (p, false),
            (_, Some(q)) => (q, true),
            _ => continue, // not a verification annotation head — leave untouched
        };
        // INTERIM no-overlap: a `@test`/`@exhaustive`-stacked `@ensures` is v-property-testing's to rewrite
        // (`rewrite_ensures_stacked_tests`); skip it here to avoid double-injection. Bare `@ensures` is ours.
        if is_ensures && test_stacked_ensures.contains(&id) {
            continue;
        }
        // INNER is USUALLY `(def SIG BODY …)`, but a def may STACK several verification/other annotations —
        // `(@ (requires P) (@ (ensures Q) (def …)))` — in which case this annotation's INNER is itself an
        // `(@ …)` wrapper, not the def. Descend through any number of intervening `(@ NAME INNER2)` layers to
        // the actual def occ, so a STACKED outer annotation still finds its def and enforces (a single scan
        // otherwise skips every annotation whose inner is another wrapper — only the innermost would enforce,
        // silently dropping the outer contracts). Each annotation is processed at its own index and wraps the
        // def's CURRENT body, so the layers compose: `(if P (let ((it BODY)) (if Q it trap)) trap)`.
        let mut def_occ = inner;
        while ast.as_form(def_occ, "def").is_none() {
            let Some(chain) = ast.as_form(def_occ, "@") else {
                break; // not a def and not an `@` wrapper — a non-def annotand, leave untouched below
            };
            let Some(&next_inner) = chain.get(1) else {
                break;
            };
            def_occ = next_inner;
        }
        // The descent target must be `(def SIG BODY …)` — a well-formed def has ≥2 tail elements (SIG, BODY).
        // An annotation around a non-def is a well-formedness concern handled elsewhere; leave it untouched.
        let Some(def_tail) = ast.as_form(def_occ, "def") else {
            continue;
        };
        let (Some(&sig), Some(&body)) = (def_tail.first(), def_tail.get(1)) else {
            continue;
        };
        // `@ensures` binds the result to `it`; if a param is literally named `it`, the injected
        // `(let ((it BODY)) …)` would SHADOW it and change the def's meaning — SKIP the `@ensures` rewrite for
        // such a def (the capture guard, mirroring `proptest_gen`). `@requires` never binds `it`, so it is
        // unaffected.
        if is_ensures && sig_binds_it(ast, sig) {
            continue;
        }
        // Any def tail beyond [SIG, BODY] (a trailing form) is preserved after the rewritten body.
        let trailing: Vec<StructId> = def_tail.get(2..).unwrap_or(&[]).to_vec();
        // Build the checked body. PRED is the SHARED predicate node (also read by `strip_annotations` into
        // `db.requires`/`db.ensures`) — reusing it is safe: it is a pure boolean expression evaluated where its
        // referenced names (params, and for `@ensures` the result binder `it`) are in scope. The trap arm is
        // the canonical `unreachable` halt.
        // - `@requires`: `(if PRE BODY (trap "@requires precondition failed"))` — checked at body-ENTRY.
        // - `@ensures`:  `(let ((it BODY)) (if Q it (trap "@ensures postcondition failed")))` — checked at
        //   body-EXIT, VALUE-TRANSPARENT (the pass arm returns `it`, the def's own value).
        let checked_body = if is_ensures {
            let it_binder = {
                let it_nm = name(ast, "it");
                let bind = push_list(ast, vec![it_nm, body]);
                push_list(ast, vec![bind])
            };
            let check = {
                let if_head = name(ast, "if");
                let it_use = name(ast, "it");
                let trap_call = {
                    let trap_head = name(ast, "trap");
                    let msg =
                        push_atom(ast, Leaf::Str("@ensures postcondition failed".to_string()));
                    push_list(ast, vec![trap_head, msg])
                };
                push_list(ast, vec![if_head, pred, it_use, trap_call])
            };
            let let_head = name(ast, "let");
            push_list(ast, vec![let_head, it_binder, check])
        } else {
            let if_head = name(ast, "if");
            let trap_call = {
                let trap_head = name(ast, "trap");
                let msg = push_atom(ast, Leaf::Str("@requires precondition failed".to_string()));
                push_list(ast, vec![trap_head, msg])
            };
            push_list(ast, vec![if_head, pred, body, trap_call])
        };
        // Rewrite the def in place: `(def SIG checked_body TRAILING…)`. Overwriting `def_occ` (the actual def
        // node id, reached by descending any stacked `@` wrappers) keeps every enclosing `(@ (requires|ensures
        // …) …)` child pointer valid, so `strip_annotations` still sees each wrapper + records its predicate.
        // When several annotations stack, each is processed at its own index and re-wraps the def's CURRENT
        // body, so the checks nest and ALL contracts are enforced.
        let def_head = name(ast, "def");
        let mut new_children = vec![def_head, sig, checked_body];
        new_children.extend(trailing);
        ast.structure[def_occ.0 as usize] = Struct::List(new_children);
    }
}
