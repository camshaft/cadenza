//! The `CompileOutput` ENVELOPE wire — the whole `{artifacts, diagnostics}` compile result as ONE
//! canonical BINARY AST payload (`cadenza_ast::codec`), the SAME wire every compile-boundary artifact
//! speaks (operator seq-254/seq-284: "Binary AST is THE data exchange format. No exceptions.").
//!
//! WHY this exists: a reducer-world guest is invoked as a pure function through `run(program, contract,
//! input) -> result<payload, error>` — it returns exactly ONE `payload`. But `compile` yields a LIST of
//! kinded artifacts (component + sidecar + spans + result-types …) PLUS a diagnostics list. This codec is
//! the multi-artifact → single-payload ENVELOPE the `cdz-platform.rcdzc.compile` contract's OUTPUT type is
//! carried as: the guest [`encode_compile_output`]s the whole result into the `close` reason, and a caller
//! [`decode_compile_output`]s the returned `run` payload back. (Design: `design/DESIGN-reducer-targets.md`
//! §5.) The `input` side reuses the same artifact-list encoding (`compile` already takes a `&[Artifact]`).
//!
//! Shape: a root `(list [artifacts, diagnostics-bytes])`. `artifacts` is a `(list [artifact-form, …])`
//! where each artifact is `(list [kind-Str, name-Str, bytes-Bytes])`. `diagnostics-bytes` is a `Bytes`
//! leaf holding [`crate::encode_diagnostics`]'s own canonical-binary blob — the existing diagnostics codec
//! reused WHOLESALE (neither module reaches into the other's private forms). TOTAL on decode: a malformed
//! tree / wrong-shape entry degrades to empty rather than panicking, the same graceful-degrade the
//! diagnostics wire gives.
//!
//! NOTE — the `CompileOutput` diagnostic-METRIC fields (`cse_partition_core_eq_calls` et al.) are NOT
//! encoded: they are rcdzc-internal regression-guard counters ("always 0 outside rcdzc's emit path", see
//! `abi.rs`), not part of the platform-facing `{artifacts, diagnostics}` contract output. `decode_compile_
//! output` reconstructs them as `0`, so a round-trip is exact for any `CompileOutput` whose metrics are `0`
//! (every value a platform consumer sees).

use crate::abi::{Artifact, CompileOutput};
use crate::diagnostics_wire::{decode_diagnostics, encode_diagnostics};
use cadenza_ast::ast::{Arenas, Builder, Leaf, Struct, StructId};
use std::sync::Arc;

/// Encode a whole `CompileOutput` as the envelope payload — canonical binary AST (see module docs).
/// Round-trips with [`decode_compile_output`] for any output whose metric fields are `0` (the platform
/// case). The metrics are intentionally not carried (see module docs).
pub fn encode_compile_output(out: &CompileOutput) -> Vec<u8> {
    let mut b = Builder::new();
    let artifact_forms: Vec<StructId> = out
        .artifacts
        .iter()
        .map(|a| encode_artifact(&mut b, a))
        .collect();
    let artifacts = b.list(artifact_forms);
    let diags = b.atom_leaf(Leaf::Bytes(Arc::from(
        encode_diagnostics(&out.diagnostics).as_slice(),
    )));
    let root = b.list(vec![artifacts, diags]);
    cadenza_ast::codec::encode(&b.finish(root))
}

/// Decode the envelope payload back into a `CompileOutput` — the inverse of [`encode_compile_output`],
/// read via the shared `cadenza_ast::codec`. TOTAL: a malformed / wrong-shape payload degrades to an
/// empty output rather than failing. The diagnostic-metric fields are reconstructed as `0` (not carried).
pub fn decode_compile_output(bytes: &[u8]) -> CompileOutput {
    let Some(a) = cadenza_ast::codec::decode(bytes) else {
        return empty();
    };
    let Struct::List(top) = a.get(a.root).clone() else {
        return empty();
    };
    let artifacts = match top.first().map(|&id| a.get(id).clone()) {
        Some(Struct::List(forms)) => forms
            .iter()
            .filter_map(|&f| decode_artifact(&a, f))
            .collect(),
        _ => Vec::new(),
    };
    let diagnostics = top
        .get(1)
        .and_then(|&id| as_bytes(&a, id))
        .map(decode_diagnostics)
        .unwrap_or_default();
    CompileOutput {
        artifacts,
        diagnostics,
        ..empty()
    }
}

