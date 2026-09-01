//! Shared codec for the "export name → full structured `Ty` sub-AST" map wires. `export_types_wire` and
//! `result_types_wire` are the same wire shape — a `(<plural> (<singular> <Str name> <ty-payload>)…)` root,
//! each payload a type sub-AST grafted verbatim (`cadenza_ast::codec`) — differing ONLY in the form-name
//! tokens. This module is that one codec, parameterized by the singular/plural head names; the two wire
//! modules are thin wrappers passing their names. The decode-validity error text is parameterized by the
//! plural name, so each wrapper's messages stay byte-identical to the hand-written versions.

use crate::graft::copy_from;
use cadenza_ast::ast::{Arenas, Builder, Leaf, StructId};

/// Encode `entries` as a `(<plural> (<singular> <Str name> <ty-payload>)…)` binary AST value. Each entry's
/// `Arenas` is a standalone arena rooted at that export's type payload; its root subtree is grafted verbatim
/// into the shared response arena. Order is preserved. Inverse of [`decode`].
pub(crate) fn encode(singular: &str, plural: &str, entries: &[(String, Arenas)]) -> Vec<u8> {
    let mut b = Builder::new();
    let mut forms: Vec<StructId> = Vec::with_capacity(entries.len());
    for (name, ty_arena) in entries {
        let head = b.name(singular);
        let name_node = b.atom_leaf(Leaf::Str(name.as_str().into()));
        let payload = copy_from(&mut b, ty_arena, ty_arena.root);
        forms.push(b.list(vec![head, name_node, payload]));
    }
    let root_head = b.name(plural);
    let mut children = Vec::with_capacity(forms.len() + 1);
    children.push(root_head);
    children.extend(forms);
    let root = b.list(children);
    cadenza_ast::codec::encode(&b.finish(root))
}

/// Decode the bytes, DISTINGUISHING a legitimately-ABSENT section from a PRESENT-but-MALFORMED one (the
/// decode-validity contract): EMPTY → `Ok(vec![])`; non-empty that fails `codec::decode` → `Err`; decoded
/// but whose root is not a `<plural>` form → `Err`. A wrong-shape *inner* form is still skipped (per-entry
/// tolerance). The `Err` carries an actionable message. See the lenient [`decode`].
pub(crate) fn decode_checked(
    singular: &str,
    plural: &str,
    bytes: &[u8],
) -> Result<Vec<(String, Arenas)>, String> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let a = cadenza_ast::codec::decode_detailed(bytes).map_err(|e| {
        format!(
            "{plural} section is present ({} bytes) but failed to decode ({e:?}) — the compiler \
             emitted a malformed/mismatched binary AST (decode-validity bug), not a typeless program",
            bytes.len()
        )
    })?;
    let Some(forms) = a.as_form(a.root, plural) else {
        return Err(format!(
            "{plural} section decoded but its root is not a `{plural}` form — \
             present-but-wrong-shape compiler output (decode-validity bug)"
        ));
    };
    Ok(forms
        .to_vec()
        .iter()
        .filter_map(|&f| decode_one(&a, singular, f))
        .collect())
}

fn decode_one(a: &Arenas, singular: &str, form: StructId) -> Option<(String, Arenas)> {
    let tail = a.as_form(form, singular)?;
    let name = a.as_str(*tail.first()?)?.to_string();
    let payload = *tail.get(1)?;
    let mut b = Builder::new();
    let root = copy_from(&mut b, a, payload);
    Some((name, b.finish(root)))
}
