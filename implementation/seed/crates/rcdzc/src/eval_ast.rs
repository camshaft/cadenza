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

/// A pending eval rewrite: overwrite the `(eval …)` node's structure entry with `replacement`, then blank
/// the dead reified-argument wrapper nodes (`arg`'s subtree minus what the reconstruction still references).
struct EvalPlan {
    eval: StructId,
    arg: StructId,
    replacement: StructId,
}

/// Every node id reachable from `root` through the structure child lists (inclusive). Used to diff the
/// dead reified-argument subtree against the live reconstruction so the dead wrapper nodes can be blanked.
fn reachable(ast: &Arenas, root: StructId) -> std::collections::HashSet<u32> {
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if !seen.insert(n.0) {
            continue;
        }
        if let Struct::List(children) = ast.get(n) {
            stack.extend(children.iter().copied());
        }
    }
    seen
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
                arg,
                replacement,
            });
        }
    }
    for EvalPlan {
        eval,
        arg,
        replacement,
    } in plans
    {
        // The nodes the LIVE reconstruction references — the copy about to be written into `eval` shares
        // them, so they must stay intact. Computed BEFORE the overwrite (the reconstruction root's subtree),
        // and it includes any REUSED live splice operand (a let-bound name `,x`, a param) `reconstruct`
        // carried over from the eval argument.
        let live = reachable(ast, replacement);
        // The dead reified-ARGUMENT wrapper subtree: every `(Ast.List …)`/`(Ast.Int …)`/`(list …)`/`(. Ast
        // …)` node the eval's argument built (by `reify_quotes` or a hand-written `Ast.*`). Once the eval
        // node is overwritten with the reconstructed source, this whole tree is unreachable EXCEPT for the
        // live splice operands the reconstruction reused. Blank each dead wrapper: `parent_index` records
        // the LAST (highest-id) parent per child, and a reified `(Ast.Int x)` wrapper (higher id than the
        // eval node) would otherwise remain `x`'s recorded parent — an orphan whose own parent is `None`,
        // so a scope walk from `x` dead-ends and a lexically-scoped `,x` resolves as a spurious "unbound
        // name". Blanking the dead wrappers leaves the reconstruction (at the eval position) the sole parent
        // of each shared splice operand, so it resolves against the eval's enclosing `let`/`def`.
        for dead in reachable(ast, arg) {
            if !live.contains(&dead) && dead != eval.0 {
                ast.structure[dead as usize] = Struct::List(Vec::new());
            }
        }
        // Overwrite the `(eval …)` node with a COPY of the reconstruction root's structure, so the eval's
        // own `StructId` (and span) is preserved as the spliced-in form's node.
        let entry = ast.get(replacement).clone();
        ast.structure[eval.0 as usize] = entry;
        // BLANK the reconstruction root when it is a FRESH appended node (a `push_*`-built compound) — it is
        // now a duplicate of the copy written into `eval`, and (higher-id) would otherwise out-rank the eval
        // copy as the shared children's parent (the same orphan hazard, mirrored from `reify_quotes`). A
        // reconstruction returning an ORIGINAL node directly (id < original_len — the whole eval arg was a
        // single `(Ast.Int <payload>)`, so `reconstruct` returns `payload` live) is the live spliced value,
        // still reachable through the eval copy; the `live`-set diff above already kept it, so don't blank.
        if replacement.0 >= original_len {
            ast.structure[replacement.0 as usize] = Struct::List(Vec::new());
        }
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
    // `(Ast.Int payload)` -> the payload AS SOURCE. `Ast.Int` arises two ways, both reconstructing to the
    // payload node itself: (a) a reified INTEGER LITERAL `(Ast.Int 42)` — payload is the literal `42`, whose
    // source is `42`; (b) an ACTIVE-UNQUOTE lift `(Ast.Int <e>)` where `reify_active` wrapped the unquote's
    // LIVE operand `<e>` (a name `,x`, a computed `,(+ 1 2)`) — its source is `<e>` itself. So unwrapping the
    // `Ast.Int` back to `payload` reconstructs both: a splice of a compile-time-known VARIABLE or expression
    // (the core macro idiom — `(eval `(+ ,x 4))` with `x`=3 → `(+ x 4)` → folds to 7) reconstructs like a
    // literal splice, rather than leaving the `(eval …)` un-desugared (its head `eval` then a misleading
    // "unbound name `eval`"). The payload node is REUSED live: for (b) it is the evaluated code, which must
    // resolve against the `(eval …)`'s enclosing scope, so it is spliced in unchanged (not copied). The
    // reconstructed source folds through the ordinary compile-time path; a payload that is NOT compile-time-
    // known then declines/errors there as ordinary code would, not here.
    if let Some(payload) = ast_ctor_arg(ast, node, "Int") {
        return Some(payload);
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