/// An all-metrics-`0`, empty-artifacts/diagnostics `CompileOutput` — the decode-degrade value and the
/// struct-update base that fills the non-carried metric fields.
fn empty() -> CompileOutput {
    CompileOutput {
        artifacts: Vec::new(),
        diagnostics: Vec::new(),
        cse_partition_core_eq_calls: 0,
        value_range_uncached_calls: 0,
        param_apply_extra_handled_calls: 0,
        is_cse_shareable_uncached_calls: 0,
    }
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

/// Read a `Bytes` leaf's raw bytes — the `Arenas` accessor `as_str`/`as_int` have no `Bytes` twin, so this
/// mirrors them for the `bytes` artifact field + the nested diagnostics blob.
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
    use crate::abi::{Diagnostic, DiagnosticFix, FixKind, Severity};

    // The envelope round-trips the FULL {artifacts, diagnostics} output exactly: multiple kinded artifacts
    // (a "component" with high/zero bytes, an empty-bytes sidecar), a coded error with a verified fix, and a
    // warning — the same drift guard the reducer-target guest + its callers rely on. Metrics stay 0 (not
    // carried), which is every platform value.
    #[test]
    fn compile_output_envelope_round_trips() {
        let out = CompileOutput {
            artifacts: vec![
                Artifact::new("component", "main", vec![0x00, 0xde, 0xad, 0xff, 0x01]),
                Artifact::new("sidecar", "debug", Vec::new()),
                Artifact::new("result-types", "types", b"main\tInt64".to_vec()),
            ],
            diagnostics: vec![
                Diagnostic {
                    severity: Severity::Warning,
                    code: Some("CDZ0100".into()),
                    message: "unused binding".into(),
                    node: Some(7),
                    fix: Some(DiagnosticFix {
                        label: "delete the binding".into(),
                        kind: FixKind::Delete,
                        node: 7,
                        replacement: String::new(),
                        verified: false,
                    }),
                },
                Diagnostic {
                    severity: Severity::Error,
                    code: None,
                    message: "unsupported construct".into(),
                    node: None,
                    fix: None,
                },
            ],
            cse_partition_core_eq_calls: 0,
            value_range_uncached_calls: 0,
            param_apply_extra_handled_calls: 0,
            is_cse_shareable_uncached_calls: 0,
        };
        assert_eq!(decode_compile_output(&encode_compile_output(&out)), out);
    }

    #[test]
    fn empty_and_garbage_degrade_totally() {
        // An empty output round-trips to empty; a garbage payload decodes to empty (total, never panics).
        let empty_out = empty();
        assert_eq!(
            decode_compile_output(&encode_compile_output(&empty_out)),
            empty_out
        );
        assert_eq!(decode_compile_output(b"not a binary-ast tree"), empty_out);
    }

    // A binary-content artifact (bytes with the full 0x00..=0xff range) survives the envelope unchanged —
    // the artifact `bytes` channel is a raw Bytes leaf, not text, so a real wasm component's bytes are exact.
    #[test]
    fn artifact_bytes_are_binary_exact() {
        let all_bytes: Vec<u8> = (0u16..=255).map(|n| n as u8).collect();
        let out = CompileOutput {
            artifacts: vec![Artifact::new("component", "wasm", all_bytes.clone())],
            ..empty()
        };
        let round = decode_compile_output(&encode_compile_output(&out));
        assert_eq!(round.artifacts.len(), 1);
        assert_eq!(round.artifacts[0].bytes, all_bytes);
    }
}
