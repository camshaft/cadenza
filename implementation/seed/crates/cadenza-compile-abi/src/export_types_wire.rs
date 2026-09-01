//! The `KIND_EXPORT_TYPES` RESULT wire — each exported item's RESOLVED TYPE as a FULL structured `Ty`
//! sub-AST, carried in ONE canonical BINARY AST value (`cadenza_ast::codec`), the SAME wire every
//! compile-boundary artifact speaks (operator seq-254/seq-284/307: "Binary AST is THE data exchange
//! format. No exceptions." + "I want the full type ast!"). The producer (`rcdzc::sidecar::run_query`'s
//! `Query::ExportedTypes`) calls [`encode_export_types`]; the consumer (`cdz doc-module`) calls
//! [`decode_export_types`]. ONE shared codec, so neither side hand-rolls a parser.
//!
//! This RETIRES the bespoke OUTER framing the wire used to carry (a hand-rolled `u32_le count` then per
//! record `u32_le name_len | name | u32_le ty_len | ty_bytes`, where each `ty_bytes` was ITSELF a whole
//! separately-`codec::encode`d `cdzast` artifact). The inner type payload was always the full structured
//! AST; only the outer envelope was bespoke length-prefixed bytes. Now the WHOLE response is a single
//! cadenza-ast value the consumer decodes once with the shared codec — no byte-length reader, no nested
//! artifacts, no post-decode parsing beyond walking the tree.
//!
//! Shape: a root `(export-types <export-type>…)` list, one `(export-type <Str name> <ty-payload>)` form
//! per export, in export (declaration) order. `<ty-payload>` is the resolved type sub-AST GRAFTED
//! directly into this one arena — the same `(-> …)` / `(Sum …)` / `(Record …)` / `(effect (op …)…)`
//! payload `encode_ty_payload` builds, structured-is-truth (NO render-name string). An export whose type
//! does not resolve is OMITTED from the list (the doc-item then gets no `(ty …)` — the graceful-degrade
//! rule). TOTAL on decode: a malformed tree / wrong-shape form is skipped, never a crash.

use crate::graft::copy_from;
use cadenza_ast::ast::{Arenas, Builder, StructId};

/// Encode the exported-item → resolved-type map as the `KIND_EXPORT_TYPES` artifact bytes — ONE canonical
/// binary AST value (see module docs). Each entry's `Arenas` is a standalone arena ROOTED at that export's
/// type payload sub-AST (as the `rcdzc` producer extracts it); its root subtree is grafted verbatim into
/// the shared response arena. Order is preserved. Round-trips with [`decode_export_types`].
pub fn encode_export_types(entries: &[(String, Arenas)]) -> Vec<u8> {
    let mut b = Builder::new();
    let mut forms: Vec<StructId> = Vec::with_capacity(entries.len());
    for (name, ty_arena) in entries {
        let head = b.name("export-type");
        let name_node = b.atom_leaf(cadenza_ast::ast::Leaf::Str(name.as_str().into()));
        let payload = copy_from(&mut b, ty_arena, ty_arena.root);
        forms.push(b.list(vec![head, name_node, payload]));
    }
    let et_head = b.name("export-types");
    let mut children = Vec::with_capacity(forms.len() + 1);
    children.push(et_head);
    children.extend(forms);
    let root = b.list(children);
    cadenza_ast::codec::encode(&b.finish(root))
}

/// Decode the `KIND_EXPORT_TYPES` bytes, DISTINGUISHING a legitimately-ABSENT section from a PRESENT-but-
/// MALFORMED one — the decode-validity contract (operator directive): a non-empty section that fails to
/// decode, or whose root is not an `export-types` form, is a compiler BUG the caller must fail LOUD on,
/// not silently degrade. EMPTY `bytes` → `Ok(vec![])`; non-empty that fails `codec::decode` → `Err`;
/// decoded but wrong root → `Err`. The `Err` carries an actionable message. Twin of
/// [`crate::result_types_wire::decode_result_types_checked`]; see the lenient [`decode_export_types`].
pub fn decode_export_types_checked(bytes: &[u8]) -> Result<Vec<(String, Arenas)>, String> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let a = cadenza_ast::codec::decode_detailed(bytes).map_err(|e| {
        format!(
            "export-types section is present ({} bytes) but failed to decode ({e:?}) — the compiler \
             emitted a malformed/mismatched binary AST (decode-validity bug), not a typeless program",
            bytes.len()
        )
    })?;
    let Some(forms) = a.as_form(a.root, "export-types") else {
        return Err(
            "export-types section decoded but its root is not an `export-types` form — \
             present-but-wrong-shape compiler output (decode-validity bug)"
                .to_string(),
        );
    };
    Ok(forms
        .to_vec()
        .iter()
        .filter_map(|&f| decode_one(&a, f))
        .collect())
}

