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

use rcdzc::ast::Node;
use rcdzc::{Artifact, CompileOutput, Diagnostic, Severity};

/// Compile a parsed program `Node`, dispatching to the selected compiler, and return the full
/// kinded-artifact output — the produced component (if any) plus the complete diagnostics list. This
/// is the primary entry; the human-facing CLI reports EVERY diagnostic from it.
pub fn compile(node: &Node) -> CompileOutput {
    rcdzc::compile_program(node)
}

/// The byte-only projection for call sites that compare bytes (the corpus differential, ignition,
/// probes) and do not display diagnostics: the component bytes, or the first error diagnostic
/// collapsed to a `Decline` (preserving its code so a coded reject still reads as one).
pub fn compile_program(node: &Node) -> Result<Vec<u8>, Decline> {
    let out = compile(node);
    match out.component() {
        Some(bytes) => Ok(bytes.to_vec()),
        None => {
            let diag = out
                .diagnostics
                .into_iter()
                .find(|d| d.severity == Severity::Error);
            match diag {
                Some(d) => Err(Decline(d.message, d.code)),
                None => Err(Decline("compiler produced no component".into(), None)),
            }
        }
    }
}
