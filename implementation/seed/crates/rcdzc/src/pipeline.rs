//! The pipeline — threads the nanopass rungs and speaks the kinded-artifact ABI.
//!
//! `bytes ─decode─▶ Ast ─resolve─▶ Hir ─infer─▶ typed-Hir ─lower─▶ Mir ─eval─▶ Mir ─select─▶ Lir
//!  ─serialize─▶ bytes`. Each arrow is one pass in its own module, applied module-wide (a module is a
//! set of functions); this file wires them and packages the result as the artifacts-in /
//! {artifacts, diagnostics}-out ABI.

use crate::abi::{Artifact, CompileOutput, Diagnostic, Severity};
use crate::ast::{self, Node};
use crate::ir::Reject;
use crate::layout::Layout;
use crate::{infer, lower, resolve, select, serialize};

/// Compile a single already-parsed program `Node` to its `CompileOutput` — a `component` artifact on
/// success, or one or more error diagnostics (no component) on a decline/reject.
pub fn compile_program(node: &Node) -> CompileOutput {
    match compile_inner(node) {
        Ok(bytes) => CompileOutput {
            artifacts: vec![Artifact::new(Artifact::KIND_COMPONENT, bytes)],
            diagnostics: Vec::new(),
        },
        Err(rejects) => failure(rejects),
    }
}

/// The pass pipeline. A pass that aborts on the first fault returns a one-element reject list; the
/// compile-time evaluator, which recovers, returns ALL its diagnostics (build-tool-interface.md,
/// Amendment 0.8.0; compiler-pipeline.md §Phases Recover From Errors — "report ALL diagnostics rather
/// than stop at the first").
fn compile_inner(node: &Node) -> Result<Vec<u8>, Vec<Reject>> {
    let hir = resolve::resolve_program(node).map_err(one)?;
    let typed = infer::infer_module(hir).map_err(one)?;
    let mir = lower::lower_module(typed);

    // The compile-time evaluator (fold) turns a constant operation that would trap — an overflow, a
    // divide-by-zero — into a POISON node, DROPPING any in a branch it proved unreachable. A poison
    // that SURVIVES to an unconditionally-reached position is a compile-time diagnostic: the compiler
    // fails compilation rather than shipping a component that traps at run time (the operator's ruling;
    // core-semantics.md's compile-time-knowable-error precedent). Collect EVERY such poison, module-wide.
    let mut poisons: Vec<Reject> = Vec::new();
    for f in &mir.funcs {
        let mut found = Vec::new();
        crate::fold::collect_reached_poisons(&f.body, &mut found);
        poisons.extend(found.into_iter().cloned());
    }
    if !poisons.is_empty() {
        return Err(poisons);
    }

    // ERASURE FENCE (Layer 1 first-class types): a compile-time-only value (a type-value, or a
    // compound containing one) that survived fold must never reach the runtime boundary. Check every
    // function body post-fold: if a surviving Mir node's type `is_comptime_only`, reject with CDZ0305.
    // This is the structural check that catches a type-value smuggled inside a heap compound.
    for f in &mir.funcs {
        if let Some(reject) = crate::fold::check_erasure_fence(&f.body) {
            return Err(one(reject));
        }
    }

    // The layout fixes the whole boundary surface + every user function's absolute index BEFORE
    // selection, so `select` can resolve calls to concrete indices and `serialize` is pure byte-laying.
    let layout = Layout::of(&mir).map_err(|e| one(Reject::decline(e)))?;

    // A module that touches the value heap ANYWHERE — its result is a compound, OR it merely
    // constructs/projects one internally (e.g. `(. (tuple 7 8) 0)`, a scalar result built via the
    // heap) — takes the RUNTIME-COMPOUND path: it imports the heap runtime and renders its result to a
    // string in-program. A program that never touches the heap takes the scalar component. The layout
    // decides (`imports_runtime`, a function of the solved types), so this is a single branch.
    let entry_ret = mir.funcs[layout.order[0]].ret.clone();
    let selected = select::select_module(mir, &layout).map_err(|e| one(Reject::decline(e)))?;
    if layout.imports_runtime {
        serialize::runtime_compound_component(&selected, &layout, &entry_ret)
            .map_err(|e| one(Reject::decline(e)))
    } else {
        serialize::component(&selected, &layout).map_err(|e| one(Reject::decline(e)))
    }
}

/// A single reject as the one-element list the aborting passes produce.
fn one(reject: Reject) -> Vec<Reject> {
    vec![reject]
}

/// The general ABI entry: kinded artifacts in, {artifacts, diagnostics} out. Selects the `ast` input
/// artifact, decodes it, and compiles. A missing/undecodable `ast` artifact is an error diagnostic.
pub fn compile(inputs: &[Artifact]) -> CompileOutput {
    let ast_art = inputs.iter().find(|a| a.kind == Artifact::KIND_AST);
    let ast_bytes = match ast_art {
        Some(a) => &a.bytes,
        None => return failure(one(Reject::decline("no `ast` input artifact"))),
    };
    match ast::decode(ast_bytes) {
        Ok(node) => compile_program(&node),
        Err(e) => failure(one(Reject::decline(format!("binary AST decode: {}", e.0)))),
    }
}

/// The degenerate convenience entry: a lone canonical-AST byte string → the component bytes, or the
/// first error diagnostic. Derived from the general `compile`.
pub fn compile_bytes(ast_bytes: &[u8]) -> Result<Vec<u8>, Diagnostic> {
    let out = compile(&[Artifact::new(Artifact::KIND_AST, ast_bytes.to_vec())]);
    match out.component() {
        Some(bytes) => Ok(bytes.to_vec()),
        None => Err(out
            .diagnostics
            .into_iter()
            .find(|d| d.severity == Severity::Error)
            .unwrap_or(Diagnostic {
                severity: Severity::Error,
                code: None,
                message: "compilation produced no component".into(),
            })),
    }
}

/// A failed compilation: no artifacts, one error diagnostic per reject (the multi-diagnostic ABI —
/// every fault is reported, not just the first).
fn failure(rejects: Vec<Reject>) -> CompileOutput {
    CompileOutput {
        artifacts: Vec::new(),
        diagnostics: rejects
            .into_iter()
            .map(|reject| Diagnostic {
                severity: Severity::Error,
                // The stable `CDZ####` string comes from the single `Code` taxonomy (diag.rs).
                code: reject.code.map(|c| c.code().to_string()),
                message: reject.message,
            })
            .collect(),
    }
}
