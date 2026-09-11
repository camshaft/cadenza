//! The `cdz-platform.rcdzc.compile` contract's CANONICAL value codec — the rcdzc guest's request/response as
//! strongly-typed Cadenza VALUES (not the old bare-list `request_wire` / `compile_output_wire`). Operator
//! directive: "use strong contracts in all of those places" — the rcdzc guest decodes a `CompileRequest` value
//! and emits a `CompileResult` value, so the (Cadenza) compile-route handler composes it via
//! `Value.decode : Option(CompileResult)`, the contract type as the literal wire.
//!
//! The types (see `cdz-platform/contracts/userspace/rcdzc-compile.cdz`) — every single-ctor single-payload sum
//! is a NOMINAL NEWTYPE that rcdzc's value form ERASES (elide ctor + ascribe the inner with the type name;
//! `value_form.rs` `Ty::Nominal`); a MULTI-ctor sum (`Severity`, `Option`) keeps `(Ctor payload)` /
//! `(Ctor unit)`:
//!   CompileRequest = | Compile(List(Artifact))            -> `(: #list([(: #record Artifact)…]) CompileRequest)`
//!   Artifact       = | Artifact(Record(kind,name,bytes))  -> `(: #record Artifact)` (elided) per element
//!   CompileResult  = | Compiled(Record(artifacts, diagnostics)) -> `(: #record CompileResult)` (elided)
//!   CompileDiagnostic = | CompileDiagnostic(Record(severity,code,message,node)) -> `(: #record CompileDiagnostic)`
//!   Severity = | Error | Warning                          -> `(Error unit)` / `(Warning unit)`
//!   code: Option(String), node: Option(UInt32)            -> `(Some x)` / `(None unit)`
//! Records are `#record((= field value)…)`, fields ascending NAME order; decoding is TOTAL + ascription-tolerant.
//! (The `DiagnosticFix` is omitted from the contract's first cut, so it is dropped here too.)

use cadenza_compile_abi::{Artifact, Diagnostic, Severity};
use cadenza_value::{self as value, ValueBuilder};

// --- request (input to the rcdzc guest) ----------------------------------------------------------------

/// Encode a kinded-input bundle as a canonical `CompileRequest.Compile(List(Artifact))` value. Round-trips
/// with [`decode_compile_request`].
#[must_use]
pub fn encode_compile_request(inputs: &[Artifact]) -> Vec<u8> {
    let mut b = ValueBuilder::new();
    let arts: Vec<_> = inputs.iter().map(|a| artifact_value(&mut b, a)).collect();
    let list = value::list_value(&mut b, arts);
    // `Compile` ctor elided (Nominal newtype); ascription-free encode (finish_value) — decode is frame-tolerant.
    value::finish_value(b, list).to_vec()
}

/// Decode a canonical `CompileRequest` value back into the kinded-input bundle. TOTAL: malformed -> empty.
#[must_use]
pub fn decode_compile_request(bytes: &[u8]) -> Vec<Artifact> {
    let Some(arenas) = value::decode(bytes) else {
        return Vec::new();
    };
    // `Compile` elided — the root is the ascribed `#list(…)` of artifacts.
    value::read_list(&arenas, arenas.root)
        .map(|elems| {
            elems
                .iter()
                .filter_map(|&e| read_artifact(&arenas, e))
                .collect()
        })
        .unwrap_or_default()
}

// --- result (output of the rcdzc guest) ----------------------------------------------------------------

/// Encode a compile result (`CompileOutput`'s `artifacts` + `diagnostics`) as a canonical
/// `CompileResult.Compiled(Record(artifacts, diagnostics))` value. Round-trips with [`decode_compile_result`]
/// (the `DiagnosticFix` + metric fields are not part of the contract, so `CompileOutput` is not taken whole).
#[must_use]
pub fn encode_compile_result(artifacts: &[Artifact], diagnostics: &[Diagnostic]) -> Vec<u8> {
    let mut b = ValueBuilder::new();
    let arts: Vec<_> = artifacts
        .iter()
        .map(|a| artifact_value(&mut b, a))
        .collect();
    let artifacts = value::list_value(&mut b, arts);
    let diags: Vec<_> = diagnostics
        .iter()
        .map(|d| diagnostic_value(&mut b, d))
        .collect();
    let diagnostics = value::list_value(&mut b, diags);
    let rec = value::record(
        &mut b,
        vec![("artifacts", artifacts), ("diagnostics", diagnostics)],
    );
    // `Compiled` ctor elided (Nominal newtype); ascription-free encode (finish_value) — decode is frame-tolerant.
    value::finish_value(b, rec).to_vec()
}

/// Decode a canonical `CompileResult` value into `(artifacts, diagnostics)` — the inverse of
/// [`encode_compile_result`] (the `DiagnosticFix` + metric fields are not part of the contract, so the full
/// `CompileOutput` is not reconstructed — the Cadenza handler consumes the value, this is for tests/symmetry).
/// TOTAL: malformed -> `(empty, empty)`.
#[must_use]
pub fn decode_compile_result(bytes: &[u8]) -> (Vec<Artifact>, Vec<Diagnostic>) {
    let Some(arenas) = value::decode(bytes) else {
        return (Vec::new(), Vec::new());
    };
    let rec = arenas.root; // `Compiled` elided — the ascribed record
    let artifacts = value::record_field(&arenas, rec, "artifacts")
        .and_then(|f| value::read_list(&arenas, f))
        .map(|es| {
            es.iter()
                .filter_map(|&e| read_artifact(&arenas, e))
                .collect()
        })
        .unwrap_or_default();
    let diagnostics = value::record_field(&arenas, rec, "diagnostics")
        .and_then(|f| value::read_list(&arenas, f))
        .map(|es| {
            es.iter()
                .filter_map(|&e| read_diagnostic(&arenas, e))
                .collect()
        })
        .unwrap_or_default();
    (artifacts, diagnostics)
}

