//! The compile REQUEST wire — the kinded-input bundle a `run`-callable rcdzc guest receives as its
//! `message.payload`: a canonical binary-AST list of `{kind, name, bytes}` artifacts (the same `&[Artifact]`
//! `rcdzc::compile` takes — "all of the sidecar and inputs as a single message", operator). This is the
//! INPUT type of the `cdz-platform.rcdzc.compile` contract, symmetric with the response envelope
//! (`cadenza_compile_abi::encode_compile_output`). Binary AST is THE data-exchange format (operator
//! seq-254/284), same wire as every other compile-boundary artifact.
//!
//! Shape: a root `(list [artifact-form, …])`, each artifact `(list [kind-Str, name-Str, bytes-Bytes])`.
//! TOTAL on decode (malformed / wrong-shape -> skipped), mirroring the diagnostics + envelope wires.
//!
//! The artifact-form encode/decode intentionally mirrors `cadenza_compile_abi::compile_output_wire`'s
//! (they encode independent payloads — request inputs vs response outputs — so they need only be
//! self-consistent, not identical to each other). Kept HERE (guest glue) rather than promoted into
//! `cadenza-compile-abi` to avoid re-gating that foundational crate (a change there forces a full corpus
//! re-grade); promote to a shared artifact-list codec the next time `cadenza-compile-abi` is touched.

use cadenza_ast::ast::{Arenas, Builder, Leaf, Struct, StructId};
use cadenza_compile_abi::Artifact;
use std::sync::Arc;

/// Encode a kinded-input bundle as the request payload — canonical binary AST (see module docs). Round-trips
/// with [`decode_compile_request`].
pub fn encode_compile_request(inputs: &[Artifact]) -> Vec<u8> {
    let mut b = Builder::new();
    let forms: Vec<StructId> = inputs.iter().map(|a| encode_artifact(&mut b, a)).collect();
    let root = b.list(forms);
    cadenza_ast::codec::encode(&b.finish(root))
}

/// Decode the request payload back into the kinded-input bundle — the inverse of [`encode_compile_request`].
/// TOTAL: a malformed / wrong-shape payload decodes to an empty input list rather than failing.
pub fn decode_compile_request(bytes: &[u8]) -> Vec<Artifact> {
    let Some(a) = cadenza_ast::codec::decode(bytes) else {
        return Vec::new();
    };
    let Struct::List(forms) = a.get(a.root).clone() else {
        return Vec::new();
    };
    forms
        .iter()
        .filter_map(|&f| decode_artifact(&a, f))
        .collect()
}

fn encode_artifact(b: &mut Builder, art: &Artifact) -> StructId {
    let kind = b.atom_leaf(Leaf::Str(art.kind.as_str().into()));
    let name = b.atom_leaf(Leaf::Str(art.name.as_str().into()));
    let bytes = b.atom_leaf(Leaf::Bytes(Arc::from(art.bytes.as_slice())));
    b.list(vec![kind, name, bytes])
}

fn decode_artifact(a: &Arenas, form: StructId) -> Option<Artifact> {
    let Struct::List(c) = a.get(form) else {
        return None;
    };
    Some(Artifact {
        kind: a.as_str(*c.first()?)?.to_string(),
        name: a.as_str(*c.get(1)?)?.to_string(),
        bytes: as_bytes(a, *c.get(2)?)?.to_vec(),
    })
}

fn as_bytes(a: &Arenas, id: StructId) -> Option<&[u8]> {
    match a.get(id) {
        Struct::Atom(l) => match a.leaf(*l) {
            Leaf::Bytes(b) => Some(b),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_request_round_trips_a_kinded_bundle() {
        // A multi-artifact bundle (an ast input + a sidecar input, one with binary bytes) round-trips exactly
        // — the guest reconstructs the same `&[Artifact]` the caller sent.
        let inputs = vec![
            Artifact::new(Artifact::KIND_AST, "main", vec![0x00, 0xde, 0xad, 0xff]),
            Artifact::new("sidecar", "spans", b"span-table".to_vec()),
        ];
        assert_eq!(
            decode_compile_request(&encode_compile_request(&inputs)),
            inputs
        );
    }

    #[test]
    fn empty_and_garbage_decode_to_empty() {
        assert!(decode_compile_request(&encode_compile_request(&[])).is_empty());
        assert!(decode_compile_request(b"not a binary-ast tree").is_empty());
    }
}
