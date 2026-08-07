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
//!     (sig  (map f xs))                             ; STRUCTURED signature sub-AST (decl head, no body)
//!     (doc  "Applies f to each element."))?          ; 0-or-1 (doc …), concatenated item prose
//!   (doc-item …)…)
//! ```
//! A `doc-module` is `cdzast`: it canonicalizes, encodes to `\x00\x01`, and round-trips through
//! sexpr/ML — the same bijection every other arena has (see the round-trip tests).
//!
//! PUBLIC surface only: an item appears iff its name is `export`ed by the program (design §8 — v1
//! extracts the public surface; a later pass can carry `(visibility …)` to include internals).

use crate::ast::{Arenas, Builder, Leaf, StructId};
use crate::query::{self, Pattern, Tree};
use std::collections::BTreeSet;

/// Project a program's canonical `Arenas` into a `doc-module` doc-AST, named `module_name`.
///
/// Doc-extraction is a COMPILER QUERY (operator: "just like any other query the compiler supports"):
/// the item selection runs on the [`crate::query`] machinery — each documentable item is found by a
/// structural `query::search` over the program, so this composes with the other compiler queries (and
/// an IDE outline / syntax-only preview reuses the same selection). For each EXPORTED, TOP-LEVEL
/// `def`/`type`/`effect` it emits a `doc-item` (name + printed syntactic signature + concatenated
/// `(doc …)` prose); leading `(module-doc …)` siblings become the module's `(module-doc …)`. The result
/// is a fresh, canonical-ready `Arenas` rooted at the `doc-module` node — ready to `encode`,
/// `canonicalize`, or print.
pub fn project(program: &Arenas, module_name: &str) -> Arenas {
    let forms = top_level_forms(program);
    let top_level: BTreeSet<u32> = forms.iter().map(|s| s.0).collect();
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

    // Select the documentable items via the QUERY machinery, in source (StructId) order, keeping only
    // TOP-LEVEL exported items (an item found nested inside another form is not a module's public API).
    let mut items: Vec<DocItemSel> = select_items(program)
        .into_iter()
        .filter(|sel| top_level.contains(&sel.origin.0) && exported.contains(sel.name.as_str()))
        .collect();
    items.sort_by_key(|sel| sel.origin.0);

    for sel in items {
        let item = build_doc_item(&mut b, program, &sel);
        children.push(item);
    }

    let root = b.list(children);
    b.finish(root)
}

/// The documentable items in `program`, as `(origin arena node, public name)` pairs — found by
/// COMPILER QUERY. A `def`/`type`/`effect` is matched by a structural [`Pattern`] over the program
/// tree; the match's `origin()` is the item's node in the original arena (so the sig/docs project from
/// it), and the bound `,name` metavar is the item's identifier.
///
/// The `def` case needs TWO patterns because a value def `(def name val)` and a function def
/// `(def (name params…) body)` bind the name differently: the FUNCTION pattern `(def (,name …) …)`
/// binds `,name` to the identifier, while the VALUE pattern `(def ,name …)` binds it to the identifier
/// only when the first arg is atomic (for a function def it would bind the whole `(name params)` sig
/// list — wrong). So we try the function pattern first and fall back to the value pattern for the
/// nodes it didn't match.
fn select_items(program: &Arenas) -> Vec<DocItemSel> {
    let tree = Tree::of(program);
    let mut out: Vec<DocItemSel> = Vec::new();
    let mut seen: BTreeSet<u32> = BTreeSet::new();

    // (pattern, kind) — function-def FIRST so a function def binds the identifier, not its sig list;
    // then value-def, type, effect. `def` (function + value) both report kind "def".
    let patterns = [
        ("(def (,name ,@params) ,@body)", "def"),
        ("(def ,name ,@rest)", "def"),
        ("(type ,name ,@rest)", "type"),
        ("(effect ,name ,@rest)", "effect"),
    ];
    for (src, kind) in patterns {
        let pat = match Pattern::compile(src) {
            Ok(p) => p,
            Err(_) => continue, // a malformed pattern is a bug, but never panic the projection
        };
        for m in query::search(&pat, &tree, None) {
            let Some(origin) = m.node.origin() else {
                continue;
            };
            if seen.contains(&origin.0) {
                continue; // already claimed by an earlier (more specific) pattern
            }
            // The bound `,name` must be a bare identifier (an `Atom(Name)`). For the value-def pattern
            // this rejects a function def (whose `,name` is the `(name params)` sig LIST), leaving it to
            // the function pattern that already claimed it.
            let Some(name) = m.bindings.get("name").and_then(bound_name) else {
                continue;
            };
            seen.insert(origin.0);
            out.push(DocItemSel { origin, name, kind });
        }
    }
    out
}

