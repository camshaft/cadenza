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

/// The do-local module the EXACT-MODE entry body is wrapped in — its `(pragma default-fraction Rational)`
/// makes a bare numeric literal in the wrapped expression an exact rational (`numeric-model.md` §A Module
/// May Declare Its Default Fraction Literal Type). Kebab (a member name, not a boundary name, but kept
/// kebab for consistency) and `cdz-`-prefixed so it can't collide with a reader's own module.
const EXACT_MODULE: &str = "cdz-calc-exact";
const EXACT_INNER: &str = "cdz-calc-value";

/// Assemble the runnable REPL program: the buffer's kept top-level items (defs/types, shell unwrapped
/// and exports dropped) plus a synthesized nullary entry `(def (cdz-repl-eval) <expr>)`, exported as
/// the sole entry, all in one fresh arena as a top-level `(do item… entry export)` block.
///
/// The default (`assemble_repl_program`) leaves the expression's numeric literals with their ordinary
/// types (integer `/` truncates). [`assemble_repl_program_exact`] instead wraps the expression in a
/// do-local `(module … (pragma default-fraction Rational) (def (cdz-calc-value) <expr>)) ((. … ) unit)`
/// so a bare literal grounds to an EXACT rational — the calculator's "exact by default" mode, so `1 / 3`
/// is `1/3` without the `R` suffix (C6). Everything else is identical.
///
/// `buffer` is the parsed definitions arena; `expr` is the parsed expression arena (its whole root is
/// the entry body). Both are copied AT THE AST LEVEL into the result — NOT string-spliced — so a string
/// literal containing parentheses (or any surface quirk) can't corrupt the assembly. The returned arena
/// is ready to `codec::encode` + hand to the compiler.
pub fn assemble_repl_program(buffer: &Arenas, expr: &Arenas) -> Arenas {
    assemble_with(buffer, expr, false)
}

/// [`assemble_repl_program`] in EXACT mode — the expression's bare numeric literals default to `Rational`
/// (exact by default), via a do-local `(pragma default-fraction Rational)` module wrapping the entry body.
pub fn assemble_repl_program_exact(buffer: &Arenas, expr: &Arenas) -> Arenas {
    assemble_with(buffer, expr, true)
}

