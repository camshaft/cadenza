//! `compile` — the pure compilation entry: kinded artifacts + targets in, {artifacts, diagnostics}
//! out. NO I/O (no filesystem, no args) — that is the CLI bin's job. This is the part that ports to
//! the Cadenza self-host; the `Target` enum and the demand-driven `Db` are also the substrate the
//! eventual query-program-driven entry will reuse.
//!
//! It orchestrates the query engine for each requested target: decode the `ast` input into a [`Db`],
//! compute the boundary [`Layout`], collect every reached fault (a compile-provable poison, a type
//! mismatch) across the reachable definitions, and — only if the program is clean — ask each target's
//! backend to fill its artifact. A fault means NO artifact and one error diagnostic per fault
//! (decline-don't-miscompile: a construct the compiler cannot compile correctly declines rather than
//! emitting a wrong or trapping component — `reference-compiler.md` §Outcomes Are Ordered By Safety).
//!
//! Poison collection descends only into positions a value is UNCONDITIONALLY used — it does not
//! descend into a conditional's branches — so a fault shielded by an untaken branch stays a runtime
//! trap rather than failing the build (`reference-compiler.md` §Reachability Is A Consequence Of
//! Reduction). Stage 0's slice reaches poisons only at a definition body root or an `if` condition.

use crate::abi::{Artifact, CompileOutput, Diagnostic, Severity};
use crate::ast::StructId;
use crate::backend::{self, Target};
use crate::core::Core;
use crate::db::Db;
use crate::diag::Reject;
use crate::infer::type_errors;
use crate::layout::{self, Layout};
use crate::lower::core_of;

/// Compile a set of kinded input artifacts to the requested targets. The `ast` input is decoded into
/// a `Db`; each target's backend fills its artifact from the shared columns. Targets default to
/// `[Wasm]` at the CLI, not here — this entry emits exactly what it is asked for.
pub fn compile(inputs: &[Artifact], targets: &[Target]) -> CompileOutput {
    // Select the `ast` input artifact and decode it.
    let ast_art = inputs.iter().find(|a| a.kind == Artifact::KIND_AST);
    let ast_bytes = match ast_art {
        Some(a) => &a.bytes,
        None => return fail(vec![Reject::decline("no `ast` input artifact")]),
    };
    let arenas = match crate::codec::decode(ast_bytes) {
        Some(a) => a,
        None => return fail(vec![Reject::decline("binary AST failed to decode")]),
    };

    let mut db = Db::load(arenas);

    // Compute the boundary layout once (target-neutral). A program with no export declines.
    let layout = match layout::compute(&mut db) {
        Ok(l) => l,
        Err(r) => return fail(vec![r]),
    };

    // Collect every reached fault across the reachable definitions, module-wide (report ALL, not just
    // the first — `compiler-pipeline.md` §Phases Recover From Errors).
    let faults = collect_faults(&mut db, &layout);
    if !faults.is_empty() {
        return fail(faults);
    }

    // Clean: ask each requested target's backend to fill its artifact.
    let mut artifacts = Vec::new();
    let mut diagnostics = Vec::new();
    for &target in targets {
        match backend::emit(target, &mut db, &layout) {
            Ok(bytes) => artifacts.push(Artifact::new(target.artifact_kind(), program_name(&db), bytes)),
            Err(r) => diagnostics.push(Diagnostic::from_reject(&r)),
        }
    }
    CompileOutput { artifacts, diagnostics }
}

/// A convenience over [`compile`]: a lone canonical-AST byte string → the WebAssembly component bytes,
/// or the first error diagnostic. What the tests and simple callers use.
pub fn compile_component(ast_bytes: &[u8]) -> Result<Vec<u8>, Diagnostic> {
    let out = compile(
        &[Artifact::new(Artifact::KIND_AST, "main", ast_bytes.to_vec())],
        &[Target::Wasm],
    );
    match out.artifact(Target::Wasm.artifact_kind()) {
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

/// Every fault across the program's definitions. Well-formedness — scope resolution and type
/// checking — is UNCONDITIONAL: it holds over EVERY top-level definition's body, not only the ones
/// reachable from an export, because a program is well-formed or not regardless of what is asked to
/// compile (`core-semantics.md` §Binding Is Lexical — the unbound-name rule is not gated on
/// reachability; an ill-formed uncalled sibling definition is still rejected). Emission stays
/// reachability-driven (the `layout` decides what is emitted); only well-formedness is total.
fn collect_faults(db: &mut Db, _layout: &Layout) -> Vec<Reject> {
    let mut faults = Vec::new();
    // Check EVERY definition's body — reachable or not. (The demand is still lazy per node; this just
    // demands each definition once, which is what well-formedness requires.)
    let bodies: Vec<(StructId, bool)> = db
        .defs
        .iter()
        .filter_map(|d| d.body.map(|b| (b, d.params.is_empty())))
        .collect();
    for (body, nullary) in bodies {
        // Scope + type checking (`type_errors`) applies to EVERY body — a function body's free
        // parameters are bound (a `Param` types fine), so an unbound name or type fault in it is still
        // caught. The reached-POISON walk lowers the body, which only makes sense for a VALUE: a
        // FUNCTION body (a def with parameters) is not lowered standalone — its params are
        // unsubstituted until it is applied — so run the trap walk only on a nullary def's body (a
        // value). A function body's traps surface when it is applied and its call site is lowered.
        if nullary {
            collect_reached_poisons(db, body, &mut faults);
        }
        faults.extend(type_errors(db, body));
    }
    faults
}

/// Collect poisons reached UNCONDITIONALLY from `id`. Descends the core form into positions a value is
/// unconditionally used (an `if` CONDITION), but NOT into a conditional's branches — a poison shielded
/// by an untaken branch is not a build failure. Reads the core column on demand.
fn collect_reached_poisons(db: &mut Db, id: StructId, out: &mut Vec<Reject>) {
    match core_of(db, id) {
        Core::Poison(r) => out.push(r),
        Core::If { cond, .. } => {
            // The condition is unconditionally evaluated; the branches are not (they are guarded).
            collect_reached_poisons(db, cond, out);
        }
        // A record's field values are all unconditionally part of the value — descend into each. (A
        // record used only to read a field folds away before reaching here; one that survives is a
        // runtime value whose fields are all reached.)
        Core::Record { fields } => {
            for (_, value) in fields {
                collect_reached_poisons(db, value, out);
            }
        }
        // Both operands of a runtime arithmetic op are unconditionally evaluated — descend into each.
        Core::Arith { lhs, rhs, .. } => {
            collect_reached_poisons(db, lhs, out);
            collect_reached_poisons(db, rhs, out);
        }
        Core::ConstInt(_) | Core::ConstBool(_) | Core::Unit => {}
    }
}

/// The program's name for artifact labelling — the first exported name, or "main". (A cosmetic label;
/// the artifact's identity is its kind + bytes.)
fn program_name(db: &Db) -> String {
    db.exports.first().map(|e| e.name.clone()).unwrap_or_else(|| "main".to_string())
}

/// A failed compilation: no artifacts, one error diagnostic per reject.
fn fail(rejects: Vec<Reject>) -> CompileOutput {
    CompileOutput {
        artifacts: Vec::new(),
        diagnostics: rejects.iter().map(Diagnostic::from_reject).collect(),
    }
}
