//! Surface tests relocated UP into the facade from the split-out bottom crates.
//!
//! `cadenza-syntax-core` (home of `span`/`spans`) and `cadenza-syntax-cedar` sit BELOW the surface
//! readers, so a test that drives input through a reader (`sexpr`/`parser`) + `canon`, or that exercises
//! cedar's ML-printer fallback, cannot live in those crates (they may not depend on the ML surface).
//! It lives here where every surface is available. In-crate `#[cfg(test)] mod` per the crate's
//! no-integration-tests house style (NOT a `tests/*.rs` binary).

use crate::{canon, parser, sexpr};

#[test]
fn node_at_offset_finds_the_innermost_node() {
    // `(+ a b)` — hovering the `a` returns the `a` leaf, NOT the enclosing `(+ a b)` list, because the
    // innermost (smallest) containing span wins.
    let src = "(+ a b)";
    let (arenas, spans) = sexpr::read_all_spanned(src).expect("parse");
    let a_off = src.find('a').unwrap();
    let node = spans.node_at_offset(a_off).expect("a node at `a`");
    assert_eq!(
        arenas.as_name(node),
        Some("a"),
        "innermost node under `a` is the `a` leaf"
    );

    // An offset on the `+` head returns the `+` leaf, not the list.
    let plus_off = src.find('+').unwrap();
    let head = spans.node_at_offset(plus_off).expect("a node at `+`");
    assert_eq!(arenas.as_name(head), Some("+"));
}

#[test]
fn node_at_offset_past_the_source_is_none() {
    let (_, spans) = sexpr::read_all_spanned("(+ 1 2)").expect("parse");
    assert_eq!(spans.node_at_offset(9999), None);
}

#[test]
fn remap_then_resolve_round_trips_a_cursor_through_canonicalization() {
    // End-to-end pairing with `canon::canonicalize_with_map` (the id_map producer): a span table built
    // by the NON-canonical ML reader, remapped through the canonical id_map, must resolve a cursor to the
    // SAME source node under the canonical ids — the ids `codec::encode` (and hence the compiler) uses.
    // This is the `ml-parser-node-order` fix exercised through the whole chain.
    let parsed = parser::read_ml("def add(a, b) = a + b");
    let (canon, id_map) = canon::canonicalize_with_map(&parsed.arenas);
    let remapped = parsed.spans.remap(&id_map, canon.structure.len());
    // The body `a` is at byte 16 (`… = a + b`). Resolving that offset in the REMAPPED table yields a
    // canonical id whose node is the `a` atom — proving remap re-keyed the span to the canonical id.
    let body_a = "def add(a, b) = a + b".rfind('a').unwrap(); // byte 16
    let id = remapped
        .node_at_offset(body_a)
        .expect("a canonical node contains the body `a` offset");
    assert_eq!(
        canon.as_name(id),
        Some("a"),
        "the cursor at the body `a` resolves to an `a` atom in the canonical arena"
    );
    // The remapped table is sized to the canonical arena (1:1), and preserves the file id.
    assert_eq!(remapped.len(), canon.structure.len());
    assert_eq!(remapped.file(), parsed.spans.file());
}

/// The cedar surface's ML-printer FALLBACK (a non-Cedar root handed to `--to cedar` → a `//`-comment
/// block that re-reads to an empty policy set). Needs the ML printer + a reader, so it lives here rather
/// than in `cadenza-syntax-cedar`. Gated on the `cedar` feature.
#[cfg(feature = "cedar")]
#[test]
fn cedar_non_root_falls_back_to_comments() {
    use crate::{Struct, cedar, printer};
    let prog = sexpr::read("(+ 1 2)").unwrap();
    let out = cedar::print(&prog, 100, printer::print);
    assert!(
        out.starts_with("// "),
        "fallback yields a comment block, got {out}"
    );
    let back = cedar::read(&out).expect("comment-only cedar is valid");
    assert_eq!(back.head_name(back.root), Some("cedar-policyset"));
    match back.get(back.root) {
        Struct::List(items) => assert_eq!(items.len(), 1, "empty set — head only"),
        _ => panic!("cedar-policyset root is a list"),
    }
}
