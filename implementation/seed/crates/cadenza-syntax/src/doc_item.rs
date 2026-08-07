//! The doc-item projection — a program's public surface projected into a DERIVED doc-AST.
//!
//! `cadenza doc` (design/DESIGN-cadenza-docs.md, increment I1): fold a program's canonical [`Arenas`]
//! into a `doc-module` construct — a rustdoc-like structured index that is ITSELF a `cdzast` value, so
//! every existing AST surface (reader/printer, sexpr/ML, the codec, the metaprog `Ast`) applies to it
//! uniformly. This is the STRUCTURAL half (increment I1, operator decision 9.1 = purely-syntactic):
//! it needs no types — it reads the `(doc …)` / `(module-doc …)` nodes the parser already splices
//! (`parser::take_docs_here`), the item's head `(name …)`, and a printed `(sig …)` via the ML
//! printer. The RESOLVED `(ty …)` is increment I2's job (post-typecheck, in `rcdzc`) — this pass
//! deliberately does NOT depend on inference, so it runs off the parser alone (a syntax-only doc
//! outline / IDE symbol list needs no typecheck).
//!
//! The projected shape (all NEW HEAD NAMES — never a new [`Struct`]/[`Leaf`] variant, honoring the
//! frozen 2-variant `Struct` + keywords-are-data):
//! ```text
//! (doc-module "<module-name>"
//!   (module-doc "…top-of-module prose…")?          ; 0-or-1, from leading (module-doc …) siblings
//!   (doc-item
//!     (name "map")
//!     (sig  "map(f, xs) = …")                       ; printed display signature (syntactic)
//!     (doc  "Applies f to each element."))?          ; 0-or-1 (doc …), concatenated item prose
//!   (doc-item …)…)
//! ```
//! A `doc-module` is `cdzast`: it canonicalizes, encodes to `\x00\x01`, and round-trips through
//! sexpr/ML — the same bijection every other arena has (see the round-trip tests).
//!
//! PUBLIC surface only: an item appears iff its name is `export`ed by the program (design §8 — v1
//! extracts the public surface; a later pass can carry `(visibility …)` to include internals).

use crate::ast::{Arenas, Builder, Leaf, StructId};
use std::collections::BTreeSet;

/// Project a program's canonical `Arenas` into a `doc-module` doc-AST, named `module_name`.
///
/// Walks the program's top-level forms: each exported `def`/`type`/`effect` becomes a `doc-item`
/// (name + printed syntactic signature + concatenated `(doc …)` prose); leading `(module-doc …)`
/// siblings become the module's `(module-doc …)`. The result is a fresh, canonical-ready `Arenas`
/// rooted at the `doc-module` node — ready to `encode`, `canonicalize`, or print.
pub fn project(program: &Arenas, module_name: &str) -> Arenas {
    let forms = top_level_forms(program);
    let exported = exported_names(program, &forms);

    let mut b = Builder::new();
    let mut children = vec![
        b.name("doc-module"),
        b.atom_leaf(Leaf::Str(module_name.to_string())),
    ];

    // Module-doc: concatenate every top-level `(module-doc "…")` sibling's text (in source order).
    let module_doc = collect_doc_text(program, forms.iter().copied(), "module-doc");
    if let Some(text) = module_doc {
        let head = b.name("module-doc");
        let t = b.atom_leaf(Leaf::Str(text));
        let node = b.list(vec![head, t]);
        children.push(node);
    }

    // One `doc-item` per exported def/type/effect, in source order.
    for &form in &forms {
        let Some((kind, name)) = item_kind_and_name(program, form) else {
            continue;
        };
        if !exported.contains(name) {
            continue;
        }
        let item = build_doc_item(&mut b, program, form, kind, name);
        children.push(item);
    }

    let root = b.list(children);
    b.finish(root)
}

/// The head names this pass treats as a documentable item, paired with how to read the item's name.
/// `def`'s name is either a bare `(name …)` (value def) or the head of a `(name p…)` sig list
/// (function def); `type`/`effect` name it directly as their first argument.
fn item_kind_and_name(program: &Arenas, form: StructId) -> Option<(&'static str, &str)> {
    for (head, kind) in [("def", "def"), ("type", "type"), ("effect", "effect")] {
        if let Some(args) = program.as_form(form, head) {
            let name = item_name(program, kind, args)?;
            return Some((kind, name));
        }
    }
    None
}

/// Read an item's public name from its form arguments.
/// - `def`: arg 0 is either an `Atom(Name)` (value def `(def name … value)`) or a `List` whose head is
///   the name (function def `(def (name p…) … body)`).
/// - `type` / `effect`: arg 0 is the `Atom(Name)` naming the declaration.
fn item_name<'a>(program: &'a Arenas, kind: &str, args: &[StructId]) -> Option<&'a str> {
    let first = *args.first()?;
    match kind {
        "def" => program.as_name(first).or_else(|| program.head_name(first)),
        _ => program.as_name(first),
    }
}

