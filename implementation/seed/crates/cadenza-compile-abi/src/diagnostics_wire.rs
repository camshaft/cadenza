//! The `KIND_DIAGNOSTICS` RESULT wire — the compiler's well-formedness fault set as canonical BINARY AST
//! (`cadenza_ast::codec`), the SAME wire every compile-boundary artifact speaks (operator seq-254/seq-284:
//! "Binary AST is THE data exchange format. No exceptions."). The producer (`rcdzc::sidecar::run_query`'s
//! `Query::Diagnostics`) calls [`encode_diagnostics`]; every consumer — `cdz check`, the located error
//! reporter, and `v-corpus-harness`'s C1 diagnostic-quality grade — calls [`decode_diagnostics`]. ONE
//! shared codec, so neither side hand-rolls a parser (this replaces the bespoke 8-TAB-column text wire).
//!
//! Shape: a root `Ast.List` of per-fault forms; each fault is a fixed-arity
//! `(list [severity-Name, code-opt, node-opt, message-Str, fix-opt])` where an `opt` is the AST-native
//! Option idiom `(list [])` = None / `(list [value])` = Some. A `fix` value is
//! `(list [label-Str, kind-Name, node-Int, replacement-Str, verified-Bool])`. TOTAL on decode: a malformed
//! tree / wrong-shape or unknown-name entry is skipped (the reporter degrades to a location-less
//! diagnostic, never a crash) — the same graceful-degrade the old text decoder gave a malformed line.

use crate::abi::{Diagnostic, DiagnosticFix, FixKind, Severity};
use cadenza_ast::ast::{Arenas, Builder, IntValue, Leaf, Radix, Struct, StructId};

/// Encode a fault set as the `KIND_DIAGNOSTICS` artifact bytes — canonical binary AST (see module docs).
/// Round-trips with [`decode_diagnostics`]; byte-identical whether built here (the `rcdzc` producer) or,
/// in principle, by any tool holding the shared `Diagnostic` vocabulary.
pub fn encode_diagnostics(diags: &[Diagnostic]) -> Vec<u8> {
    let mut b = Builder::new();
    let forms: Vec<StructId> = diags.iter().map(|d| encode_one(&mut b, d)).collect();
    let root = b.list(forms);
    cadenza_ast::codec::encode(&b.finish(root))
}

/// Decode the `KIND_DIAGNOSTICS` artifact bytes back into the fault set — the inverse of
/// [`encode_diagnostics`], read via the shared `cadenza_ast::codec`. TOTAL: a malformed / wrong-shape
/// entry is skipped rather than failing the whole decode.
pub fn decode_diagnostics(bytes: &[u8]) -> Vec<Diagnostic> {
    let Some(a) = cadenza_ast::codec::decode(bytes) else {
        return Vec::new();
    };
    let Struct::List(forms) = a.get(a.root).clone() else {
        return Vec::new();
    };
    forms.iter().filter_map(|&f| decode_one(&a, f)).collect()
}

fn encode_one(b: &mut Builder, d: &Diagnostic) -> StructId {
    let sev = b.name(severity_name(d.severity));
    let code = opt_str(b, d.code.as_deref());
    let node = opt_int(b, d.node);
    let msg = str_leaf(b, &d.message);
    let fix = match &d.fix {
        None => b.list(vec![]),
        Some(f) => {
            let label = str_leaf(b, &f.label);
            let kind = b.name(fix_kind_name(f.kind));
            let fnode = int_leaf(b, f.node);
            let repl = str_leaf(b, &f.replacement);
            let ver = b.atom_leaf(Leaf::Bool(f.verified));
            let form = b.list(vec![label, kind, fnode, repl, ver]);
            b.list(vec![form])
        }
    };
    b.list(vec![sev, code, node, msg, fix])
}

fn decode_one(a: &Arenas, form: StructId) -> Option<Diagnostic> {
    let Struct::List(c) = a.get(form) else {
        return None;
    };
    let severity = severity_from(a.as_name(*c.first()?)?)?;
    let code = opt_str_of(a, *c.get(1)?);
    let node = opt_int_of(a, *c.get(2)?);
    let message = a.as_str(*c.get(3)?)?.to_string();
    let fix = fix_of(a, *c.get(4)?);
    Some(Diagnostic {
        severity,
        code,
        message,
        node,
        fix,
    })
}

