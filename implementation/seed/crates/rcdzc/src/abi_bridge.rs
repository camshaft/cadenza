//! ABI-projection bridges — convert compiler-internal diagnostic values (`crate::diag::{Reject, Fix,
//! Code}`) into the plain `cadenza-compile-abi` boundary types (`Diagnostic`/`DiagnosticFix`). These
//! conversions couple to rcdzc-internal `diag` types, so — now that the boundary types live in the
//! shared crate — the orphan rule requires them as FREE FUNCTIONS here rather than inherent impls on
//! the moved types (the E3b-diag extraction; v-cdz-crate-split + v-inference). Only `rcdzc` PRODUCES a
//! `Diagnostic` (via these); `cdz` never converts — it only READS the boundary `Diagnostic`'s fields.

use crate::ast::StructId;
use cadenza_compile_abi::{Diagnostic, DiagnosticFix, FixKind, Severity, WRAP_HOLE};

/// The ABI projection of a compiler-internal [`crate::diag::Fix`] — lower its structural edit to a
/// `(kind, node, replacement)` triple and its applicability to the `verified` flag. (Was
/// `DiagnosticFix::from_fix`, moved here as a free fn since `DiagnosticFix` is now a shared-crate type.)
pub fn diagnostic_fix_from_fix(fix: &crate::diag::Fix) -> DiagnosticFix {
    let (kind, node, replacement) = match &fix.edit {
        crate::diag::Edit::ReplaceNode { at, replacement } => {
            (FixKind::Replace, at.0, replacement.clone())
        }
        crate::diag::Edit::InsertArms { at, arms } => (FixKind::InsertInto, at.0, arms.join(" ")),
        crate::diag::Edit::Wrap { at, prefix, suffix } => {
            (FixKind::Wrap, at.0, format!("{prefix}{WRAP_HOLE}{suffix}"))
        }
        crate::diag::Edit::Delete { at } => (FixKind::Delete, at.0, String::new()),
    };
    DiagnosticFix {
        label: fix.label.clone(),
        kind,
        node,
        replacement,
        verified: matches!(fix.applicability, crate::diag::Applicability::Verified),
    }
}

/// Build an error [`Diagnostic`] from a produced "no" (a reject carries a code; a decline does not),
/// carrying its origin node index for the consumer to resolve to a span, and its proposed fix. (Was
/// `Diagnostic::from_reject`.)
pub fn diagnostic_from_reject(reject: &crate::diag::Reject) -> Diagnostic {
    Diagnostic {
        severity: Severity::Error,
        code: reject.code.map(|c| c.code().to_string()),
        message: reject.message.clone(),
        node: reject.at.map(|id| id.0),
        fix: reject.fix.as_ref().map(|f| diagnostic_fix_from_fix(f)),
    }
}

/// Build a WARNING [`Diagnostic`] — a non-error that rides alongside a produced artifact (it does not
/// deny the component; `has_error` ignores it). (Was `Diagnostic::warning`; a free fn since it couples
/// to the rcdzc-internal `diag::Code`.)
pub fn diagnostic_warning(
    code: crate::diag::Code,
    message: impl Into<String>,
    node: Option<StructId>,
) -> Diagnostic {
    Diagnostic {
        severity: Severity::Warning,
        code: Some(code.code().to_string()),
        message: message.into(),
        node: node.map(|id| id.0),
        fix: None,
    }
}