/// A selected documentable item: its node in the program arena, its public name, and its kind
/// (`def`/`type`/`effect`) — the head-name the selecting query pattern matched.
struct DocItemSel {
    origin: StructId,
    name: String,
    kind: &'static str,
}

/// If a matched `,name` binding is a bare `Name` atom, its text — the identifier. A non-atom binding
/// (e.g. a function def's `(name params)` sig list captured by the value-def pattern) yields `None`.
fn bound_name(t: &Tree) -> Option<String> {
    match t {
        Tree::Atom(Leaf::Name(n), _) => Some(n.clone()),
        _ => None,
    }
}

/// Build one `(doc-item (name "…") (sig <sub-ast>) (doc "…")? (kind …) (visibility …))` node into `b`.
///
/// `sig` carries the item's STRUCTURED SIGNATURE — a real `cdzast` sub-AST, NOT a printed string
/// (operator ruling PR #2559 r3735402339 / revised design §2.1: the compiler EMITS structured
/// render-sufficient info; RENDERING is the consumer's job — it runs the ML printer over the `(sig …)`
/// subtree, a pure function of the node, so no drift). The signature is the item's DECLARATION shape
/// WITHOUT its body: a function def's `(name params…)` head (with any inline `(: p T)` annotations), a
/// value-def / type / effect's declaration form. (I2 enriches with the resolved arrow type in a
/// sibling `(ty …)`, post-typecheck.) `doc` is the concatenated leading `(doc …)` prose, omitted when
/// the item carries none. `kind` is `def`/`type`/`effect`; `visibility` is `public` (v1 projects only
/// the exported surface — a later pass can carry `internal` when it includes non-exported items).
fn build_doc_item(b: &mut Builder, program: &Arenas, sel: &DocItemSel) -> StructId {
    let form = sel.origin;
    let mut item = vec![b.name("doc-item")];

    // (name "…")
    let name_head = b.name("name");
    let name_val = b.atom_leaf(Leaf::Str(sel.name.clone()));
    let name_node = b.list(vec![name_head, name_val]);
    item.push(name_node);

    // (sig <sub-ast>) — the STRUCTURED signature subtree (declaration shape, NOT the body). For a
    // function def `(def (f p…) body)` that is the head `(f p…)`; for a value-def / type / effect it is
    // the declaration head (name + decl args, sans the initializer/body). Grafted into the doc-AST.
    let sig_head = b.name("sig");
    let sig_src = signature_node(program, form);
    let sig_sub = graft_subtree(b, program, sig_src);
    let sig_node = b.list(vec![sig_head, sig_sub]);
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

    // (kind def|type|effect) — the item's kind (the query head-name that selected it).
    let kind_head = b.name("kind");
    let kind_val = b.name(sel.kind);
    let kind_node = b.list(vec![kind_head, kind_val]);
    item.push(kind_node);

    // (visibility public) — v1 projects only the EXPORTED surface, so every item is public. (A later
    // internal-docs pass carries `internal` for non-exported items; the field is present now for that.)
    let vis_head = b.name("visibility");
    let vis_val = b.name("public");
    let vis_node = b.list(vec![vis_head, vis_val]);
    item.push(vis_node);

    b.list(item)
}

/// The node whose subtree is the item's STRUCTURED SIGNATURE — the declaration shape without the body.
/// For a function def `(def (f p…) doc… body)` that is the head list `(f p…)` (name + params, inline
/// annotations kept). For a value def `(def x doc… val)`, a `type`, or an `effect`, the declaration
/// head is the NAME atom itself (there is no param list) — the signature is just the name, and I2's
/// `(ty …)` carries the resolved type. Falls back to the whole `form` if the shape is unexpected (never
/// panics — a malformed item still yields a sig node).
fn signature_node(program: &Arenas, form: StructId) -> StructId {
    // A `def` whose first arg is a `List` is a function def: its head `(name params…)` IS the signature.
    if let Some(args) = program.as_form(form, "def")
        && let Some(&head) = args.first()
        && matches!(program.get(head), crate::ast::Struct::List(_))
    {
        return head;
    }
    // Value def / type / effect: the signature is the declaration's NAME (first arg). If it isn't there
    // for some reason, fall back to the whole form (a robust, if broad, sig — never a panic).
    for kw in ["def", "type", "effect"] {
        if let Some(args) = program.as_form(form, kw)
            && let Some(&first) = args.first()
        {
            return first;
        }
    }
    form
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

/// Copy the subtree rooted at `id` of `program` INTO the in-progress builder `b`, returning its new
/// root `StructId` — so the item's structured signature is grafted directly into the doc-AST being
/// built (the `(sig <sub-ast>)` payload). Iterative post-order (an explicit stack) so a deep item can't
/// overflow the native stack; leaves interned by value like any Builder push.
fn graft_subtree(b: &mut Builder, program: &Arenas, id: StructId) -> StructId {
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
    results.pop().expect("graft_subtree leaves a root")
}
