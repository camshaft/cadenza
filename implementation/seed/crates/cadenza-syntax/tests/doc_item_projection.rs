//! Integration tests for the doc-item projection (`doc_item::project`, design/DESIGN-cadenza-docs.md
//! I1). These build real programs via the ML parser (so `///` doc-comments become the `(doc …)` /
//! `(module-doc …)` nodes the projection reads) and assert (a) the projected `doc-module` structure and
//! (b) that a projected doc-AST is ordinary `cdzast` — it encodes → `\x00\x01` → decodes identically
//! (the frozen bijection) and re-reads through the s-expr surface. They exercise the PUBLIC API from
//! outside the crate, so they live here rather than in-crate.

use cadenza_syntax::ast::{Arenas, StructId};
use cadenza_syntax::{codec, doc_item, parser, sexpr};

/// Parse ML `src` into its canonical arena (asserting no parse errors), the input to the projection.
fn program(src: &str) -> Arenas {
    let parsed = parser::read_ml(src);
    assert!(
        parsed.errors.is_empty(),
        "parse errors in test program: {:?}",
        parsed.errors
    );
    parsed.arenas
}

/// The string payload of the `(<field> "…")` CHILD of a `doc-item` (the field is a child form, not the
/// item's own head — so search the item's children for a form headed `field`).
fn item_field<'a>(doc: &'a Arenas, item: StructId, field: &str) -> Option<&'a str> {
    let children = match doc.get(item) {
        cadenza_syntax::ast::Struct::List(kids) => kids.as_slice(),
        cadenza_syntax::ast::Struct::Atom(_) => return None,
    };
    for &c in children {
        if let Some(args) = doc.as_form(c, field) {
            return doc.as_str(*args.first()?);
        }
    }
    None
}

/// The `(name "…")` string of a `doc-item`, for assertions.
fn item_name(doc: &Arenas, item: StructId) -> Option<&str> {
    item_field(doc, item, "name")
}

/// The `(doc "…")` string of a `doc-item`, or None if it carries none.
fn item_doc(doc: &Arenas, item: StructId) -> Option<&str> {
    item_field(doc, item, "doc")
}

/// The `(sig <sub-ast>)` STRUCTURED signature node of a `doc-item` — the item form's subtree (a
/// `List`), NOT a string. (Operator ruling: the compiler emits structured info; a consumer renders it.)
fn item_sig_node(doc: &Arenas, item: StructId) -> Option<StructId> {
    let children = match doc.get(item) {
        cadenza_syntax::ast::Struct::List(kids) => kids.as_slice(),
        cadenza_syntax::ast::Struct::Atom(_) => return None,
    };
    for &c in children {
        if let Some(args) = doc.as_form(c, "sig") {
            return args.first().copied();
        }
    }
    None
}

/// All `doc-item` children of a `doc-module` root.
fn doc_items(doc: &Arenas) -> Vec<StructId> {
    let Some(children) = doc.as_form(doc.root, "doc-module") else {
        panic!("root is not a doc-module");
    };
    children
        .iter()
        .copied()
        .filter(|&c| doc.as_form(c, "doc-item").is_some())
        .collect()
}

#[test]
fn projects_an_exported_def_with_its_doc_name_and_sig() {
    // A documented, exported function def → one doc-item carrying its name, prose, and printed sig.
    let src = "\
/// Applies f to each element.
def map(f, xs) = f
export { map }";
    let doc = doc_item::project(&program(src), "mymod");

    // root is (doc-module "mymod" …)
    let mod_args = doc
        .as_form(doc.root, "doc-module")
        .expect("root is a doc-module");
    assert_eq!(doc.as_str(mod_args[0]), Some("mymod"), "module name");

    let items = doc_items(&doc);
    assert_eq!(items.len(), 1, "exactly one exported item");
    assert_eq!(item_name(&doc, items[0]), Some("map"));
    assert_eq!(
        item_doc(&doc, items[0]),
        Some("Applies f to each element."),
        "the item's /// prose"
    );
    // (sig …) is a STRUCTURED sub-AST (a List, not a string) — the SIGNATURE (declaration head, no
    // body). For `def map(f, xs) = f` the signature is the head `(map f xs)` (name + params), NOT the
    // whole `(def …)` form and NOT the body. A consumer RENDERS it via the ML printer.
    let sig = item_sig_node(&doc, items[0]).expect("a structured sig node");
    assert!(
        matches!(doc.get(sig), cadenza_syntax::ast::Struct::List(_)),
        "sig is a structured sub-AST (List), not a printed string"
    );
    // The signature head is the function NAME, followed by its params — it is `(map f xs)`.
    assert_eq!(
        doc.head_name(sig),
        Some("map"),
        "sig subtree is the signature head `(map f xs)` (name + params), not the whole def form"
    );
    let rendered = cadenza_syntax::sexpr::print_from(&doc, sig);
    assert!(
        rendered.contains("map") && rendered.contains('f') && rendered.contains("xs"),
        "rendering the structured sig yields the signature (name + params), got {rendered:?}"
    );
    // The body (`f` as the def value) is NOT part of the signature — sig is the head only.
    assert!(
        !rendered.starts_with("def"),
        "sig is the signature head, not the `def` declaration form, got {rendered:?}"
    );
}

