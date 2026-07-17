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
//! PLAIN `@requires` only. `@requires` never binds `it` and is never rewritten by `proptest_gen`, so there
//! is NO double-injection risk with v-property-testing's still-present `@test @ensures` rewrite. The
//! universal `@ensures` enforcement (plain AND `@test`-stacked, via a shared `(let ((it BODY)) (if Q it
//! (trap)))` helper, with v-property-testing deleting their `rewrite_ensures_stacked_tests` in lockstep)
//! is the immediately-following increment once this pass is on trunk.

use crate::ast::{Arenas, Leaf, Struct, StructId};
use crate::prelude::{push_atom, push_list};

/// Append a bare-`Name` atom occurrence (the `Arenas`-level shorthand, twin of `proptest_gen::name`).
fn name(ast: &mut Arenas, n: &str) -> StructId {
    push_atom(ast, Leaf::Name(n.to_string()))
}

/// Enforce every PLAIN `@requires` at body-entry: rewrite `(@ (requires PRE) (def SIG BODY))` so the def
/// body becomes `(if PRE BODY (trap "@requires precondition failed"))`. A no-op for a program with no plain
/// `@requires`. See the module docs for ordering + scope.
pub(crate) fn enforce(ast: &mut Arenas) {
    // Only ORIGINAL nodes can be a source annotation; the rewrite APPENDS (if/trap/name nodes), so bound the
    // scan to the pre-pass length — the appended nodes are never themselves a `(@ (requires …) def)` to scan.
    let original_len = ast.structure.len() as u32;
    for i in 0..original_len {
        let id = StructId(i);
        // `(@ NAME INNER)` — the annotation head. A plain `@requires` is `(@ (requires PRE) (def SIG BODY))`,
        // so NAME is the application `(requires PRE)` (a list), INNER is the def.
        let Some(tail) = ast.as_form(id, "@") else {
            continue;
        };
        let (Some(&name_occ), Some(&inner)) = (tail.first(), tail.get(1)) else {
            continue;
        };
        // NAME must be the application `(requires PRE)` with EXACTLY one predicate argument. A malformed
        // `@requires()`/`@requires(a b)` is left for `strip_annotations`' arity check (b4a2, CDZ0201) — this
        // pass only rewrites the well-formed single-predicate form.
        let Some(&pred) = ast.as_form(name_occ, "requires").and_then(|t| match t {
            [only] => Some(only),
            _ => None,
        }) else {
            continue;
        };
        // INNER is USUALLY `(def SIG BODY …)`, but a def may STACK several verification/other annotations —
        // `(@ (requires P1) (@ (requires P2) (def …)))` — in which case this `@requires`' INNER is itself an
        // `(@ …)` wrapper, not the def. Descend through any number of intervening `(@ NAME INNER2)` layers to
        // the actual def occ, so a STACKED outer `@requires` still finds its def and enforces (a single scan
        // otherwise skips every `@requires` whose inner is another wrapper — only the innermost would enforce,
        // silently dropping the outer preconditions). Each stacked `@requires` is processed at its own index
        // and wraps the def's CURRENT body, so the layers compose: `(if P1 (if P2 BODY trap) trap)`.
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
        // Any def tail beyond [SIG, BODY] (a trailing form) is preserved after the rewritten body.
        let trailing: Vec<StructId> = def_tail.get(2..).unwrap_or(&[]).to_vec();
        // Build `(if PRE BODY (trap "@requires precondition failed"))`. PRE is the SHARED predicate node
        // (also read by `strip_annotations` into `db.requires`) — reusing it is safe: it is evaluated at
        // body-entry where every param it references is in scope, and it is a pure boolean expression. The
        // trap arm is the canonical `unreachable` halt (same shape as the `@test @ensures` false branch).
        let checked_body = {
            let if_head = name(ast, "if");
            let trap_call = {
                let trap_head = name(ast, "trap");
                let msg = push_atom(ast, Leaf::Str("@requires precondition failed".to_string()));
                push_list(ast, vec![trap_head, msg])
            };
            push_list(ast, vec![if_head, pred, body, trap_call])
        };
        // Rewrite the def in place: `(def SIG checked_body TRAILING…)`. Overwriting `def_occ` (the actual def
        // node id, reached by descending any stacked `@` wrappers) keeps every enclosing `(@ (requires PRE) …)`
        // child pointer valid, so `strip_annotations` still sees each wrapper + records its `PRE`. When several
        // `@requires` stack, each is processed at its own index and re-wraps the def's CURRENT body, so the
        // checks nest — `(if P_outer (if P_inner BODY trap) trap)` — and ALL preconditions are enforced.
        let def_head = name(ast, "def");
        let mut new_children = vec![def_head, sig, checked_body];
        new_children.extend(trailing);
        ast.structure[def_occ.0 as usize] = Struct::List(new_children);
    }
}
