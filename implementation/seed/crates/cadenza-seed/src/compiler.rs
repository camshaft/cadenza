//! Compiler selection — the old `cdz-compiler` (default, the byte ORACLE) or the rewritten `rcdzc`
//! (opt-in via `CADENZA_COMPILER=v2`).
//!
//! `rcdzc` is the from-scratch spec-shaped rewrite (Ast→Hir→Mir→Lir, nanopass, real HM). It is being
//! built up beside the old compiler; until it reaches parity the seed SHIPS the old one and uses it
//! as the differential byte oracle. This shim lets every CLI/gate call site pick the compiler by one
//! env var.
//!
//! The COMMON CURRENCY is the kinded-artifact `CompileOutput` — a produced component (if any) plus an
//! **always-live diagnostics list** (`build-tool-interface.md`, Amendment 0.8.0; and
//! `compiler-pipeline.md` §Phases Recover From Errors: "report ALL diagnostics rather than stop at
//! the first"). Both compilers speak it: `rcdzc` natively; the old compiler's single `Decline` maps to
//! a one-element list (it does not yet recover past the first error). Multi-diagnostic OUTPUT arrives
//! as `rcdzc`'s passes gain error recovery across independent sites (Phase 1+); the surface is multi
//! from the start so nothing is retrofitted. A byte-only `Result<Vec<u8>, Decline>` projection remains
//! for the differential/ignition call sites that compare bytes and do not display diagnostics.

use cdz_compiler::ast::Node;
use cdz_compiler::codegen::{self, Decline};
use rcdzc::{Artifact, CompileOutput, Diagnostic, Severity};

/// Is the rewritten compiler selected? `CADENZA_COMPILER=v2` (any other value / unset → the old one).
pub fn use_v2() -> bool {
    std::env::var("CADENZA_COMPILER").as_deref() == Ok("v2")
}

/// Compile a parsed program `Node`, dispatching to the selected compiler, and return the full
/// kinded-artifact output — the produced component (if any) plus the complete diagnostics list. This
/// is the primary entry; the human-facing CLI reports EVERY diagnostic from it.
pub fn compile(node: &Node) -> CompileOutput {
    if use_v2() {
        rcdzc::compile_program(node)
    } else {
        // The old compiler is single-result; lift its `Result<Vec<u8>, Decline>` into the common
        // multi-diagnostic shape (a component artifact, or a one-element error-diagnostic list).
        match codegen::compile_program(node) {
            Ok(bytes) => CompileOutput {
                artifacts: vec![Artifact::new(Artifact::KIND_COMPONENT, bytes)],
                diagnostics: Vec::new(),
            },
            Err(d) => CompileOutput {
                artifacts: Vec::new(),
                diagnostics: vec![Diagnostic {
                    severity: Severity::Error,
                    code: d.code().map(|c| c.to_string()),
                    message: d.message().to_string(),
                }],
            },
        }
    }
}

/// The byte-only projection for call sites that compare bytes (the corpus differential, ignition,
/// probes) and do not display diagnostics: the component bytes, or the first error diagnostic
/// collapsed to a `Decline` (preserving its code so a coded reject still reads as one).
pub fn compile_program(node: &Node) -> Result<Vec<u8>, Decline> {
    let out = compile(node);
    match out.component() {
        Some(bytes) => Ok(bytes.to_vec()),
        None => {
            let diag = out.diagnostics.into_iter().find(|d| d.severity == Severity::Error);
            match diag {
                Some(d) => Err(Decline(d.message, d.code)),
                None => Err(Decline("compiler produced no component".into(), None)),
            }
        }
    }
}