#[test]
fn projects_only_the_exported_public_surface() {
    // `helper` is defined but NOT exported → it must NOT appear; only the exported `pub_fn` does.
    let src = "\
/// internal.
def helper(x) = x
/// public.
def pub_fn(y) = y
export { pub_fn }";
    let doc = doc_item::project(&program(src), "m");
    let items = doc_items(&doc);
    let names: Vec<_> = items.iter().filter_map(|&i| item_name(&doc, i)).collect();
    assert_eq!(
        names,
        vec!["pub_fn"],
        "only the exported name is a doc-item"
    );
}

#[test]
fn a_module_doc_becomes_the_doc_module_module_doc() {
    // A `///` on a NON-def form (here an `import`) is preserved by the parser as a top-level
    // `(module-doc …)` sibling (parser.rs stmt), which the projection lifts to the doc-module's
    // `(module-doc …)`. (A `///` directly before a def instead documents that def — see the item-doc
    // tests — so the module-doc case is a header doc on a non-documentable form.)
    let src = "\
/// Top-of-module prose.
import { x } from \"other\"
def f(x) = x
export { f }";
    let doc = doc_item::project(&program(src), "m");
    let mod_args = doc.as_form(doc.root, "doc-module").unwrap();
    // Some child is (module-doc "Top-of-module prose.")
    let md = mod_args
        .iter()
        .find_map(|&c| doc.as_form(c, "module-doc").map(|a| doc.as_str(a[0])));
    assert_eq!(
        md,
        Some(Some("Top-of-module prose.")),
        "the leading /// is the module-doc"
    );
}

#[test]
fn the_sig_is_a_structured_sub_ast_not_a_printed_string() {
    // Operator ruling (PR #2559 r3735402339): the compiler EMITS the structured signature; RENDERING is
    // the consumer's job. So `(sig …)` must be a SUB-AST (a List), NOT a `Leaf::Str`. And per revised
    // design §2.1 it is the SIGNATURE (declaration head, no body) — for `def add(a, b) = a` that's the
    // head `(add a b)`, not the whole `(def …)` form. Pin the shape directly + that a consumer can
    // recover the display string via the ML printer (structured-is-truth).
    let src = "\
/// doc.
def add(a, b) = a
export { add }";
    let doc = doc_item::project(&program(src), "m");
    let item = doc_items(&doc)[0];
    let sig = item_sig_node(&doc, item).expect("a sig node");

    // NOT a string leaf — a structured List.
    match doc.get(sig) {
        cadenza_syntax::ast::Struct::List(_) => {}
        cadenza_syntax::ast::Struct::Atom(_) => {
            panic!("(sig …) must be a structured sub-AST (List), not a printed-string Atom")
        }
    }
    // The signature head is the function name `add` (followed by its params) — NOT `def` (the whole
    // declaration) and NOT the body.
    assert_eq!(
        doc.head_name(sig),
        Some("add"),
        "the sig subtree is the signature head `(add a b)`, not the `def` form"
    );
    // Structured-is-truth, printed-is-derived: a consumer renders the SAME subtree via the ML printer.
    let display = cadenza_syntax::sexpr::print_from(&doc, sig);
    assert!(
        display.contains("add"),
        "rendering the structured sig recovers the signature, got {display:?}"
    );
}

#[test]
fn a_doc_item_carries_kind_and_visibility() {
    // Revised design §2.1: each doc-item carries (kind def|type|effect) + (visibility public). v1
    // projects only the exported surface, so visibility is always `public`; kind is the item's form.
    let src = "\
def f(x) = x
type T = | A
effect E = | op : Str
export { f, T, E }";
    let doc = doc_item::project(&program(src), "m");
    let items = doc_items(&doc);
    // helper: the name-head value of a doc-item's `(field <name-atom>)` child (kind/visibility are
    // name atoms, not strings).
    let field_name = |item: StructId, field: &str| -> Option<String> {
        let kids = match doc.get(item) {
            cadenza_syntax::ast::Struct::List(k) => k.clone(),
            cadenza_syntax::ast::Struct::Atom(_) => return None,
        };
        for c in kids {
            if let Some(args) = doc.as_form(c, field) {
                return doc.as_name(*args.first()?).map(str::to_string);
            }
        }
        None
    };
    for item in &items {
        assert_eq!(
            field_name(*item, "visibility").as_deref(),
            Some("public"),
            "every projected item is public in v1"
        );
    }
    // kinds line up with the three item forms (in source order f/T/E).
    let kinds: Vec<_> = items
        .iter()
        .filter_map(|&i| field_name(i, "kind"))
        .collect();
    assert_eq!(kinds, vec!["def", "type", "effect"], "kind per item form");
}

