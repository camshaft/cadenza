//! GAP-1 tool-calling transport — the JSON⟷`Document` boundary (O4). Behind `live-net`.
//!
//! The kernel carries a tool-call's input / a tool's inputSchema / a tool-result as OPAQUE JSON BYTES
//! ([`crate::converse`]'s intermediates); Bedrock's `Converse` API expresses those as an
//! [`aws_smithy_types::Document`] (its dynamic JSON value). Per the operator standing-order (host owns the
//! JSON⟷Bedrock mapping, O4), THIS is where that translation lives — a pure, total pair of functions the
//! Bedrock transport calls. Kept in its own module so it's unit-testable without a live Bedrock call (the
//! conversion is pure; only the surrounding `converse()` call needs the network).
//!
//! Totality: `json_bytes_to_document` returns `Err` on non-JSON bytes (a malformed tool input the reducer
//! built — the transport folds it PERMANENT, never a panic). `document_to_json_bytes` is infallible
//! (`Document` is always representable as JSON — it IS a JSON value).

use aws_smithy_types::{Document, Number};

/// Parse opaque JSON bytes (a tool-call `input` / a tool `schema`) into a Bedrock [`Document`]. `Err` on
/// bytes that aren't valid JSON — a structural problem with the reducer's request (folded PERMANENT upstream).
pub fn json_bytes_to_document(bytes: &[u8]) -> Result<Document, String> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|e| format!("tool payload was not valid JSON: {e}"))?;
    Ok(value_to_document(&value))
}

/// Serialize a Bedrock [`Document`] (a `Converse` tool-use `input` / tool-result content) back to JSON bytes
/// for the kernel's opaque `input`/`result` payload. Infallible — a `Document` is always a JSON value.
pub fn document_to_json_bytes(doc: &Document) -> Vec<u8> {
    // `Document` maps 1:1 to serde_json::Value, which always serializes.
    serde_json::to_vec(&document_to_value(doc)).unwrap_or_else(|_| b"null".to_vec())
}

/// `serde_json::Value` → `Document` (recursive, total). A JSON number becomes the tightest `Number` variant:
/// a non-negative integer → `PosInt`, a negative integer → `NegInt`, anything else → `Float`.
fn value_to_document(v: &serde_json::Value) -> Document {
    match v {
        serde_json::Value::Null => Document::Null,
        serde_json::Value::Bool(b) => Document::Bool(*b),
        serde_json::Value::String(s) => Document::String(s.clone()),
        serde_json::Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                Document::Number(Number::PosInt(u))
            } else if let Some(i) = n.as_i64() {
                Document::Number(Number::NegInt(i))
            } else {
                // as_f64 is Some for any JSON number that isn't a u64/i64 (or is fractional). A NaN/inf can't
                // arise from parsed JSON, so the fallback 0.0 is unreachable in practice.
                Document::Number(Number::Float(n.as_f64().unwrap_or(0.0)))
            }
        }
        serde_json::Value::Array(a) => Document::Array(a.iter().map(value_to_document).collect()),
        serde_json::Value::Object(o) => Document::Object(
            o.iter()
                .map(|(k, v)| (k.clone(), value_to_document(v)))
                .collect(),
        ),
    }
}

/// `Document` → `serde_json::Value` (recursive, total). The inverse of [`value_to_document`]; every `Number`
/// variant maps back to a JSON number.
fn document_to_value(d: &Document) -> serde_json::Value {
    match d {
        Document::Null => serde_json::Value::Null,
        Document::Bool(b) => serde_json::Value::Bool(*b),
        Document::String(s) => serde_json::Value::String(s.clone()),
        Document::Number(n) => match n {
            Number::PosInt(u) => serde_json::Value::from(*u),
            Number::NegInt(i) => serde_json::Value::from(*i),
            Number::Float(f) => serde_json::Number::from_f64(*f)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
        },
        Document::Array(a) => serde_json::Value::Array(a.iter().map(document_to_value).collect()),
        Document::Object(o) => serde_json::Value::Object(
            o.iter()
                .map(|(k, v)| (k.clone(), document_to_value(v)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_nested_tool_input() {
        // A realistic tool-call input (shell command args): object with string, array, int, bool, null.
        let json =
            br#"{"cmd":"cargo test","args":["--lib","-q"],"retries":3,"verbose":true,"note":null}"#;
        let doc = json_bytes_to_document(json).expect("valid JSON → Document");
        // Round-trip: Document → JSON bytes → Value equals the original parsed Value (key order aside).
        let back = document_to_json_bytes(&doc);
        let orig: serde_json::Value = serde_json::from_slice(json).unwrap();
        let round: serde_json::Value = serde_json::from_slice(&back).unwrap();
        assert_eq!(round, orig, "JSON⟷Document round-trips a nested tool input");
    }

    #[test]
    fn maps_number_variants_tightly() {
        let doc = json_bytes_to_document(br#"{"pos":7,"neg":-4,"flt":1.5}"#).unwrap();
        let Document::Object(o) = &doc else {
            panic!("expected object, got {doc:?}")
        };
        assert!(matches!(
            o.get("pos"),
            Some(Document::Number(Number::PosInt(7)))
        ));
        assert!(matches!(
            o.get("neg"),
            Some(Document::Number(Number::NegInt(-4)))
        ));
        assert!(matches!(
            o.get("flt"),
            Some(Document::Number(Number::Float(_)))
        ));
    }

    #[test]
    fn non_json_bytes_are_a_clean_error_not_a_panic() {
        let err = json_bytes_to_document(b"not json {{{").unwrap_err();
        assert!(err.contains("not valid JSON"), "{err}");
    }

    #[test]
    fn empty_object_and_scalars_round_trip() {
        for src in [&b"{}"[..], b"[]", b"\"hi\"", b"42", b"true", b"null"] {
            let doc = json_bytes_to_document(src).expect("valid JSON");
            let back = document_to_json_bytes(&doc);
            let a: serde_json::Value = serde_json::from_slice(src).unwrap();
            let b: serde_json::Value = serde_json::from_slice(&back).unwrap();
            assert_eq!(
                a,
                b,
                "scalar/empty round-trips: {:?}",
                std::str::from_utf8(src)
            );
        }
    }
}