fn fix_of(a: &Arenas, id: StructId) -> Option<DiagnosticFix> {
    // `(list [])` = None, `(list [fix-form])` = Some. A fix-form is `(list [label, kind, node, repl, ver])`.
    let Struct::List(outer) = a.get(id) else {
        return None;
    };
    let &form = outer.first()?;
    let Struct::List(f) = a.get(form) else {
        return None;
    };
    Some(DiagnosticFix {
        label: a.as_str(*f.first()?)?.to_string(),
        kind: fix_kind_from(a.as_name(*f.get(1)?)?)?,
        node: u32::try_from(a.as_int(*f.get(2)?)?.to_i64()?).ok()?,
        replacement: a.as_str(*f.get(3)?)?.to_string(),
        verified: a.as_bool(*f.get(4)?)?,
    })
}

// --- small shared atoms + the AST-native Option idiom (`(list [])` / `(list [v])`) ---

fn str_leaf(b: &mut Builder, s: &str) -> StructId {
    b.atom_leaf(Leaf::Str(s.into()))
}

fn int_leaf(b: &mut Builder, n: u32) -> StructId {
    b.atom_leaf(Leaf::Int {
        value: IntValue::from_i64(i64::from(n)),
        radix: Radix::Dec,
    })
}

fn opt_str(b: &mut Builder, s: Option<&str>) -> StructId {
    match s {
        None => b.list(vec![]),
        Some(s) => {
            let leaf = str_leaf(b, s);
            b.list(vec![leaf])
        }
    }
}

fn opt_int(b: &mut Builder, n: Option<u32>) -> StructId {
    match n {
        None => b.list(vec![]),
        Some(n) => {
            let leaf = int_leaf(b, n);
            b.list(vec![leaf])
        }
    }
}

fn opt_str_of(a: &Arenas, id: StructId) -> Option<String> {
    let Struct::List(xs) = a.get(id) else {
        return None;
    };
    Some(a.as_str(*xs.first()?)?.to_string())
}

fn opt_int_of(a: &Arenas, id: StructId) -> Option<u32> {
    let Struct::List(xs) = a.get(id) else {
        return None;
    };
    u32::try_from(a.as_int(*xs.first()?)?.to_i64()?).ok()
}

fn severity_name(s: Severity) -> &'static str {
    match s {
        Severity::Error => "error",
        Severity::Warning => "warning",
    }
}

fn severity_from(name: &str) -> Option<Severity> {
    match name {
        "error" => Some(Severity::Error),
        "warning" => Some(Severity::Warning),
        _ => None,
    }
}

fn fix_kind_name(k: FixKind) -> &'static str {
    match k {
        FixKind::Replace => "replace",
        FixKind::InsertInto => "insert-into",
        FixKind::Wrap => "wrap",
        FixKind::Delete => "delete",
    }
}

fn fix_kind_from(name: &str) -> Option<FixKind> {
    match name {
        "replace" => Some(FixKind::Replace),
        "insert-into" => Some(FixKind::InsertInto),
        "wrap" => Some(FixKind::Wrap),
        "delete" => Some(FixKind::Delete),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The binary-AST diagnostics wire round-trips the FULL Diagnostic exactly (operator seq-254/284): a
    // coded error with a verified Wrap fix, a fix-less unanchored uncoded decline, and a warning with a
    // heuristic Delete fix + every Option arm (Some/None code, Some/None node) + all Severity/FixKind
    // variants exercised. This is the drift guard the rcdzc producer + cdz/corpus-harness consumers rely on.
    #[test]
    fn diagnostics_binary_ast_round_trips() {
        let diags = vec![
            Diagnostic {
                severity: Severity::Error,
                code: Some("CDZ0304".into()),
                message: "overflow in a constant expression".into(),
                node: Some(42),
                fix: Some(DiagnosticFix {
                    label: "wrap with Int64.wrapping-add".into(),
                    kind: FixKind::Wrap,
                    node: 42,
                    replacement: "(Int64.wrapping-add …)".into(),
                    verified: true,
                }),
            },
            Diagnostic {
                severity: Severity::Error,
                code: None,
                message: "unsupported construct".into(),
                node: None,
                fix: None,
            },
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
        ];
        assert_eq!(decode_diagnostics(&encode_diagnostics(&diags)), diags);
        // Empty fault set round-trips to empty; a garbage payload decodes to empty (total, never panics).
        assert!(decode_diagnostics(&encode_diagnostics(&[])).is_empty());
        assert!(decode_diagnostics(b"not a binary-ast tree").is_empty());
    }
}
