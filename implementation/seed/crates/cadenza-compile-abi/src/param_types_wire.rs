//! The `KIND_PARAM_TYPES` map wire — each boundary export's COMPILED PARAM types as a FULL structured
//! `Ty` sub-AST, carried in ONE canonical BINARY AST value (`cadenza_ast::codec`), the SAME wire every
//! compile-boundary artifact speaks (operator seq-254/seq-284: "Binary AST everywhere. No exceptions." +
//! "I want the full type ast!"). The SIBLING of [`crate::result_types_wire`]: identical shape, differing
//! ONLY in the form-name tokens. Where the result wire carries one result `Ty` per export, this carries the
//! export's PARAM types packed as a single `(Tuple <param-ty>…)` payload (an empty `(Tuple)` for a nullary
//! export), in signature order — so the consumer decodes ONE `Ty` per export and reads its tuple elements as
//! the ordered param types.
//!
//! Why: `cdz-run` coerces a `--arg` literal against the export's param types obtained from wasmtime
//! `make.params()` introspection — the ERASED component types (`list<u8>` for a value-form leaf), which
//! cannot distinguish a `BigInt`/`Rational`/`Symbol` value-form param from a genuine `list<u8>`. The
//! `cdz-param-type` custom section (the run-wiring twin of `cdz-result-type`) carries the un-erased Cadenza
//! param `Ty`s so the arg-decode can pick the value-form decode for a BigInt param rather than declining.
//!
//! This crate is a GENERIC, full-fidelity codec — it performs NO render-specific projection (see
//! [`crate::result_types_wire`] for the structured-is-truth rationale). Shape: a root
//! `(param-types <param-type>…)` list, one `(param-type <Str name> <ty-payload>)` form per boundary export,
//! in export order; `<ty-payload>` is the `(Tuple …)` of param types grafted verbatim. TOTAL on decode: a
//! malformed / wrong-shape form is skipped, never a crash.

use cadenza_ast::ast::Arenas;

/// Encode the export→param-types map as the `KIND_PARAM_TYPES` artifact / `cdz-param-type` section bytes —
/// ONE canonical binary AST value (see module docs). Each entry's `Arenas` is a standalone arena ROOTED at
/// that export's `(Tuple <param-ty>…)` payload (as the `rcdzc` producer extracts it via `encode_ty_payload`
/// over a synthetic `Ty::Tuple` of the params); its root subtree is grafted verbatim into the shared
/// response arena. Order is preserved. Round-trips with [`decode_param_types`].
pub fn encode_param_types(entries: &[(String, Arenas)]) -> Vec<u8> {
    crate::ty_map_wire::encode("param-type", "param-types", entries)
}

/// Decode the `KIND_PARAM_TYPES` bytes, DISTINGUISHING a legitimately-ABSENT section from a PRESENT-but-
/// MALFORMED one — the decode-validity contract (operator directive): a decode failure on a present section
/// means the compiler emitted a malformed/mismatched AST, a BUG the caller must fail LOUD on, never silently
/// degrade. Contract:
///   * EMPTY `bytes` → `Ok(vec![])` — no param-types section was emitted (a legitimately param-typeless build).
///   * NON-EMPTY `bytes` whose `codec::decode` fails → `Err` (malformed binary AST — a compiler bug).
///   * decoded but ROOT is not a `param-types` form → `Err` (present-but-wrong-shape — a compiler bug).
///   * a valid `param-types` root → `Ok(entries)` (a wrong-shape *inner* `param-type` form is still skipped).
///
/// See the lenient [`decode_param_types`].
pub fn decode_param_types_checked(bytes: &[u8]) -> Result<Vec<(String, Arenas)>, String> {
    crate::ty_map_wire::decode_checked("param-type", "param-types", bytes)
}