/// Build one `(doc-item (name "…") (sig "…") (doc "…")?)` node into `b`. `sig` is the item's WHOLE
/// form printed via the ML printer (the syntactic signature as written — a def's params + a value
/// body's shape, a type/effect's decl); `doc` is the concatenated leading `(doc …)` prose, omitted
/// when the item carries none.
fn build_doc_item(
    b: &mut Builder,
    program: &Arenas,
    form: StructId,
    _kind: &str,
    name: &str,
) -> StructId {
    let mut item = vec![b.name("doc-item")];

    // (name "…")
    let name_head = b.name("name");
    let name_val = b.atom_leaf(Leaf::Str(name.to_string()));
    let name_node = b.list(vec![name_head, name_val]);
    item.push(name_node);

    // (sig "…") — the item form printed to a display string (syntactic signature).
    let sig_head = b.name("sig");
    let sig_val = b.atom_leaf(Leaf::Str(print_subtree(program, form)));
    let sig_node = b.list(vec![sig_head, sig_val]);
    item.push(sig_node);

    // (doc "…") — the item's own leading `(doc …)` children, concatenated; omitted if none.
    let item_children = match program.get(form) {
        crate::ast::Struct::List(kids) => kids.as_slice(),
        crate::ast::Struct::Atom(_) => &[],
    };
    if let Some(text) = collect_doc_text(program, item_children.iter().copied(), "doc") {
        let doc_head = b.name("doc");
        let doc_val = b.atom_leaf(Leaf::Str(text));
        let doc_node = b.list(vec![doc_head, doc_val]);
        item.push(doc_node);
    }

    b.list(item)
}

/// Concatenate the text of every `(<head> "text")` node among `nodes` (in order), joined by newlines.
/// Returns `None` if there are none (so the caller can omit an empty `(doc …)`/`(module-doc …)`).
fn collect_doc_text(
    program: &Arenas,
    nodes: impl Iterator<Item = StructId>,
    head: &str,
) -> Option<String> {
    let mut texts: Vec<&str> = Vec::new();
    for id in nodes {
        if let Some(args) = program.as_form(id, head)
            && let Some(&text_id) = args.first()
            && let Some(s) = program.as_str(text_id)
        {
            texts.push(s);
        }
    }
    if texts.is_empty() {
        None
    } else {
        Some(texts.join("\n"))
    }
}

/// The top-level forms of a program: the children of a top-level `(do …)`, or the single root form
/// when the program is one form (not wrapped in a `do`).
fn top_level_forms(program: &Arenas) -> Vec<StructId> {
    if let Some(items) = program.as_form(program.root, "do") {
        items.to_vec()
    } else {
        vec![program.root]
    }
}

/// The set of names the program `export`s — the public surface. An `(export a b …)` form lists the
/// exported names as `Atom(Name)` children; a program may carry more than one export form.
fn exported_names<'a>(program: &'a Arenas, forms: &[StructId]) -> BTreeSet<&'a str> {
    let mut out = BTreeSet::new();
    for &form in forms {
        if let Some(args) = program.as_form(form, "export") {
            for &arg in args {
                if let Some(n) = program.as_name(arg) {
                    out.insert(n);
                }
            }
        }
    }
    out
}

/// Print the subtree rooted at `id` to a display string via the ML printer. The printer prints a whole
/// `Arenas`, so extract `id`'s subtree into a fresh dense arena first, then print that.
fn print_subtree(program: &Arenas, id: StructId) -> String {
    let sub = extract_subtree(program, id);
    crate::printer::print_display(&sub, crate::printer::DEFAULT_WIDTH)
}

/// Extract the subtree rooted at `id` of `program` into its own standalone, dense `Arenas` (a fresh
/// root). Iterative post-order (an explicit stack) so a deep item can't overflow the native stack.
fn extract_subtree(program: &Arenas, id: StructId) -> Arenas {
    let mut b = Builder::new();
    enum Job {
        Visit(StructId),
        EmitList(usize),
    }
    let mut jobs = vec![Job::Visit(id)];
    let mut results: Vec<StructId> = Vec::new();
    while let Some(job) = jobs.pop() {
        match job {
            Job::Visit(sid) => match program.get(sid) {
                crate::ast::Struct::Atom(lid) => {
                    let leaf = program.leaf(*lid).clone();
                    let node = b.atom_leaf(leaf);
                    results.push(node);
                }
                crate::ast::Struct::List(kids) => {
                    jobs.push(Job::EmitList(kids.len()));
                    for &k in kids.iter().rev() {
                        jobs.push(Job::Visit(k));
                    }
                }
            },
            Job::EmitList(n) => {
                let kids = results.split_off(results.len() - n);
                let node = b.list(kids);
                results.push(node);
            }
        }
    }
    let root = results.pop().expect("extract_subtree leaves a root");
    b.finish(root)
}