/// Decode the `KIND_EXPORT_TYPES` bytes back into export name → standalone type arena pairs — the inverse
/// of [`encode_export_types`], read via the shared `cadenza_ast::codec`. Each returned `Arenas` is a fresh
/// standalone arena whose ROOT is that export's `(ty …)` payload subtree (so a consumer grafts it directly).
/// LENIENT/TOTAL: a malformed section yields no entries. A consumer that must tell PRESENT-but-MALFORMED
/// from legitimately-ABSENT (to fail loud per the decode-validity contract) uses
/// [`decode_export_types_checked`] instead.
pub fn decode_export_types(bytes: &[u8]) -> Vec<(String, Arenas)> {
    decode_export_types_checked(bytes).unwrap_or_default()
}

fn decode_one(a: &Arenas, form: StructId) -> Option<(String, Arenas)> {
    let tail = a.as_form(form, "export-type")?;
    let name = a.as_str(*tail.first()?)?.to_string();
    let payload = *tail.get(1)?;
    let mut b = Builder::new();
    let root = copy_from(&mut b, a, payload);
    Some((name, b.finish(root)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cadenza_ast::ast::{Builder, Leaf};

    /// A standalone `(-> a b)` type arena, as the sidecar extracts for one export's resolved type.
    fn arrow_ty() -> Arenas {
        let mut b = Builder::new();
        let head = b.name("->");
        let a = b.name("a");
        let bb = b.name("b");
        let root = b.list(vec![head, a, bb]);
        b.finish(root)
    }

    /// A `(Sum Option <decl> (Some a) None)`-ish payload, to exercise a richer type shape.
    fn sum_ty() -> Arenas {
        let mut b = Builder::new();
        let head = b.name("Sum");
        let nm = b.name("Option");
        let some = b.name("Some");
        let none = b.name("None");
        let root = b.list(vec![head, nm, some, none]);
        b.finish(root)
    }

    #[test]
    fn export_types_binary_ast_round_trips() {
        let entries = vec![("f".to_string(), arrow_ty()), ("Opt".to_string(), sum_ty())];
        let decoded = decode_export_types(&encode_export_types(&entries));
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].0, "f");
        assert_eq!(decoded[0].1.head_name(decoded[0].1.root), Some("->"));
        assert_eq!(decoded[1].0, "Opt");
        assert_eq!(decoded[1].1.head_name(decoded[1].1.root), Some("Sum"));
        // The grafted payload is structurally identical to the source subtree.
        assert!(decoded[0].1.structurally_eq(&arrow_ty()));
        assert!(decoded[1].1.structurally_eq(&sum_ty()));
    }

    #[test]
    fn export_types_empty_round_trips() {
        assert!(decode_export_types(&encode_export_types(&[])).is_empty());
    }

    #[test]
    fn export_types_total_on_garbage() {
        // A non-AST / garbage payload decodes to an empty list (total, graceful-degrade — never panics).
        assert!(decode_export_types(b"not a binary-ast tree").is_empty());
    }

    #[test]
    fn export_types_skips_a_malformed_form() {
        // A root list with a wrong-headed form in the middle keeps only the well-formed export-type forms.
        let mut b = Builder::new();
        let et_head = b.name("export-types");
        // a good (export-type "f" (-> a b))
        let good_head = b.name("export-type");
        let good_name = b.atom_leaf(Leaf::Str("f".into()));
        let ar_head = b.name("->");
        let ar_a = b.name("a");
        let ar_b = b.name("b");
        let ar = b.list(vec![ar_head, ar_a, ar_b]);
        let good = b.list(vec![good_head, good_name, ar]);
        // a bogus (nonsense 1) form
        let bad_head = b.name("nonsense");
        let bad = b.list(vec![bad_head]);
        let root = b.list(vec![et_head, good, bad]);
        let bytes = cadenza_ast::codec::encode(&b.finish(root));
        let decoded = decode_export_types(&bytes);
        assert_eq!(decoded.len(), 1, "the bogus form is skipped");
        assert_eq!(decoded[0].0, "f");
    }
}
