//! EVAL DESUGAR — turn `(eval AST-VALUE)` into the SOURCE FORM that `AST-VALUE` denotes, then let the
//! ordinary pipeline resolve/type/lower it (`metaprogramming.md` §Eval Is Optional For Macros And
//! Interactive Use). `eval` executes an AST value AS code; because compile-time evaluation is ONE TIER
//! (`metaprogramming.md` §Compile-Time Evaluation Is One Tier — macro expansion, generic reduction,
//! folding are the SAME mechanism), evaluating a COMPILE-TIME-VISIBLE AST is exactly macro expansion:
//! reconstruct the syntax the AST denotes and splice it in, so `(eval (quote (+ 1 2)))` becomes `(+ 1 2)`
//! and folds to `3` through the ordinary path — no separate interpreter.
//!
//! This is the INVERSE of [`crate::quote::reify_quotes`]. That pass rewrites `(quote FORM)` into the
//! `Ast` construction that BUILDS `FORM`'s value; this pass reads such a construction back into `FORM`.
//! Running AFTER `reify_quotes` means `(eval (quote (+ 1 2)))` has already become `(eval (Ast.List (list
//! (Ast.Name "+") (Ast.Int 1) (Ast.Int 2))))`, so ONE reconstruction handles both the quoted spelling
//! and a hand-written `(Ast.* …)` argument — they are the same nodes by then.
//!
//! ```text
//! (Ast.Int N)               ->  N                        -- the integer literal
//! (Ast.Name "foo")          ->  foo                      -- the bare name
//! (Ast.List (list a b c))   ->  (<recon a> <recon b> …)  -- the compound form
//! ```
//!
//! Only a fully-reconstructable, COMPILE-TIME-VISIBLE argument is rewritten (the `Ast` sum's `Int`/`Name`/
//! `List` variants over `(list …)` literals — the same three the reifier produces). A `eval` of a RUNTIME
//! or non-constant AST value is LEFT UNTOUCHED for `resolve` to decline (the compiler does not execute a
//! dynamically-constructed AST — `metaprogramming.md`: "the compiler constructs and analyzes AST but does
//! not execute dynamically-constructed AST"). A malformed reconstruction (an empty `Ast.List` — a compound
//! with no operator) is rewritten to `(trap "malformed AST")`, the runtime halt the corpus records.
//!
//! ## Ordering / in-place rewrite
//!
//! Modelled on [`crate::quote::reify_quotes`] and [`crate::effects::desugar_handles`]: a scan collects
//! the rewrites, then they are applied by overwriting each `(eval …)` node's structure entry with its
//! reconstruction. Runs during `Db::load` AFTER `reify_quotes` (so a quoted argument is already an
//! `Ast.*` tree) and BEFORE the parent index — so the spliced-in source resolves like hand-written code.

use crate::ast::{Arenas, Leaf, Struct, StructId};
use crate::prelude::{push_atom, push_list};

/// A pending eval rewrite: overwrite the `(eval …)` node's structure entry with `replacement`.
struct EvalPlan {
    eval: StructId,
    replacement: StructId,
}

/// Desugar every `(eval AST)` whose argument is a compile-time-visible `Ast` construction into the source
/// form that AST denotes (see the module docs). A non-reconstructable argument is left for `resolve`.
///
/// The compiler does NOT require `eval` to compile programs, and it does NOT execute a dynamically-built
/// AST: only a COMPILE-TIME-VISIBLE `Ast` construction is reconstructed to source; a runtime/non-constant
/// AST argument is left for `resolve` to decline. And because `eval` folds the reconstructed source
/// through the ordinary compile-time path (the SAME tier as generic reduction, monomorphization, and
/// constant folding), there is one place the meaning of compile-time computation lives.
//= spec/capabilities/metaprogramming.md#eval-is-optional-for-macros-and-interactive-use
//# The compiler MUST NOT require `eval` to compile programs — the compiler constructs and analyzes AST but does not execute dynamically-constructed AST.
//= spec/capabilities/metaprogramming.md#compile-time-evaluation-is-one-tier
//# Macro expansion, generic reduction, monomorphization, and constant folding MUST be the same compile-time evaluation mechanism rather than separate subsystems, so that there is one place the meaning of compile-time computation lives and the four cannot drift apart.
pub fn desugar_eval(ast: &mut Arenas) {
    // Only ORIGINAL nodes can be a source `(eval …)`; reconstruction APPENDS, so bound the scan.
    let original_len = ast.structure.len() as u32;
    let mut plans: Vec<EvalPlan> = Vec::new();
    for i in 0..original_len {
        let id = StructId(i);
        // Match `(eval ARG)` — head name `eval`, exactly one argument. Clone the slice's ids so the
        // borrow ends before the reconstruction's `&mut ast`.
        let Some(arg) = ast.as_form(id, "eval").and_then(|tail| match tail {
            [only] => Some(*only),
            _ => None,
        }) else {
            continue;
        };
        if let Some(replacement) = reconstruct(ast, arg) {
            plans.push(EvalPlan {
                eval: id,
                replacement,
            });
        }
    }
    for EvalPlan { eval, replacement } in plans {
        let entry = ast.get(replacement).clone();
        ast.structure[eval.0 as usize] = entry;
    }
}

