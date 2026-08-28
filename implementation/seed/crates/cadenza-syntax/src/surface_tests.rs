//! Surface tests relocated UP into the facade from the split-out bottom crates.
//!
//! `cadenza-syntax-core` (home of `span`/`spans`) and `cadenza-syntax-cedar` sit BELOW the surface
//! readers, so a test that drives input through a reader (`sexpr`/`parser`) + `canon`, or that exercises
//! cedar's ML-printer fallback, cannot live in those crates (they may not depend on the ML surface).
//! It lives here where every surface is available. In-crate `#[cfg(test)] mod` per the crate's
//! no-integration-tests house style (NOT a `tests/*.rs` binary).

use crate::span::Span;
use crate::{Builder, Struct, StructId, canon, parser, query, sexpr};

// ---- s-expr cross-surface tests relocated from `cadenza-syntax-sexpr` --------------------------------
// These need the ML `parser` (byte-soup arena source) or `query::Tree` (subtree materialization) — both
// ABOVE the sexpr crate — so they live here. The tiny deterministic generators they share with the sexpr
// crate's own tests are duplicated (test-only, ~25 lines).

struct SplitMix64(u64);
impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

fn gen_pretty_prog(rng: &mut SplitMix64, depth: usize) -> String {
    let names = ["a", "b", "x", "y", "f", "+", "g", "\"s\"", "42", "true"];
    if depth == 0 || rng.next().is_multiple_of(3) {
        return names[(rng.next() as usize) % names.len()].to_string();
    }
    let sub = |rng: &mut SplitMix64| gen_pretty_prog(rng, depth - 1);
    match rng.next() % 6 {
        0 => format!("(+ {} {})", sub(rng), sub(rng)),
        1 => format!("(f {} {} {})", sub(rng), sub(rng), sub(rng)),
        2 => format!("(if {} {} {})", sub(rng), sub(rng), sub(rng)),
        3 => format!("(let ((x {}) (y {})) {})", sub(rng), sub(rng), sub(rng)),
        4 => format!("(record (x {}) (y {}))", sub(rng), sub(rng)),
        _ => format!(
            "(match {} ((Some n) {}) ((None _) {}))",
            sub(rng),
            sub(rng),
            sub(rng)
        ),
    }
}

#[test]
fn every_node_span_slices_back_to_that_node_over_generated_programs() {
    // For EVERY node of every generated program, the span must (a) be an in-bounds char-boundary slice,
    // (b) nest inside its parent's span, and (c) slice to text that RE-READS to a tree structurally equal
    // to that node's subtree (materialized via `query::Tree`). A list node additionally brackets `(`…`)`.
    let mut rng = SplitMix64(0x5a4c_0de5_acc0_1a7e);
    let mut nodes_checked = 0usize;
    for _ in 0..3000 {
        let depth = 1 + (rng.next() as usize) % 4;
        let src = gen_pretty_prog(&mut rng, depth);
        let Ok((a, spans)) = sexpr::read_spanned(&src) else {
            continue;
        };
        let full = Span::new(0, src.len());
        let mut stack: Vec<(StructId, Span)> = vec![(a.root, full)];
        while let Some((id, parent)) = stack.pop() {
            let sp = spans.get(id).expect("span table is total");
            assert!(
                sp.start <= sp.end
                    && sp.end <= src.len()
                    && src.is_char_boundary(sp.start)
                    && src.is_char_boundary(sp.end),
                "span {sp:?} out of bounds / off a char boundary in {src:?}"
            );
            assert!(
                parent.start <= sp.start && sp.end <= parent.end,
                "node span {sp:?} escapes parent {parent:?} in {src:?}"
            );
            let text = &src[sp.start..sp.end];
            let sub = query::Tree::from_arena(&a, id).to_arena();
            let reparsed = sexpr::read(text).unwrap_or_else(|e| {
                panic!("node span text {text:?} must re-read ({e:?}) in {src:?}")
            });
            assert!(
                reparsed.structurally_eq(&sub),
                "node span text {text:?} re-reads to a DIFFERENT tree than the node it spans in {src:?}"
            );
            if let Struct::List(items) = a.get(id) {
                assert!(
                    text.starts_with('(') && text.ends_with(')'),
                    "a list node's span {text:?} must bracket parens in {src:?}"
                );
                for &child in items {
                    stack.push((child, sp));
                }
            }
            nodes_checked += 1;
        }
    }
    assert!(
        nodes_checked >= 3000,
        "swept a meaningful node population, got {nodes_checked}"
    );
}

