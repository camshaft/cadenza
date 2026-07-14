//! Shared REPL module-assembly — the front-end half of "evaluate one expression against a buffer of
//! definitions" that EVERY calculator/REPL surface reuses.
//!
//! A REPL surface (the browser playground, the native `cdz calc`, a Raycast extension) does the same
//! thing: it holds a BUFFER of the user's definitions and, when the user types one more EXPRESSION,
//! assembles a single runnable program = the buffer's definitions + a synthesized nullary entry whose
//! body is that expression, exported as the sole entry. Compiling + running THAT yields the expression's
//! value — scalar or compound — exactly as a normal program's `main` would have.
//!
//! This module owns ONLY the surface-level assembly (arena in, arena out); it does not compile or run
//! (those are the compiler's / runtime's concern, held by whichever crate drives the REPL). Keeping the
//! assembly here — not duplicated per surface — is why the browser and native REPLs can never DRIFT in
//! how they build the program: `cdz-wasm::repl_eval` and the native `cdz calc` both call
//! [`assemble_repl_program`], so a change to the shell-unwrapping or the entry synthesis lands in both.
//!
//! The assembly is PURE (no I/O, no compiler types), so it is natively unit-testable in this crate.

use crate::ast::{Arenas, Builder, Struct, StructId};

/// The synthesized entry a REPL evaluation is wrapped in. Must be KEBAB-CASE: it becomes a
/// component-model export name, and the component model requires extern names in kebab case (an
/// underscore/camel name fails jco transpile with "not a valid extern name"). The `cdz-` prefix +
/// `-eval` make a collision with a reader's own definition very unlikely.
pub const REPL_ENTRY: &str = "cdz-repl-eval";

/// Does form `id` start with the name `head` (`(head …)`)?
fn is_form_head(src: &Arenas, id: StructId, head: &str) -> bool {
    matches!(src.get(id), Struct::List(kids) if kids.first().is_some_and(|&h| src.as_name(h) == Some(head)))
}

/// The buffer's top-level item forms (defs/types) — the definitions a REPL expression can call —
/// unwrapping whatever shell the buffer arrived in. The guide's editor wraps a snippet as a `(do item…)`
/// block; a hand-written program may use `(module NAME item…)`; and either may present a bare single
/// form. `(export …)` clauses are dropped (the REPL supplies its own sole export). Any leading shell
/// head (`do`/`module`) is skipped so only the real item forms remain.
pub fn buffer_items(src: &Arenas) -> Vec<StructId> {
    let root = src.root;
    match src.get(root) {
        // A `(do item…)` (guide wrap) or `(module NAME item…)` (hand-written) shell. `module` carries a
        // NAME child after the head; `do` does not — skip past the head, and for `module` the name too.
        Struct::List(kids)
            if is_form_head(src, root, "do") || is_form_head(src, root, "module") =>
        {
            let skip = if src.as_name(kids[0]) == Some("module") {
                2
            } else {
                1
            };
            kids.iter()
                .skip(skip)
                .copied()
                .filter(|&it| !is_form_head(src, it, "export"))
                .collect()
        }
        // A bare `(def …)` / `(type …)` buffer: keep it. A bare expression has nothing to call.
        _ if is_form_head(src, root, "def") || is_form_head(src, root, "type") => vec![root],
        _ => Vec::new(),
    }
}

/// The NAME a top-level `def` item binds, if `item` is a `def`. Two shapes: `(def (name param…) body)`
/// — a function, whose name is the head of the signature list — and `(def name body)` — a bare value
/// binding, whose name is the second child directly. Returns `None` for a non-`def` item (e.g. a
/// `type`) or a malformed one. Used by a REPL surface's name-completion (the buffer's callable names).
pub fn def_name(src: &Arenas, item: StructId) -> Option<String> {
    let Struct::List(kids) = src.get(item) else {
        return None;
    };
    if src.as_name(*kids.first()?) != Some("def") {
        return None;
    }
    let target = *kids.get(1)?;
    match src.get(target) {
        // `(def (name param…) body)` — the signature list; its head is the function name.
        Struct::List(sig) => sig
            .first()
            .and_then(|&h| src.as_name(h))
            .map(str::to_string),
        // `(def name body)` — a bare value binding.
        Struct::Atom(_) => src.as_name(target).map(str::to_string),
    }
}

/// The names of every top-level `def` the buffer declares, in source order — for a REPL's
/// autocomplete. Callable functions AND bare value bindings are both included.
pub fn defined_names(src: &Arenas) -> Vec<String> {
    buffer_items(src)
        .into_iter()
        .filter_map(|it| def_name(src, it))
        .collect()
}

/// Copy a subtree from `src` into `b`, preserving structure and leaf values. Leaves re-intern (dedup is
/// fine — an atom occurrence is what carries identity), lists rebuild child-by-child.
fn copy_subtree(b: &mut Builder, src: &Arenas, id: StructId) -> StructId {
    match src.get(id) {
        Struct::Atom(leaf_id) => b.atom_leaf(src.leaf(*leaf_id).clone()),
        Struct::List(kids) => {
            let copied: Vec<StructId> = kids.iter().map(|&k| copy_subtree(b, src, k)).collect();
            b.list(copied)
        }
    }
}