// --- element codecs ------------------------------------------------------------------------------------

/// `Artifact.Artifact(Record(kind, name, bytes))` -> `(: #record Artifact)` (ctor elided).
fn artifact_value(b: &mut ValueBuilder, a: &Artifact) -> value::ValueId {
    let kind = value::str_leaf(b, &a.kind);
    let name = value::str_leaf(b, &a.name);
    let bytes = value::bytes_leaf(b, &a.bytes);
    // Ascription-free: the bare record decodes against `Artifact` by shape (frame-tolerant decode).
    value::record(b, vec![("kind", kind), ("name", name), ("bytes", bytes)])
}

fn read_artifact(arenas: &value::Arenas, id: value::ValueId) -> Option<Artifact> {
    Some(Artifact {
        kind: value::read_str(arenas, value::record_field(arenas, id, "kind")?)?,
        name: value::read_str(arenas, value::record_field(arenas, id, "name")?)?,
        bytes: value::read_bytes(arenas, value::record_field(arenas, id, "bytes")?)?.to_vec(),
    })
}

/// A compile `Diagnostic` -> `(: #record CompileDiagnostic)` (ctor elided); `severity`/`code`/`node` are
/// multi-ctor sums, so kept as `(Ctor payload)` / `(Ctor unit)`.
fn diagnostic_value(b: &mut ValueBuilder, d: &Diagnostic) -> value::ValueId {
    let severity = {
        let u = value::unit(b);
        let name = match d.severity {
            Severity::Error => "Error",
            Severity::Warning => "Warning",
        };
        value::bare_ctor(b, name, vec![u])
    };
    let code = option_str(b, d.code.as_deref());
    let message = value::str_leaf(b, &d.message);
    let node = option_uint(b, d.node.map(u64::from));
    let rec = value::record(
        b,
        vec![
            ("severity", severity),
            ("code", code),
            ("message", message),
            ("node", node),
        ],
    );
    // Ascription-free: decodes against `CompileDiagnostic` by shape (frame-tolerant decode).
    rec
}

fn read_diagnostic(arenas: &value::Arenas, id: value::ValueId) -> Option<Diagnostic> {
    let severity = match value::read_ctor(arenas, value::record_field(arenas, id, "severity")?)? {
        "Error" => Severity::Error,
        "Warning" => Severity::Warning,
        _ => return None,
    };
    Some(Diagnostic {
        severity,
        code: read_option_str(arenas, value::record_field(arenas, id, "code")?),
        message: value::read_str(arenas, value::record_field(arenas, id, "message")?)?,
        node: read_option_uint(arenas, value::record_field(arenas, id, "node")?)
            .and_then(|n| u32::try_from(n).ok()),
        fix: None,
    })
}

// --- Option helpers ((Some x) / (None unit)) -----------------------------------------------------------

fn option_str(b: &mut ValueBuilder, s: Option<&str>) -> value::ValueId {
    match s {
        Some(s) => {
            let leaf = value::str_leaf(b, s);
            value::bare_ctor(b, "Some", vec![leaf])
        }
        None => {
            let u = value::unit(b);
            value::bare_ctor(b, "None", vec![u])
        }
    }
}

fn option_uint(b: &mut ValueBuilder, n: Option<u64>) -> value::ValueId {
    match n {
        Some(n) => {
            let leaf = value::uint_leaf(b, n);
            value::bare_ctor(b, "Some", vec![leaf])
        }
        None => {
            let u = value::unit(b);
            value::bare_ctor(b, "None", vec![u])
        }
    }
}

fn read_option_str(arenas: &value::Arenas, id: value::ValueId) -> Option<String> {
    match value::read_ctor(arenas, id)? {
        "Some" => value::read_str(arenas, *value::ctor_payload(arenas, id)?.first()?),
        _ => None, // None (or malformed) -> no code
    }
}

fn read_option_uint(arenas: &value::Arenas, id: value::ValueId) -> Option<u64> {
    match value::read_ctor(arenas, id)? {
        "Some" => value::read_uint(arenas, *value::ctor_payload(arenas, id)?.first()?),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_request_round_trips() {
        let inputs = vec![
            Artifact::new(Artifact::KIND_AST, "main", vec![0x00, 0xde, 0xad]),
            Artifact::new("sidecar", "spans", b"span-table".to_vec()),
        ];
        assert_eq!(
            decode_compile_request(&encode_compile_request(&inputs)),
            inputs
        );
        assert!(decode_compile_request(b"not a value").is_empty());
    }

    #[test]
    fn compile_result_round_trips_artifacts_and_diagnostics() {
        let artifacts = vec![Artifact::new("component", "main", b"\0asm".to_vec())];
        let diagnostics = vec![
            Diagnostic {
                severity: Severity::Error,
                code: Some("CDZ0101".to_string()),
                message: "unbound name".to_string(),
                node: Some(7),
                fix: None,
            },
            Diagnostic {
                severity: Severity::Warning,
                code: None,
                message: "unused".to_string(),
                node: None,
                fix: None,
            },
        ];
        let (arts, diags) = decode_compile_result(&encode_compile_result(&artifacts, &diagnostics));
        assert_eq!(arts, artifacts);
        assert_eq!(diags, diagnostics);
    }

    #[test]
    fn garbage_result_degrades_to_empty() {
        let (arts, diags) = decode_compile_result(b"not a value form");
        assert!(arts.is_empty() && diags.is_empty());
    }
}
