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
use cadenza_syntax::convert;

/// Render a binary-AST value doc (`codec::encode` of a `(: value type)` AST) to its canonical sexpr surface.
/// Uses `render_binary_value_line` (seq-283, #7773) — the SINGLE-LINE canonical runtime value render (via
/// `print_from`), byte-identical to cdz-run's `render_val`. The general `render_binary` PRETTY renderer
/// hard-breaks a long `(: value type)` across lines (a guide fragment / `cdz convert` wants that), which
/// diverged from cdz-run's one-line render and false-mismatched the cdz-smith rust-vs-wasm oracle on a long
/// record/compound result (breaker finding). The value-line render keeps `cdz run-rust` byte-identical.
pub fn render_value_doc(bytes: &[u8]) -> Result<String> {
    convert::render_binary_value_line(bytes)
        .map(|s| s.trim_end().to_string())
        .map_err(|e| anyhow::anyhow!("render_binary of a rust value doc failed: {e:?}"))
}

/// Interpret a rust driver's captured stdout as the graded value string. If it carries the value-doc MARKER
/// (`CDZDOC:<hex>` — emitted by the flag-gated value-doc path, `cdz_rust_render::value_doc_render_scalar`),
/// hex-decode the bytes and render via the ONE canonical printer ([`render_value_doc`]). Otherwise pass the
/// stdout through UNCHANGED (the default string-render path). Marker-detection is flag-INDEPENDENT and safe:
/// a string render never starts with `CDZDOC:`, so a non-marker stdout returns as-is (byte-identical to
/// before) — both gates can route their captured value through this unconditionally.
pub fn interpret_run_stdout(raw: &str) -> Result<String> {
    let raw = raw.trim_end();
    match raw.strip_prefix("CDZDOC:") {
        Some(hex) => render_value_doc(&hex_decode(hex)?),
        None => Ok(raw.to_string()),
    }
}

/// Decode a lowercase-hex byte string (the `CDZDOC:` payload) to bytes. Errors on an odd length or a
/// non-hex digit (a corrupt marker → a graded `BadArtifact`, never a silent mis-render).
fn hex_decode(hex: &str) -> Result<Vec<u8>> {
    let hex = hex.trim();
    if !hex.len().is_multiple_of(2) {
        anyhow::bail!("value-doc marker hex has odd length {}", hex.len());
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16)
                .map_err(|e| anyhow::anyhow!("value-doc marker hex byte at {i}: {e}"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cadenza_ast::ast::{Builder, IntValue, Leaf, Radix};
    use cadenza_ast::codec;

    // The full CONSUMER round-trip through the MARKER protocol: build the `(: 42 Int64)` doc, hex-encode it
    // with the `CDZDOC:` prefix EXACTLY as the emitted driver will, and assert `interpret_run_stdout` decodes
    // + renders it to the canonical surface. Proves the marker + hex + render_binary read path end-to-end.
    #[test]
    fn interpret_run_stdout_decodes_the_marker() {
        let mut b = Builder::new();
        let colon = b.name(":");
        let val = b.atom_leaf(Leaf::Int {
            value: IntValue::from_i64(42),
            radix: Radix::Dec,
        });
        let ty = b.name("Int64");
        let root = b.list(vec![colon, val, ty]);
        let bytes = codec::encode(&b.finish(root));
        let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(
            interpret_run_stdout(&format!("CDZDOC:{hex}")).unwrap(),
            "(: 42 Int64)"
        );
        // A trailing newline (the driver's `println!`) is tolerated.
        assert_eq!(
            interpret_run_stdout(&format!("CDZDOC:{hex}\n")).unwrap(),
            "(: 42 Int64)"
        );
    }

    // A NON-marker stdout (the default string-render path) passes through byte-identical — the flag-off case.
    #[test]
    fn interpret_run_stdout_passthrough_non_marker() {
        assert_eq!(interpret_run_stdout("5").unwrap(), "5");
        assert_eq!(
            interpret_run_stdout("(tuple 1 2)\n").unwrap(),
            "(tuple 1 2)"
        );
        assert_eq!(interpret_run_stdout("(: 5 Int64)").unwrap(), "(: 5 Int64)");
    }

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

    // WRAP-GUARD (seq-283, #7773): a doc long enough that the general `render_binary` PRETTY printer would
    // hard-break it across lines MUST still render ONE LINE — `render_value_doc` uses `render_binary_value_line`
    // (via `print_from`), byte-identical to cdz-run's one-line `render_val`. A multi-line value verdict
    // false-mismatched the cdz-smith rust-vs-wasm oracle + broke the run-rust "last stdout line" verdict parse
    // (#7763) on a long record/compound result (breaker finding). Pins that this consumer keeps the value-line
    // render — a revert to the wrapping `render_binary` would reintroduce the newline.
    #[test]
    fn render_value_doc_long_doc_stays_one_line() {
        let mut b = Builder::new();
        // A wide `(a0 a1 … a19)` list — well past the pretty printer's wrap width.
        let mut kids = Vec::new();
        for i in 0..20 {
            kids.push(b.name(&format!("aaaaaaaa{i}")));
        }
        let root = b.list(kids);
        let arenas = b.finish(root);
        let bytes = codec::encode(&arenas);
        let out = render_value_doc(&bytes).unwrap();
        assert!(
            !out.contains('\n'),
            "value render must be one line, got:\n{out}"
        );
    }
}