/// Assemble the runnable REPL program: the buffer's kept top-level items (defs/types, shell unwrapped
/// and exports dropped) plus a synthesized nullary entry `(def (cdz-repl-eval) <expr>)`, exported as
/// the sole entry, all in one fresh arena as a top-level `(do item… entry export)` block. The compiler
/// accepts a bare `(do …)` (exactly what the guide editor emits), so no `(module …)` wrapping is added.
///
/// `buffer` is the parsed definitions arena; `expr` is the parsed expression arena (its whole root is
/// the entry body). Both are copied AT THE AST LEVEL into the result — NOT string-spliced — so a string
/// literal containing parentheses (or any surface quirk) can't corrupt the assembly. The returned arena
/// is ready to `codec::encode` + hand to the compiler.
pub fn assemble_repl_program(buffer: &Arenas, expr: &Arenas) -> Arenas {
    let mut b = Builder::new();
    let do_head = b.name("do");
    let mut do_kids = vec![do_head];
    for it in buffer_items(buffer) {
        do_kids.push(copy_subtree(&mut b, buffer, it));
    }
    // The synthesized entry: `(def (cdz-repl-eval) <expr>)`.
    let entry_def_head = b.name("def");
    let entry_name = b.name(REPL_ENTRY);
    let entry_sig = b.list(vec![entry_name]); // `(cdz-repl-eval)` — a nullary signature
    let entry_body = copy_subtree(&mut b, expr, expr.root);
    let entry_def = b.list(vec![entry_def_head, entry_sig, entry_body]);
    do_kids.push(entry_def);
    // `(export cdz-repl-eval)`.
    let export_head = b.name("export");
    let export_name = b.name(REPL_ENTRY);
    let export_form = b.list(vec![export_head, export_name]);
    do_kids.push(export_form);

    let program = b.list(do_kids);
    b.finish(program)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse a SINGLE-form s-expr program into arenas (test helper). Uses `read` (not `read_all`) so a
    /// lone top-level form stays its own root — matching the production caller, whose buffer arena root
    /// is the form itself, not a synthetic `(do …)` wrap.
    fn parse(src: &str) -> Arenas {
        crate::sexpr::read(src).expect("valid single-form s-expr")
    }

    #[test]
    fn buffer_items_unwraps_a_do_shell_and_drops_exports() {
        // A `(do (def …) (def …) (export …))` buffer keeps the two defs, drops the export.
        let buf = parse("(do (def (f x) (* x 2)) (def g 5) (export f))");
        let items = buffer_items(&buf);
        assert_eq!(items.len(), 2, "two defs kept, export dropped");
        assert_eq!(def_name(&buf, items[0]).as_deref(), Some("f"));
        assert_eq!(def_name(&buf, items[1]).as_deref(), Some("g"));
    }

    #[test]
    fn buffer_items_unwraps_a_module_shell_skipping_the_name() {
        // `(module M (def …))` skips both the head AND the name child.
        let buf = parse("(module M (def (f) 1))");
        let items = buffer_items(&buf);
        assert_eq!(items.len(), 1);
        assert_eq!(def_name(&buf, items[0]).as_deref(), Some("f"));
    }

    #[test]
    fn buffer_items_keeps_a_bare_def_but_not_a_bare_expr() {
        let def_buf = parse("(def (f) 1)");
        assert_eq!(buffer_items(&def_buf).len(), 1, "a bare def is kept");
        // A bare expression (nothing to call) yields no items.
        let expr_buf = parse("(+ 1 2)");
        assert!(
            buffer_items(&expr_buf).is_empty(),
            "a bare expression has no callable items"
        );
    }

    #[test]
    fn defined_names_lists_functions_and_value_bindings() {
        let buf = parse("(do (def (f x) x) (def y 3) (type T (A unit)) (export f))");
        // f (function) + y (value); the `type` and `export` are not defs.
        assert_eq!(defined_names(&buf), vec!["f".to_string(), "y".to_string()]);
    }

    #[test]
    fn assemble_builds_a_do_block_with_the_entry_and_export() {
        let buf = parse("(do (def (dbl x) (* x 2)) (export dbl))");
        let expr = parse("(dbl 21)");
        let program = assemble_repl_program(&buf, &expr);
        // Re-render to s-expr and check the shape: the def is kept, the synthesized entry calls dbl,
        // and the sole export is the entry.
        let rendered = crate::query::Tree::of(&program).to_sexpr();
        assert!(
            rendered.contains("(def (dbl x)"),
            "buffer def kept: {rendered}"
        );
        assert!(
            rendered.contains(&format!("(def ({REPL_ENTRY}) (dbl 21))")),
            "entry wraps the expression: {rendered}"
        );
        assert!(
            rendered.contains(&format!("(export {REPL_ENTRY})")),
            "the entry is exported: {rendered}"
        );
        // The buffer's OWN export (dbl) is dropped — only the entry is exported.
        assert!(
            !rendered.contains("(export dbl)"),
            "the buffer's own export is dropped: {rendered}"
        );
    }

    #[test]
    fn assemble_tolerates_a_bare_expression_buffer() {
        // A buffer that is itself a bare expression has nothing to call; the entry stands alone.
        let buf = parse("(+ 1 1)");
        let expr = parse("(* 6 7)");
        let program = assemble_repl_program(&buf, &expr);
        let rendered = crate::query::Tree::of(&program).to_sexpr();
        assert!(
            rendered.contains(&format!("(def ({REPL_ENTRY}) (* 6 7))")),
            "entry present with no buffer items: {rendered}"
        );
    }
}