#[test]
fn sexpr_printer_is_total_over_arbitrary_arenas() {
    // The s-expr PRINTER over the full diversity of arena SHAPES — including shapes no s-expr TEXT
    // produces — sourced from `read_ml` on byte-soup (it recovers, never bails). Neither `print` (flat)
    // nor `print_pretty_width` (layout) may PANIC at any width, both outputs re-read without panicking,
    // and flat/pretty agree structurally.
    let alphabet: Vec<char> = "()[]{}|,;=>-+*/<:.@#`\"\\ \tabcdefimntxλ中0123456789\n"
        .chars()
        .collect();
    let mut rng = SplitMix64(0xc0de_5e37_a5f1_0d1c);
    for len in 0..=32usize {
        for _ in 0..80 {
            let s: String = (0..len)
                .map(|_| alphabet[(rng.next() as usize) % alphabet.len()])
                .collect();
            let parsed = parser::read_ml(&s);
            let a = &parsed.arenas;
            let flat = sexpr::print(a); // must not panic
            let flat_back = sexpr::read_all(&flat); // must not panic
            for width in [0usize, 1, 20, 80] {
                let pretty = sexpr::print_pretty_width(a, width); // must not panic
                let pretty_back = sexpr::read_all(&pretty); // must not panic
                if let (Ok(f), Ok(p)) = (&flat_back, &pretty_back) {
                    assert!(
                        f.structurally_eq(p),
                        "flat vs pretty (width {width}) differ for {s:?}"
                    );
                }
            }
        }
    }
    // Shapes no s-expr TEXT yields, built directly, to hit the printer's list arms head-on.
    let mut b = Builder::new();
    let empty = b.list(vec![]);
    let uq_head = b.name("unquote");
    let uq_empty = b.list(vec![uq_head, empty]);
    let a = b.finish(uq_empty);
    let _ = sexpr::print(&a); // must not panic
    for width in [0usize, 1, 40] {
        let _ = sexpr::print_pretty_width(&a, width); // must not panic
    }
}

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

/// The JSON surface's ML-printer FALLBACK (a non-JSON root handed to `--to json` → a JSON string
/// carrying the ML text). Needs the ML printer + a reader, so it lives here rather than in
/// `cadenza-syntax-json`.
#[test]
fn json_non_root_falls_back_to_json_string() {
    use crate::json;
    let prog = sexpr::read("(+ 1 2)").unwrap();
    let out = json::print(&prog, 100, crate::printer::print);
    let back = json::read(&out).expect("fallback output is valid JSON");
    assert!(
        back.as_str(back.root).is_some(),
        "fallback yields a JSON string, got {out}"
    );
}

/// The TOML surface's ML-printer FALLBACK (a non-TOML root handed to `--to toml` → a `program = "<ml>"`
/// key). Needs the ML printer + a reader, so it lives here rather than in `cadenza-syntax-toml`.
#[test]
fn toml_non_root_falls_back_to_program_key() {
    use crate::toml_surface;
    let prog = sexpr::read("(+ 1 2)").unwrap();
    let out = toml_surface::print(&prog, 100, crate::printer::print);
    assert!(
        out.starts_with("program = "),
        "fallback yields a program key, got {out}"
    );
    let back = toml_surface::read(&out).expect("fallback output is valid TOML");
    assert_eq!(back.head_name(back.root), Some("toml-document"));
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