#[test]
fn an_exported_def_without_docs_has_a_sig_and_name_but_no_doc() {
    // No `///` → the doc-item still carries name + sig (structural), but omits the empty (doc …).
    let src = "\
def f(x) = x
export { f }";
    let doc = doc_item::project(&program(src), "m");
    let items = doc_items(&doc);
    assert_eq!(items.len(), 1);
    assert_eq!(item_name(&doc, items[0]), Some("f"));
    assert!(
        item_sig_node(&doc, items[0]).is_some(),
        "sig is always present (a structured sub-AST)"
    );
    assert_eq!(
        item_doc(&doc, items[0]),
        None,
        "no /// → no (doc …) child (not an empty one)"
    );
}

#[test]
fn projects_exported_type_and_effect_items() {
    // A doc-item is minted for an exported `type` and `effect`, not just `def`.
    let src = "\
/// A color.
type Color = | Red | Green
/// A logger.
effect Log = | info : Str
export { Color, Log }";
    let doc = doc_item::project(&program(src), "m");
    let items = doc_items(&doc);
    let names: Vec<_> = items.iter().filter_map(|&i| item_name(&doc, i)).collect();
    assert!(names.contains(&"Color"), "exported type is a doc-item");
    assert!(names.contains(&"Log"), "exported effect is a doc-item");
}

#[test]
fn a_def_nested_inside_another_form_is_not_a_top_level_doc_item() {
    // The selection is a QUERY over the whole program, but a doc-item is a TOP-LEVEL public item —
    // a `def` nested inside another form's body (here a `def` bound inside `outer`'s body via a `do`)
    // must NOT surface as a module item, even though the query matches it structurally. This pins the
    // top-level scope filter that distinguishes "module's public surface" from "any def anywhere".
    let src = "\
def outer(x) = (do (def inner(y) = y) (inner x))
export { outer }";
    let doc = doc_item::project(&program(src), "m");
    let items = doc_items(&doc);
    let names: Vec<_> = items.iter().filter_map(|&i| item_name(&doc, i)).collect();
    assert_eq!(
        names,
        vec!["outer"],
        "only the top-level exported `outer` is a doc-item; the nested `inner` def is not"
    );
}

#[test]
fn the_projected_doc_ast_round_trips_through_the_codec_byte_identically() {
    // THE GATE (design §7.2): a doc-module is ordinary cdzast — encode → \x00\x01 → decode is
    // structurally identical, and re-encoding the decoded arena is BYTE-identical (the bijection).
    let src = "\
/// doc one.
def a(x) = x
/// doc two.
def b(y, z) = y
export { a, b }";
    let doc = doc_item::project(&program(src), "roundtrip");

    let bytes = codec::encode(&doc);
    assert_eq!(&bytes[..8], b"cdzast\x00\x01", "canonical inline header");
    let decoded = codec::decode(&bytes).expect("a doc-module decodes");
    assert!(
        doc.structurally_eq(&decoded),
        "decode(encode(doc-module)) must be structurally identical"
    );
    assert_eq!(
        bytes,
        codec::encode(&decoded),
        "re-encode is byte-identical (the frozen bijection)"
    );
}

#[test]
fn the_projected_doc_ast_re_reads_through_the_sexpr_surface() {
    // A doc-module prints to s-expr and re-reads to a structurally identical arena — it is a normal
    // arena to every surface, so the s-expr oracle (a different code path) round-trips it.
    let src = "\
/// prose.
def f(x) = x
export { f }";
    let doc = doc_item::project(&program(src), "m");

    // Print the doc-AST as raw s-expr, then read it back through the independent s-expr reader (a
    // different code path from the codec — the corpus round-trip oracle).
    // `sexpr::read` (single form — NOT `read_all`, which wraps a lone top-level form in a synthetic
    // `(do …)`) so the re-read is the doc-module itself, not a do-wrapped one.
    let printed = sexpr::print(&doc);
    let reread = sexpr::read(&printed)
        .unwrap_or_else(|e| panic!("doc-module s-expr must re-read, got {e:?} for:\n{printed}"));
    assert!(
        doc.structurally_eq(&reread),
        "doc-module must re-read structurally identically through the s-expr surface\nprinted:\n{printed}"
    );
}

#[test]
fn an_empty_program_projects_an_empty_doc_module() {
    // A program that exports nothing → a doc-module with a name and no doc-items (never a panic).
    let src = "\
def f(x) = x";
    let doc = doc_item::project(&program(src), "empty");
    assert!(
        doc.as_form(doc.root, "doc-module").is_some(),
        "still a doc-module"
    );
    assert!(
        doc_items(&doc).is_empty(),
        "nothing exported → no doc-items"
    );
}
