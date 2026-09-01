//! The `KIND_RESULT_TYPES` map wire — each boundary export's COMPILED result type as a FULL structured
//! `Ty` sub-AST, carried in ONE canonical BINARY AST value (`cadenza_ast::codec`), the SAME wire every
//! compile-boundary artifact speaks (operator seq-254/seq-284: "Binary AST everywhere. No exceptions." +
//! "I want the full type ast!"). The producer (`rcdzc`'s `compile`, emitting both the standalone artifact
//! and the `cdz-result-type` component custom section) builds each export's type via
//! `rcdzc::eval::encode_ty_payload` and calls [`encode_result_types`]; a consumer calls
//! [`decode_result_types`] and gets each export's FULL type back as a standalone `Arenas` it walks itself.
//!
//! This crate is a GENERIC, full-fidelity codec — it performs NO render-specific projection. A Cadenza type
//! is not WIT; it merely supports it, so WIT-erasure disambiguation (a `list<u8>` as `Bytes` vs `List UInt8`,
//! a `string` as a `Symbol`) is a RENDER concern that lives in the consumer (`cdz-run`), NOT here: coupling
//! the shared boundary crate to one consumer's render semantics would force every other tool (a doc
//! generator, a type-diff, a schema export) to inherit a lossy projection or re-walk the tree. So the shape
//! MIRRORS its sibling [`crate::export_types_wire`] exactly — full Ty in, full Ty out — and each caller maps
//! the returned `Arenas` to whatever it needs.
//!
//! Shape: a root `(result-types <result-type>…)` list, one `(result-type <Str name> <ty-payload>)` form per
//! boundary export, in export order. `<ty-payload>` is the resolved type sub-AST grafted verbatim — the same
//! `(-> …)` / `(Sum …)` / `(Record …)` / `(List …)` / scalar payload `encode_ty_payload` builds. TOTAL on
//! decode: a malformed / wrong-shape form is skipped, never a crash.

use cadenza_ast::ast::Arenas;

/// Encode the export→result-type map as the `KIND_RESULT_TYPES` artifact / `cdz-result-type` section
/// bytes — ONE canonical binary AST value (see module docs). Each entry's `Arenas` is a standalone arena
/// ROOTED at that export's type payload sub-AST (as the `rcdzc` producer extracts it via
/// `encode_ty_payload`); its root subtree is grafted verbatim into the shared response arena. Order is
/// preserved. Round-trips with [`decode_result_types`].
pub fn encode_result_types(entries: &[(String, Arenas)]) -> Vec<u8> {
    crate::ty_map_wire::encode("result-type", "result-types", entries)
}

/// Decode the `KIND_RESULT_TYPES` bytes, DISTINGUISHING a legitimately-ABSENT section from a PRESENT-but-
/// MALFORMED one — the decode-validity contract (operator directive): wherever an expected value IS a
/// cadenza-AST, a decode failure means the compiler emitted a malformed/mismatched AST — a BUG the caller
/// must fail LOUD on, never silently degrade (e.g. `cdz-run` rendering TYPE-BLIND, the Qty-erases-to-raw-
/// bytes class). Contract:
///   * EMPTY `bytes` → `Ok(vec![])` — no result-types section was emitted (a legitimately typeless program).
///   * NON-EMPTY `bytes` whose `codec::decode` fails → `Err` (malformed binary AST — a compiler bug).
///   * decoded but ROOT is not a `result-types` form → `Err` (present-but-wrong-shape — a compiler bug).
///   * a valid `result-types` root → `Ok(entries)` (a wrong-shape *inner* `result-type` form is still
///     skipped by `decode_one`, matching the per-entry tolerance; the SECTION-level shape is what's checked).
///
/// The `Err` carries an actionable message for the fail-loud path. See the lenient [`decode_result_types`].
pub fn decode_result_types_checked(bytes: &[u8]) -> Result<Vec<(String, Arenas)>, String> {
    crate::ty_map_wire::decode_checked("result-type", "result-types", bytes)
}

