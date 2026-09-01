//! Render a self-describing `(: value type)` binary-AST VALUE doc to its canonical surface string, via the
//! ONE canonical printer (`cadenza_syntax::convert::render_binary`) — the SAME path cdz-run's wasm value
//! render uses (op-seq-283: one canonical path, not a bespoke per-target renderer; the tuple renders
//! `(tuple …)`, NOT `#tuple`). This is the CONSUMER half of the rust value-doc emit: the emitted rust guest
//! builds a codec doc from its result value (a `cadenza_ast` `(: <value> <type-node>)` AST — the shape
//! cdz-run's `value_codec` emits) and prints the bytes; this turns those bytes back into the graded surface
//! string. Once the emit produces docs for every result shape, this REPLACES the type-note-driven
//! `cdz_render_at` string walk and deletes cdz-rust-render's hand-rolled parser (the operator-directed
//! parser-elimination; render-ty owns the pinned `render_binary` contract, #7424).

use anyhow::Result;
use cadenza_syntax::convert::{self, Format, FragmentKind, Options};

/// Render a binary-AST value doc (`codec::encode` of a `(: value type)` AST) to its canonical sexpr surface.
/// The kind is `Expr` (a value fragment); the canonical printer renders the idiomatic surface from the AST
/// shape (so `(: value type)` prints as-is and a value tuple prints `(tuple …)`).
pub fn render_value_doc(bytes: &[u8]) -> Result<String> {
    convert::render_binary(bytes, Format::Sexpr, FragmentKind::Expr, Options::default())
        .map(|s| s.trim_end().to_string())
        .map_err(|e| anyhow::anyhow!("render_binary of a rust value doc failed: {e:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cadenza_ast::ast::{Builder, IntValue, Leaf, Radix};
    use cadenza_ast::codec;

    // Pins the ENTIRE consumer recipe end-to-end: construct the value doc EXACTLY as the emit will
    // (cadenza_ast::Builder → codec::encode), then render it. The node tree matches `cdz convert -t debug`
    // of `(: 42 Int64)`:  List[ Atom Name ":", Atom Int 42 (dec), Atom Name "Int64" ]. If this passes, the
    // rust value-doc emit's scalar path is proven modulo the guest-side value walk (Inc 1b).
    #[test]
    fn render_value_doc_scalar_int64_matches_canonical() {
        let mut b = Builder::new();
        let colon = b.name(":");
        let val = b.atom_leaf(Leaf::Int {
            value: IntValue::from_i64(42),
            radix: Radix::Dec,
        });
        let ty = b.name("Int64");
        let root = b.list(vec![colon, val, ty]);
        let arenas = b.finish(root);
        let bytes = codec::encode(&arenas);
        assert_eq!(render_value_doc(&bytes).unwrap(), "(: 42 Int64)");
    }

    // A NEGATIVE scalar (distinct codec Int tag) + a Bool-ish type name, pinning the sign path + that a
    // non-Int64 type name round-trips verbatim.
    #[test]
    fn render_value_doc_negative_int() {
        let mut b = Builder::new();
        let colon = b.name(":");
        let val = b.atom_leaf(Leaf::Int {
            value: IntValue::from_i64(-7),
            radix: Radix::Dec,
        });
        let ty = b.name("Int64");
        let root = b.list(vec![colon, val, ty]);
        let arenas = b.finish(root);
        let bytes = codec::encode(&arenas);
        assert_eq!(render_value_doc(&bytes).unwrap(), "(: -7 Int64)");
    }
}
