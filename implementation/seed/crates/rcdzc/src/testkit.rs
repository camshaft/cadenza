//! Shared test fixtures — the s-expr `parse` reader plus builders for the tiny Stage-0 programs the
//! query modules assert over.
//!
//! Kept in one place so `db`, `resolve`, `infer`, and `lower` tests all build the SAME subject AST
//! (a change to the slice's shape updates one builder, not four). `parse` reads the s-expression
//! surface through the REAL front-end (`cadenza-syntax::sexpr`, a dev-dep) via a byte round-trip, so
//! tests share the corpus gate's reader instead of a hand-rolled one. Compiled only under `#[cfg(test)]`.

#![cfg(test)]

use crate::ast::{Arenas, Builder, IntValue, Leaf, Radix, StructId};

/// Read a test program from the s-expression surface into rcdzc's `Arenas`, using the REAL front-end
/// reader (`cadenza-syntax::sexpr`, a dev-dependency) rather than a hand-rolled one — so a test case is
/// one readable line AND it goes through the exact surface the corpus gate uses (dotted names like
/// `UInt8.wrap` desugar to `(. UInt8 wrap)`, `-3` is an integer, etc.), never a divergent reimplementation.
///
/// The bridge between the two crates' distinct `Arenas` types is BYTES: `sexpr::read` → cadenza-syntax's
/// `codec::encode` → rcdzc's `codec::decode`. This is exactly the round-trip the compiler relies on
/// (`cadenza-syntax` emits the binary AST the gate feeds `rcdzc`), so every test that uses `parse` also
/// EXERCISES that rcdzc's COPIED `codec.rs` stays byte-compatible with `cadenza-syntax` — the invariant
/// the "copy, don't depend" directive rests on, checked rather than assumed. (Tests-only: like
/// `wasm-encoder`/`wasmtime`, this dependency never enters the compile path.)
pub fn parse(src: &str) -> Arenas {
    let arenas = cadenza_syntax::sexpr::read(src)
        .unwrap_or_else(|e| panic!("test s-expr failed to read: {e:?}\n  src: {src}"));
    let bytes = cadenza_syntax::codec::encode(&arenas);
    crate::codec::decode(&bytes)
        .unwrap_or_else(|| panic!("cadenza-syntax bytes failed to decode with rcdzc codec: {src}"))
}

/// Read a test program AND its span side-table, the way a real debug-enabled driver would: the
/// front-end's `read_spanned` produces both the arena and a `SpanTable` keyed by the same `StructId`
/// space, which we project to rcdzc's [`crate::spans::SpanData`] (the `spans` input artifact form).
/// The arena bytes round-trip through the codec exactly as [`parse`] does, so the returned `SpanData`
/// aligns 1:1 with the decoded arena. Used by the debug-info tests to supply the `spans` input.
pub fn parse_spanned(src: &str) -> (Arenas, crate::spans::SpanData) {
    let (arenas, span_table) = cadenza_syntax::sexpr::read_spanned(src)
        .unwrap_or_else(|e| panic!("test s-expr failed to read: {e:?}\n  src: {src}"));
    let bytes = cadenza_syntax::codec::encode(&arenas);
    let decoded = crate::codec::decode(&bytes)
        .unwrap_or_else(|| panic!("cadenza-syntax bytes failed to decode with rcdzc codec: {src}"));
    // Project the front-end SpanTable → rcdzc's (start, len) form, one entry per occurrence in id order.
    let spans: Vec<(u32, u32)> = (0..span_table.len())
        .map(|i| {
            let sp = span_table
                .get(cadenza_syntax::ast::StructId(i as u32))
                .expect("span for every occurrence");
            (sp.start as u32, (sp.end - sp.start) as u32)
        })
        .collect();
    let data = crate::spans::SpanData {
        module_path: "test.cdz".to_string(),
        spans,
        source: src.to_string(),
    };
    (decoded, data)
}

/// Build `(module m (def (main) 42) (export main))` and return `(arenas, the-42-literal-node-id)`.
/// The literal id is what the `type_of` / `core_of` queries are asked about.
pub fn scalar_program() -> (Arenas, StructId) {
    let mut b = Builder::new();
    let module = b.name("module");
    let m = b.name("m");
    // (def (main) 42)
    let def = b.name("def");
    let main_sig_name = b.name("main");
    let sig = b.list(vec![main_sig_name]);
    let body = b.atom_leaf(Leaf::Int {
        value: IntValue::from_i64(42),
        radix: Radix::Dec,
    });
    let def_form = b.list(vec![def, sig, body]);
    // (export main)
    let export = b.name("export");
    let main_ref = b.name("main");
    let export_form = b.list(vec![export, main_ref]);
    let root = b.list(vec![module, m, def_form, export_form]);
    let ast = b.finish(root);
    (ast, body)
}

/// Build `(module m (def (main) (if false 1 2)) (export main))` and return `(arenas, if-node-id)`.
/// The two-way branch case — its id is the `if` node.
pub fn if_program() -> (Arenas, StructId) {
    let mut b = Builder::new();
    let module = b.name("module");
    let m = b.name("m");
    let def = b.name("def");
    let main_sig_name = b.name("main");
    let sig = b.list(vec![main_sig_name]);
    // (if false 1 2)
    let if_head = b.name("if");
    let cond = b.atom_leaf(Leaf::Bool(false));
    let then_ = b.atom_leaf(Leaf::Int {
        value: IntValue::from_i64(1),
        radix: Radix::Dec,
    });
    let else_ = b.atom_leaf(Leaf::Int {
        value: IntValue::from_i64(2),
        radix: Radix::Dec,
    });
    let if_form = b.list(vec![if_head, cond, then_, else_]);
    let def_form = b.list(vec![def, sig, if_form]);
    let export = b.name("export");
    let main_ref = b.name("main");
    let export_form = b.list(vec![export, main_ref]);
    let root = b.list(vec![module, m, def_form, export_form]);
    let ast = b.finish(root);
    (ast, if_form)
}