/// Decode the `KIND_RESULT_TYPES` bytes back into export name → standalone type-arena pairs — the inverse
/// of [`encode_result_types`], read via the shared `cadenza_ast::codec`. Each returned `Arenas` is a fresh
/// standalone arena whose ROOT is that export's full type payload subtree (so a consumer grafts/walks it
/// directly — the FULL structured `Ty`, no projection). LENIENT/TOTAL: a malformed section yields no
/// entries. A consumer that must tell PRESENT-but-MALFORMED from legitimately-ABSENT (to fail loud per the
/// decode-validity contract) uses [`decode_result_types_checked`] instead.
pub fn decode_result_types(bytes: &[u8]) -> Vec<(String, Arenas)> {
    decode_result_types_checked(bytes).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cadenza_ast::ast::{Builder, Leaf, StructId};

    /// A standalone type-payload arena rooted at `root_build`'s node — as the producer extracts one
    /// export's `encode_ty_payload` subtree.
    fn ty(root_build: impl FnOnce(&mut Builder) -> StructId) -> Arenas {
        let mut b = Builder::new();
        let root = root_build(&mut b);
        b.finish(root)
    }

    #[test]
    fn result_types_full_ty_binary_ast_round_trips() {
        // The FULL type survives verbatim: a Bytes leaf, a (List (UInt 8)) byte-list, an (-> Int64 Symbol)
        // arrow, a (Record (: id Symbol)). The consumer walks each returned arena itself.
        let bytes_ty = || ty(|b| b.name("Bytes"));
        let list_u8 = || {
            ty(|b| {
                let head = b.name("List");
                let uint = b.name("UInt");
                let w = b.atom_leaf(Leaf::Int {
                    value: cadenza_ast::ast::IntValue::from_i64(8),
                    radix: cadenza_ast::ast::Radix::Dec,
                });
                let inner = b.list(vec![uint, w]);
                b.list(vec![head, inner])
            })
        };
        let arrow = || {
            ty(|b| {
                let h = b.name("->");
                let p = b.name("Int64");
                let r = b.name("Symbol");
                b.list(vec![h, p, r])
            })
        };
        let entries = vec![
            ("g".to_string(), bytes_ty()),
            ("xs".to_string(), list_u8()),
            ("f".to_string(), arrow()),
        ];
        let decoded = decode_result_types(&encode_result_types(&entries));
        assert_eq!(decoded.len(), 3);
        // Names preserved in order; each payload is STRUCTURALLY IDENTICAL to its source — full fidelity,
        // no projection (a leaf `Bytes`, a `(List (UInt 8))`, an `(-> Int64 Symbol)` all survive verbatim).
        assert_eq!(decoded[0].0, "g");
        assert!(decoded[0].1.structurally_eq(&bytes_ty()));
        assert_eq!(decoded[1].0, "xs");
        assert!(decoded[1].1.structurally_eq(&list_u8()));
        assert_eq!(decoded[2].0, "f");
        assert!(decoded[2].1.structurally_eq(&arrow()));
    }

    #[test]
    fn empty_and_garbage_are_total() {
        assert!(decode_result_types(&encode_result_types(&[])).is_empty());
        assert!(decode_result_types(b"not a binary-ast tree").is_empty());
    }

    #[test]
    fn a_malformed_form_is_skipped() {
        // A root list with a wrong-headed form in the middle keeps only the well-formed result-type forms.
        let mut b = Builder::new();
        let rt_head = b.name("result-types");
        let good_head = b.name("result-type");
        let good_name = b.atom_leaf(Leaf::Str("g".into()));
        let good_ty = b.name("Bytes");
        let good = b.list(vec![good_head, good_name, good_ty]);
        let bad_head = b.name("nonsense");
        let bad = b.list(vec![bad_head]);
        let root = b.list(vec![rt_head, good, bad]);
        let bytes = cadenza_ast::codec::encode(&b.finish(root));
        let decoded = decode_result_types(&bytes);
        assert_eq!(decoded.len(), 1, "the bogus form is skipped");
        assert_eq!(decoded[0].0, "g");
    }

    #[test]
    fn checked_distinguishes_absent_malformed_and_valid() {
        // The decode-validity contract: EMPTY = legitimately absent (Ok empty); NON-EMPTY that fails
        // codec::decode = malformed (Err); decoded-but-wrong-root = malformed (Err); valid = Ok(entries).
        // ABSENT: no section → Ok(empty), NOT an error.
        assert_eq!(decode_result_types_checked(&[]), Ok(Vec::new()));

        // MALFORMED (non-empty garbage that codec::decode rejects) → Err, NOT a silent empty.
        let garbage = b"not a binary AST at all".to_vec();
        assert!(
            cadenza_ast::codec::decode(&garbage).is_none(),
            "precondition: garbage doesn't decode"
        );
        assert!(
            decode_result_types_checked(&garbage).is_err(),
            "present-but-malformed must be an Err (fail-loud), not a silent typeless fallback"
        );
        // and the lenient variant still swallows it (unchanged behavior for total callers).
        assert!(decode_result_types(&garbage).is_empty());

        // PRESENT-but-WRONG-ROOT: a valid binary AST whose root is not a `result-types` form → Err.
        let mut b = Builder::new();
        let head = b.name("something-else");
        let root = b.list(vec![head]);
        let wrong = cadenza_ast::codec::encode(&b.finish(root));
        assert!(
            decode_result_types_checked(&wrong).is_err(),
            "decoded-but-wrong-root must be an Err (present-but-wrong-shape compiler output)"
        );

        // VALID: a real round-trip decodes to Ok(entries) via the checked path too.
        let mut b = Builder::new();
        let rt_head = b.name("result-types");
        let e_head = b.name("result-type");
        let e_name = b.atom_leaf(Leaf::Str("x".into()));
        let e_ty = b.name("Bytes");
        let e = b.list(vec![e_head, e_name, e_ty]);
        let root = b.list(vec![rt_head, e]);
        let bytes = cadenza_ast::codec::encode(&b.finish(root));
        let ok = decode_result_types_checked(&bytes).expect("valid section decodes Ok");
        assert_eq!(ok.len(), 1);
        assert_eq!(ok[0].0, "x");
    }
}
