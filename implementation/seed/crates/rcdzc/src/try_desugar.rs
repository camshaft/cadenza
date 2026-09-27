//! Pre-resolution desugar of a leading `try` value-def in a `do` block to an equivalent `let`.
//!
//! A two-form `(do (def x (try e)) body)` is SEMANTICALLY a `(let ((x (try e))) body)` — a do-local
//! value declaration is scoped to the forms that follow it, so a leading value-def whose only successor
//! is the body IS a `let` binding whose body is that form. Both spellings route through the same BRICK-3b
//! runtime-`?` `lower_let` short-circuit and build the SAME `Core::MatchSum` boundary.
//!
//! WHY REWRITE IT — the do-def form is FRAGILE under INLINING. When the enclosing function is β-copied at
//! a call site, a do-local reference resolves via `resolve::do_local_binds`' scope-ascent, and under a
//! NESTED inline that ascent can land on the ORIGINAL do-block rather than the copy — so the copied
//! `try`'s BRICK-3b `core_override` (a `SumPayload` on the copied init) is missed, the copied MatchSum's
//! success-arm body re-lowers the reference through the generic `Resolved::Try` path, and the boundary
//! re-declines `CDZ0900`. That decline is a SAFE FLOOR over a latent miscompile: naively un-declining it
//! (returning the payload read) drops the short-circuit and reads a disconnected scrutinee on the failure
//! leg (`23-try-operator.sexp` tdd1: `main`'s `n=5` leg returned `0` instead of `-1`). A `let` binding's
//! reference, by contrast, resolves ROBUSTLY through the β-copy (its binder is recognized structurally by
//! `resolve::binder_in` Cases 1/2), so the `let` twin compiles AND runs correctly whether inlined or not.
//!
//! Rewriting the `try` do-def to a `let` makes the `try` shape INLINING-INVARIANT (v-core-opt's endorsed
//! direction — order/inline-dependence IS the fragility), producing the identical MatchSum whether inlined
//! or not. TRY-SPECIFIC: only a `(def x (try …))` value-def with exactly a body successor is rewritten, so
//! a non-`try` do-def keeps the do-fold's copy-propagation path unchanged and the blast radius is minimal.
//!
//! Runs at LOAD, BEFORE the parent index and resolution (alongside `reify_quotes` / `desugar_eval`), so the
//! rewritten `let` resolves like hand-written source.

use crate::ast::{Arenas, Leaf, Struct, StructId};
use crate::prelude::{push_atom, push_list};

/// Rewrite every two-form `(do (def x (try e)) body)` in `ast` to `(let ((x (try e))) body)` in place.
pub fn desugar_try_do_defs(ast: &mut Arenas) {
    // FAST BAIL for a program with no `(try …)` form (the overwhelming common case). A `(def x (try e))`
    // needs a `try` head, which is a `Leaf::Name("try")` in the interned leaf pool; if none exists, no
    // `try` form exists anywhere and the whole scan is dead work. A single O(leaves) prescan (leaves are
    // interned once, far fewer than a per-node structural probe) over-approximates: it falls through
    // spuriously only for a program that MENTIONS the identifier `try` without a `(try …)` form (e.g. a
    // user def named `try`), which then runs the exact shape scan below — same result, just not skipped.
    // Sibling of the `quote::reify_quotes` / `eval_ast::desugar_eval` fast-bails.
    if !ast
        .leaves
        .iter()
        .any(|l| matches!(l, Leaf::Name(n) if n.as_ref() == "try"))
    {
        return;
    }
    // Only ORIGINAL nodes can be a source do-block; the rewrite APPENDS the `let` scaffolding (ids >= this
    // bound) and overwrites the do node in place, so the scan must not consider its own output.
    let original_len = ast.structure.len() as u32;
    // Plan `(do_node, def_form, x_name, try_node, body)` for each two-form `(do (def x (try e)) body)`.
    let mut plans: Vec<(StructId, StructId, StructId, StructId, StructId)> = Vec::new();
    for i in 0..original_len {
        let id = StructId(i);
        // A `(do …)` block with EXACTLY two tail forms — a value-def and a body. A do-block with other
        // statements (an effect before/after the def) is NOT a simple `let` (the extra forms are sequenced
        // for effect), so it is left to the ordinary do-fold; only the clean two-form idiom is rewritten.
        let Some(tail) = ast.as_form(id, "do") else {
            continue;
        };
        let [def_form, body] = tail else {
            continue;
        };
        let (def_form, body) = (*def_form, *body);
        // The first form must be a bare-name value-def `(def x V)` whose value `V` is a `(try …)` form.
        let Some(def_tail) = ast.as_form(def_form, "def") else {
            continue;
        };
        let [x_name, v] = def_tail else {
            continue;
        };
        let (x_name, v) = (*x_name, *v);
        // `x` is a bare NAME atom (a value-def) — not a `(f p…)` function signature (a List head).
        if ast.as_name(x_name).is_none() {
            continue;
        }
        // `V` is a `(try e)` form (the whole point — a non-`try` value-def is left alone).
        if ast.as_form(v, "try").is_none() {
            continue;
        }
        plans.push((id, def_form, x_name, v, body));
    }
    for (do_node, def_form, x_name, try_node, body) in plans {
        // Build `(let ((x (try e))) body)`, REUSING the original `x_name` / `try_node` / `body` occurrences
        // so their subtrees (and the `try`'s operand) are shared, not re-copied.
        let pair = push_list(ast, vec![x_name, try_node]); // the binding pair `(x (try e))`
        let bindings = push_list(ast, vec![pair]); //            the bindings list `((x (try e)))`
        let let_head = push_atom(ast, Leaf::Name("let".into()));
        // Overwrite the `(do …)` node IN PLACE, preserving its `StructId`/span as the `let`'s node.
        ast.structure[do_node.0 as usize] = Struct::List(vec![let_head, bindings, body]);
        // Blank the now-dead `(def x (try e))` wrapper: `x_name` and `try_node` were re-parented into the
        // binding pair above, and the parent index (built later) records the LAST parent per child — leaving
        // the def form would out-rank the pair as their recorded parent and orphan the binder. Emptying it
        // leaves the binding pair the sole parent of `x_name`/`try_node` (mirrors the dead-wrapper blanking
        // in `reify_quotes` / `desugar_eval`).
        ast.structure[def_form.0 as usize] = Struct::List(Vec::new());
    }
}