fn assemble_with(buffer: &Arenas, expr: &Arenas, exact: bool) -> Arenas {
    let mut b = Builder::new();
    let do_head = b.name("do");
    let mut do_kids = vec![do_head];
    for it in buffer_items(buffer) {
        do_kids.push(copy_subtree(&mut b, buffer, it));
    }
    // The synthesized entry: `(def (cdz-repl-eval) <body>)`, where <body> is either the expression
    // directly, or — in exact mode — the expression wrapped so its literals default to Rational.
    let entry_def_head = b.name("def");
    let entry_name = b.name(REPL_ENTRY);
    let entry_sig = b.list(vec![entry_name]); // `(cdz-repl-eval)` — a nullary signature
    let expr_core = copy_subtree(&mut b, expr, expr.root);
    let entry_body = if exact {
        // `(do (module cdz-calc-exact (pragma default-fraction Rational) (def (cdz-calc-value) <expr>))
        //      ((. cdz-calc-exact cdz-calc-value) unit))` — a do-local module whose pragma grounds <expr>'s
        // bare literals to exact rationals, then called. (A do-local module IS visible to a later form in
        // the same do; a top-level module sibling is not — so the pragma module lives inside the entry.)
        let module_head = b.name("module");
        let module_name = b.name(EXACT_MODULE);
        let pragma_head = b.name("pragma");
        let pragma_key = b.name("default-fraction");
        let pragma_ty = b.name("Rational");
        let pragma = b.list(vec![pragma_head, pragma_key, pragma_ty]);
        let inner_def_head = b.name("def");
        let inner_name = b.name(EXACT_INNER);
        let inner_sig = b.list(vec![inner_name]); // `(cdz-calc-value)` — nullary
        let inner_def = b.list(vec![inner_def_head, inner_sig, expr_core]);
        let module = b.list(vec![module_head, module_name, pragma, inner_def]);
        // `((. cdz-calc-exact cdz-calc-value) unit)` — member-access the inner value + apply to `unit`.
        let dot = b.name(".");
        let acc_mod = b.name(EXACT_MODULE);
        let acc_val = b.name(EXACT_INNER);
        let member = b.list(vec![dot, acc_mod, acc_val]);
        let unit = b.name("unit");
        let call = b.list(vec![member, unit]);
        // The body is a do declaring the module then calling it.
        let body_do = b.name("do");
        b.list(vec![body_do, module, call])
    } else {
        expr_core
    };
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
    fn buffer_items_drops_exports_from_a_module_shell_too() {
        // The export-dropping filter runs on a `(module …)` buffer (the hand-written-program path), not
        // just a `(do …)` wrap — otherwise the buffer's own `(export …)` would leak into the assembled
        // program alongside the REPL's synthesized sole export. Multiple items + an export interleaved.
        let buf = parse("(module M (def (f x) x) (export f) (def g 5))");
        let items = buffer_items(&buf);
        assert_eq!(items.len(), 2, "two defs kept, the export dropped");
        assert_eq!(def_name(&buf, items[0]).as_deref(), Some("f"));
        assert_eq!(def_name(&buf, items[1]).as_deref(), Some("g"));
    }

    #[test]
    fn buffer_items_of_an_empty_module_is_no_items() {
        // A module with only a name and no body items unwraps to nothing (skip past head + name leaves an
        // empty tail) — no panic on the `skip(2)` of a 2-element list.
        let buf = parse("(module M)");
        assert!(buffer_items(&buf).is_empty());
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
    fn def_name_reads_each_def_shape_and_rejects_non_defs() {
        // `def_name` extracts the bound name from a top-level item. Exercise its branches directly (each
        // item is a whole arena whose root IS the item): a function-signature def, a bare value def, and
        // the guard paths that must return None.
        let name_of = |s: &str| {
            let a = parse(s);
            def_name(&a, a.root)
        };
        // `(def (f x y) body)` — a function signature; the sig head is the name.
        assert_eq!(name_of("(def (f x y) x)"), Some("f".to_string()));
        // A nullary signature `(def (main) body)` still names `main`.
        assert_eq!(name_of("(def (main) 0)"), Some("main".to_string()));
        // `(def y body)` — a bare value binding; the atom is the name.
        assert_eq!(name_of("(def y 3)"), Some("y".to_string()));
        // NOT a def — a different head returns None.
        assert_eq!(name_of("(type T (A unit))"), None);
        assert_eq!(name_of("(export f)"), None);
        // An empty signature list `(def () body)` has no name head → None (a guard branch, not a panic).
        assert_eq!(name_of("(def () 0)"), None);
        // A bare atom (not a list) is not a def → None.
        assert_eq!(name_of("42"), None);
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
    fn assemble_exact_wraps_the_expr_in_a_default_fraction_module() {
        let buf = parse("0"); // no items (a bare expression buffer)
        let expr = parse("(/ 1 3)");
        let program = assemble_repl_program_exact(&buf, &expr);
        let rendered = crate::query::Tree::of(&program).to_sexpr();
        // The entry body wraps the expr in a do-local `(module … (pragma default-fraction Rational) …)`.
        assert!(
            rendered.contains("(pragma default-fraction Rational)"),
            "exact mode declares the default-fraction pragma: {rendered}"
        );
        assert!(
            rendered.contains("(/ 1 3)"),
            "the expression is inside the module's inner def: {rendered}"
        );
        assert!(
            rendered.contains(&format!("(def ({REPL_ENTRY})")),
            "still exported as the sole entry: {rendered}"
        );
        // The NON-exact assembler does NOT wrap — the expr is the bare entry body.
        let plain = crate::query::Tree::of(&assemble_repl_program(&buf, &expr)).to_sexpr();
        assert!(
            !plain.contains("default-fraction"),
            "non-exact mode adds no pragma: {plain}"
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

    #[test]
    fn assemble_over_a_module_buffer_keeps_defs_drops_the_buffers_export() {
        // The "surfaces never drift" invariant reaches the MODULE-shelled buffer (the hand-written
        // program path), not just the guide's `(do …)` wrap: the module's defs are kept, its own
        // `(export …)` is dropped, and only the synthesized entry is exported — same shape as a `do`
        // buffer assembles to.
        let buf = parse("(module M (def (dbl x) (* x 2)) (export dbl))");
        let expr = parse("(dbl 21)");
        let rendered = crate::query::Tree::of(&assemble_repl_program(&buf, &expr)).to_sexpr();
        assert!(
            rendered.contains("(def (dbl x)"),
            "the module's def is kept: {rendered}"
        );
        assert!(
            rendered.contains(&format!("(def ({REPL_ENTRY}) (dbl 21))")),
            "entry wraps the expression: {rendered}"
        );
        assert!(
            rendered.contains(&format!("(export {REPL_ENTRY})")),
            "the synthesized entry is exported: {rendered}"
        );
        assert!(
            !rendered.contains("(export dbl)"),
            "the buffer module's own export is dropped: {rendered}"
        );
        // The `module` shell itself is unwrapped — the assembled program is a flat `(do …)`, not a
        // nested module.
        assert!(
            !rendered.contains("(module M"),
            "the buffer's module shell is unwrapped, not re-nested: {rendered}"
        );
    }
}