/// Reconstruct the source form an `Ast` construction `node` denotes — the inverse of the reifier's map.
/// Returns the root of the fresh reconstructed tree, or `None` if `node` is not a fully compile-time-
/// visible `Ast.*` construction (then the `(eval …)` is left for `resolve` to decline).
///
/// An empty `Ast.List` (a compound with no operator — malformed AST) reconstructs to `(trap "malformed
/// AST")`: eval of a malformed AST is a runtime halt (`metaprogramming.md` §Eval Is Optional: "eval on
/// malformed AST traps"), not a value.
fn reconstruct(ast: &mut Arenas, node: StructId) -> Option<StructId> {
    // `(Ast.Int payload)` -> the integer payload verbatim. The head is the projection `(. Ast Int)`.
    if let Some(payload) = ast_ctor_arg(ast, node, "Int") {
        // The payload must be an integer literal (a constant AST leaf); anything else is not reconstructable.
        if matches!(ast.get(payload), Struct::Atom(l) if matches!(ast.leaf(*l), Leaf::Int { .. })) {
            let leaf = match ast.get(payload) {
                Struct::Atom(l) => ast.leaf(*l).clone(),
                _ => unreachable!(),
            };
            return Some(push_atom(ast, leaf));
        }
        return None;
    }
    // `(Ast.Name payload)` -> the bare name the String payload spells. `Ast.Name` carries the identifier
    // as a String (the reifier turned a `Leaf::Name` into a `Leaf::Str`); reconstruction turns it back.
    if let Some(payload) = ast_ctor_arg(ast, node, "Name") {
        let name = ast.as_str(payload)?.to_string();
        return Some(push_atom(ast, Leaf::Name(name)));
    }
    // `(Ast.List (list e…))` -> the compound form `(<recon e>…)`. An empty list is malformed (no operator).
    if let Some(payload) = ast_ctor_arg(ast, node, "List") {
        let elems = list_elems(ast, payload)?;
        if elems.is_empty() {
            return Some(trap_form(ast, "malformed AST"));
        }
        let mut children = Vec::with_capacity(elems.len());
        for e in elems {
            children.push(reconstruct(ast, e)?);
        }
        return Some(push_list(ast, children));
    }
    None
}

/// If `node` is the constructor application `(Ast.<variant> payload)` — a list whose head is the
/// projection `(. Ast <variant>)` and which carries exactly one argument — that argument. The shape
/// `crate::quote::ast_ctor` builds and the reader produces for a hand-written `(Ast.<variant> x)`.
fn ast_ctor_arg(ast: &Arenas, node: StructId, variant: &str) -> Option<StructId> {
    let Struct::List(items) = ast.get(node) else {
        return None;
    };
    if items.len() != 2 {
        return None;
    }
    // The head is `(. Ast <variant>)` — a 3-element list `[., Ast, <variant>]`.
    let Struct::List(head) = ast.get(items[0]) else {
        return None;
    };
    if head.len() == 3
        && ast.as_name(head[0]) == Some(".")
        && ast.as_name(head[1]) == Some("Ast")
        && ast.as_name(head[2]) == Some(variant)
    {
        Some(items[1])
    } else {
        None
    }
}

/// The element occurrences of a `(list e…)` value-constructor form — the reader-shaped list literal the
/// reifier wraps `Ast.List`'s payload in. Accepts EITHER the `list` name head or the unshadowable `"list"`
/// string-ctor head (both denote the same list literal). `None` if `payload` is not a list form.
fn list_elems(ast: &Arenas, payload: StructId) -> Option<Vec<StructId>> {
    if let Some(tail) = ast.as_form(payload, "list") {
        return Some(tail.to_vec());
    }
    if let Some(tail) = ast.as_ctor_form(payload, "list") {
        return Some(tail.to_vec());
    }
    None
}

/// Build `(trap "MSG")` — the diverging halt an eval of a malformed AST reconstructs to. `trap` is the
/// prelude diverging primitive `∀a. String → a`, so it validates in the `(eval …)` result position.
fn trap_form(ast: &mut Arenas, msg: &str) -> StructId {
    let trap = push_atom(ast, Leaf::Name("trap".to_string()));
    let message = push_atom(ast, Leaf::Str(msg.to_string()));
    push_list(ast, vec![trap, message])
}