/// Decode the `KIND_PARAM_TYPES` bytes back into export name → standalone param-types-tuple arena pairs — the
/// inverse of [`encode_param_types`], read via the shared `cadenza_ast::codec`. Each returned `Arenas` is a
/// fresh standalone arena whose ROOT is that export's `(Tuple <param-ty>…)` subtree. LENIENT/TOTAL: a
/// malformed section yields no entries. A consumer that must tell PRESENT-but-MALFORMED from
/// legitimately-ABSENT uses [`decode_param_types_checked`] instead.
pub fn decode_param_types(bytes: &[u8]) -> Vec<(String, Arenas)> {
    decode_param_types_checked(bytes).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cadenza_ast::ast::{Builder, Leaf, StructId};

    /// A standalone type-payload arena rooted at `root_build`'s node — as the producer extracts one export's
    /// `encode_ty_payload` subtree (here a `(Tuple …)` of the param types).
    fn ty(root_build: impl FnOnce(&mut Builder) -> StructId) -> Arenas {
        let mut b = Builder::new();
        let root = root_build(&mut b);
        b.finish(root)
    }

    #[test]
    fn param_types_tuple_payload_round_trips() {
        // Each export's params pack as a `(Tuple <param-ty>…)`: `f` takes `(Tuple BigInt)`, `g` takes
        // `(Tuple Int64 Symbol)`, `nullary` takes the empty `(Tuple)`. Full fidelity, no projection.
        let one_bigint = || {
            ty(|b| {
                let head = b.name("Tuple");
                let p = b.name("BigInt");
                b.list(vec![head, p])
            })
        };
        let two = || {
            ty(|b| {
                let head = b.name("Tuple");
                let a = b.name("Int64");
                let s = b.name("Symbol");
                b.list(vec![head, a, s])
            })
        };
        let nullary = || {
            ty(|b| {
                let head = b.name("Tuple");
                b.list(vec![head])
            })
        };
        let entries = vec![
            ("f".to_string(), one_bigint()),
            ("g".to_string(), two()),
            ("nullary".to_string(), nullary()),
        ];
        let decoded = decode_param_types(&encode_param_types(&entries));
        assert_eq!(decoded.len(), 3);
        assert_eq!(decoded[0].0, "f");
        assert!(decoded[0].1.structurally_eq(&one_bigint()));
        assert_eq!(decoded[1].0, "g");
        assert!(decoded[1].1.structurally_eq(&two()));
        assert_eq!(decoded[2].0, "nullary");
        assert!(decoded[2].1.structurally_eq(&nullary()));
    }

    #[test]
    fn empty_and_garbage_are_total() {
        assert!(decode_param_types(&encode_param_types(&[])).is_empty());
        assert!(decode_param_types(b"not a binary-ast tree").is_empty());
    }

    #[test]
    fn checked_distinguishes_absent_malformed_and_valid() {
        // ABSENT: no section → Ok(empty), NOT an error.
        assert_eq!(decode_param_types_checked(&[]), Ok(Vec::new()));
        // MALFORMED (non-empty garbage codec::decode rejects) → Err, NOT a silent empty.
        let garbage = b"not a binary AST at all".to_vec();
        assert!(
            cadenza_ast::codec::decode(&garbage).is_none(),
            "precondition: garbage doesn't decode"
        );
        assert!(decode_param_types_checked(&garbage).is_err());
        // VALID → Ok(entries).
        let mut b = Builder::new();
        let head = b.name("param-types");
        let f_head = b.name("param-type");
        let f_name = b.atom_leaf(Leaf::Str("f".into()));
        let tup = b.name("Tuple");
        let p = b.name("BigInt");
        let f_ty = b.list(vec![tup, p]);
        let f = b.list(vec![f_head, f_name, f_ty]);
        let root = b.list(vec![head, f]);
        let bytes = cadenza_ast::codec::encode(&b.finish(root));
        let decoded = decode_param_types_checked(&bytes).expect("valid section decodes");
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].0, "f");
    }
}
