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
//! Poison collection descends only into positions a value is UNCONDITIONALLY used — an `if`
//! condition, a `let` binding and body, arithmetic/comparison/conversion operands, a call's
//! arguments, a match scrutinee, and tuple/record/sum construction — but NOT a conditional's branches
//! or a match arm's body, so a fault shielded by an untaken branch stays a runtime trap rather than
//! failing the build (`reference-compiler.md` §Reachability Is A Consequence Of Reduction). These are
//! exactly the OBSERVED positions — a value flows to the result, a host call, or an operation that
//! inspects it — so a trap in an observed computation is not elided: its computation is evaluated (or,
//! for a compile-provable trap, faults the build here) rather than skipped.
//= spec/capabilities/core-semantics.md#a-trap-occurs-only-where-its-computation-is-observed
//# A trap MUST occur when the computation that would raise it is observed — when its value flows to the program's result, to a host call, or to an operation that inspects it (an arithmetic or comparison operand, an `if` condition, a match scrutinee, a projected tuple element or record field, a referenced binding, or an argument bound to a parameter the function body uses).
//= spec/capabilities/core-semantics.md#a-trap-occurs-only-where-its-computation-is-observed
//# A computation whose value is observed in this sense MUST be evaluated, so its trap MUST occur.

use crate::abi::{Artifact, CompileOutput, Diagnostic, Severity};
use crate::ast::StructId;
use crate::backend::{self, Target};
use crate::core::Core;
use crate::db::Db;
use crate::diag::{Code, Reject};
use crate::infer::type_errors;
use crate::layout;
use crate::link;
use crate::lower::core_of;
use crate::sidecar;
use crate::spans;
use tracing::trace;

/// Compile a set of kinded input artifacts to the requested targets. The `ast` input is decoded into
/// a `Db`; each target's backend fills its artifact from the shared columns. Targets default to
/// `[Wasm]` at the CLI, not here — this entry emits exactly what it is asked for.
///
/// The program arrives as the `ast`-kinded artifact — the canonical binary form — which this entry
/// decodes (`codec::decode`) into the AST arena the whole pipeline reads; the compiler never receives
/// source text, only an AST value obtained by decoding the binary form.
///
//= spec/capabilities/compiler-pipeline.md#the-compiler-operates-on-ast-values
//# The compiler MUST receive the program as an AST value obtained via quote or decode from the binary form.
///
/// A program compiles on the CORE guarantees alone — static typing (`collect_faults`), determinism, and
/// capability-safety (the no-home effect check) — with NO verification layer as a precondition; the seed
/// realizes no contract/refinement/proof layer, so engaging one is not something a program can (or must)
/// opt into here.
///
/// The pipeline this drives is a sequence of phases each with a defined input and output — decode → layout
/// → the demand-driven `Db` columns (resolve → infer → lower → select) → backend emission — and each phase
/// is a deterministic function of its input (the `Db` is a pure memoized column store; no phase reads a
/// clock, the environment, or iteration order). Faults are collected across the whole program, not raised
/// at the first: `collect_faults` returns EVERY reached fault and `compile` reports one diagnostic per
/// fault, so an error in one definition does not abort typing of its well-formed siblings.
///
//= spec/capabilities/compiler-pipeline.md#the-pipeline-has-defined-phases
//# The compiler MUST proceed through phases each of which has a defined input and a defined output.
///
//= spec/capabilities/compiler-pipeline.md#the-pipeline-has-defined-phases
//# Each phase MUST produce output that is a deterministic function of its input.
///
//= spec/capabilities/compiler-pipeline.md#phases-recover-from-errors
//# The compiler MUST report all diagnostics it can produce for a program rather than stop at the first.
///
//= constitution.md#viii-verification-is-progressive-and-meaning-preserving
//# A program MUST be compilable when only the core guarantees — static typing, determinism, and capability-safety — are satisfied.
///
//= spec/capabilities/verification-layers.md#a-program-compiles-without-any-layer
//# A program MUST compile when only the core guarantees — static typing, determinism, and capability-safety — are satisfied.
///
//= spec/capabilities/verification-layers.md#a-program-compiles-without-any-layer
//# Engaging a verification layer MUST be something a program opts into, not a precondition of compiling.
pub fn compile(inputs: &[Artifact], targets: &[Target]) -> CompileOutput {
    trace!(target: "rcdzc::compile", inputs = inputs.len(), targets = targets.len(), "compile requested");
    // Select the `ast` input artifact(s) and decode them into ONE arena. A single `ast` (the common
    // case) decodes directly — byte-identical to today. TWO OR MORE `ast` artifacts (or an explicit
    // `entry` marker) is a PACKAGE: the files are spliced into one arena under a synthesized `(do …)`
    // root by `link()` before `Db::load` (`DESIGN-package-linking.md` §3). Everything downstream of the
    // splice is unchanged — it sees one program in one arena.
    let ast_arts: Vec<&Artifact> = inputs
        .iter()
        .filter(|a| a.kind == Artifact::KIND_AST)
        .collect();
    let entry_name = inputs
        .iter()
        .find(|a| a.kind == link::KIND_ENTRY)
        .map(|a| String::from_utf8_lossy(&a.bytes).into_owned());
    let (arenas, linkage) = match link_inputs(&ast_arts, entry_name.as_deref()) {
        Ok(a) => a,
        Err(r) => return fail(vec![r]),
    };
    // For a linked PACKAGE, emit the `link-map` demux artifact (`DESIGN-package-linking.md` §6): it
    // lets a consumer map a cross-file diagnostic's GLOBAL node id → `(file, local id)` for source
    // mapping. It rides alongside every output that carries node-anchored diagnostics — seeded into the
    // artifact base below so both the fault path and the clean-emit path carry it. `None` (single file)
    // adds nothing.
    let link_map = linkage.as_ref().map(|lk| {
        Artifact::new(
            link::KIND_LINK_MAP,
            "link-map",
            link::encode_link_map(&lk.files),
        )
    });

    let mut db = Db::load_linked(arenas, linkage);
    trace!(target: "rcdzc::compile", defs = db.defs.len(), exports = db.exports.len(), "loaded program");

    // Decode the optional `sidecar` request list — the program that DRIVES this compilation
    // (`DESIGN-sidecar-api.md`). Absent (the common case) means "no requests": behavior is exactly
    // today's, driven by `targets` alone. A present-but-MALFORMED list is a DECLINE (a diagnostic),
    // never a panic or a silently-ignored input — reject-don't-miscompile at the tool edge
    // (`build-tool-interface.md` §The kind of an artifact … reported as a diagnostic).
    let requests = match inputs.iter().find(|a| a.kind == sidecar::KIND_SIDECAR) {
        Some(a) => match sidecar::decode(&a.bytes) {
            Some(rs) => rs,
            None => return fail(vec![Reject::decline("malformed `sidecar` request list")]),
        },
        None => Vec::new(),
    };
    // Partition the requests: a QUERY reads a fact column (total — run now, answers even for a program
    // that fails to emit), an EMIT materializes a backend artifact (fault-gated — joins `targets`,
    // which IS the Emit half already). `targets` first, then the sidecar's Emit requests, in order.
    let mut queries = Vec::new();
    let mut emit_targets: Vec<Target> = targets.to_vec();
    for req in &requests {
        match req {
            sidecar::Request::Query(q) => queries.push(q.clone()),
            sidecar::Request::Emit(t) => emit_targets.push(*t),
        }
    }

    // Decode the optional `spans` input — the source-position side-table the backend reads to emit
    // debug information (`DESIGN-debug-info-rcdzc.md` §2.1a). A present-but-MALFORMED table is a
    // DECLINE, exactly like a malformed sidecar list (reject-don't-miscompile at the tool edge). Absent
    // is the common case (no debug build), and nothing reads it.
    let span_data = match inputs.iter().find(|a| a.kind == spans::KIND_SPANS) {
        Some(a) => match spans::decode(&a.bytes) {
            Some(s) => Some(s),
            None => return fail(vec![Reject::decline("malformed `spans` artifact")]),
        },
        None => None,
    };

    // §9.4 — a debug `Emit` request needs the `spans` DATA to draw its debug info from. If a debug
    // target is requested but no `spans` input is present, DECLINE with a specific diagnostic rather
    // than silently emit an undecorated component (which would let "the user asked for debug and got a
    // debug-free artifact with no explanation" happen). The debug `Emit` is the SIGNAL; `spans` is the
    // DATA — both are required together. (D0's `name` section does not itself need spans, but keeping
    // the requirement uniform means the DWARF increments D2+ inherit it for free and a debug build
    // always either carries full debug info or says why it cannot.)
    if emit_targets.iter().any(|t| t.needs_spans()) && span_data.is_none() {
        return fail(vec![Reject::decline(
            "a debug artifact was requested but no `spans` input artifact was supplied \
             (debug info needs the source span side-table)",
        )]);
    }

    // Run the QUERIES first. A fact read is TOTAL (`tooling-and-lsp.md` §An Agent Queries The Compiler
    // For Any Static Fact) and PURE with respect to the artifact channel: it answers regardless of
    // whether the program emits, and it never denies a component. So query artifacts ride ALONGSIDE
    // whatever the emit path produces — including alongside the diagnostics when emit declines. (They
    // are computed before layout so a query answers even for a program with no export, which layout
    // would otherwise decline.) Running them first also warms the shared columns the emit path reads.
    // Seed the artifact base with the package `link-map` (if any), so it rides EVERY output path —
    // the fault path (`fail_with(query_artifacts, …)`), the query-only return, and the clean emit
    // (which starts `artifacts = query_artifacts`). A single-file compile seeds nothing.
    let mut query_artifacts: Vec<Artifact> = link_map.into_iter().collect();
    query_artifacts.extend(queries.iter().map(|q| {
        let r = sidecar::run_query(&mut db, q);
        Artifact::new(r.kind, r.name, r.bytes)
    }));

    // QUERY-ONLY mode: the sidecar asked for facts but no artifact to build (`emit_targets` empty
    // because neither `targets` nor an Emit request named one). There is nothing to lay out or emit —
    // return the query answers directly, without running the (possibly-declining) layout/fault path.
    // Guarded on `!queries.is_empty()` so a plain `compile(inputs, &[])` with no sidecar keeps today's
    // behavior (fall through to the emit path, which runs layout as before).
    if emit_targets.is_empty() && !queries.is_empty() {
        return CompileOutput {
            artifacts: query_artifacts,
            diagnostics: Vec::new(),
        };
    }

    // Compute the boundary layout once (target-neutral). A program with no export declines. Layout also
    // gates emission on properties `collect_faults` does not model (e.g. a boundary shape it cannot lay
    // out), so it must run — a well-formed program can still fail to lay out. (Its coarser declines that
    // DUPLICATE a `collect_faults` coded fault — the ambiguous-param case — are handled by reporting the
    // coded fault too, below, so the sidecar `check` surfaces it; the emit path keeps layout's decline.)
    let layout = match layout::compute(&mut db) {
        Ok(l) => l,
        Err(r) => {
            trace!(target: "rcdzc::compile", reason = %r.message, "layout declined");
            // Layout's decline can be the WEAKER twin of a coded well-formedness fault `collect_faults`
            // reports better — e.g. a missing export: layout returns an UNCODED "export `x` names no
            // definition", but `collect_faults` gives the coded CDZ0101 WITH a "did you mean?" + replace
            // fix. `compile` short-circuited on layout's decline before `collect_faults` ran, so `cdz
            // compile` showed the fix-less message while `cdz check` showed the actionable one — a
            // check≡compile discrepancy. Run `collect_faults` now; if it found any coded fault, report
            // THAT set (the richer, actionable diagnostics), keeping layout's decline only as the fallback
            // for a decline `collect_faults` does not model (a boundary-shape it cannot lay out).
            let mut faults = collect_faults(&mut db);
            if faults.is_empty() {
                return fail_with(query_artifacts, vec![r]);
            }
            for f in &mut faults {
                sanitize_origin(&db, f);
            }
            return fail_with(query_artifacts, faults);
        }
    };

    // Collect every reached fault across the reachable definitions, module-wide (report ALL, not just
    // the first — `compiler-pipeline.md` §Phases Recover From Errors). The check does not stop at the
    // first fault; it recovers and gathers the whole independent set in one pass:
    //= spec/capabilities/diagnostics.md#diagnosis-reports-the-maximal-independent-set-in-one-pass
    //# The compiler MUST recover from an error and report the maximal set of independent problems in one pass rather than only the first.
    let _ = &layout; // layout gates EMISSION below; well-formedness (faults) is layout-independent.
    let mut faults = collect_faults(&mut db);
    if !faults.is_empty() {
        trace!(target: "rcdzc::compile", faults = faults.len(), "compilation FAILED (faults reported, no artifact)");
        for f in &mut faults {
            sanitize_origin(&db, f);
        }
        return fail_with(query_artifacts, faults);
    }
    trace!(target: "rcdzc::compile", targets = emit_targets.len(), "program clean — emitting artifacts");

    // WARNINGS (non-error; ride alongside the artifact). The program is clean — every REACHED provable
    // trap already faulted above — so a computation that PROVES it traps yet survives to here was
    // dropped by dead-code elimination (its value is unobserved: an unprojected element, an unreferenced
    // binding, an unused argument). That is conformant but almost always a defect, so warn — the build
    // still succeeds. Dropping an unobserved trapping computation is the MAY-elide; the non-error warning
    // is the SHOULD-emit-a-diagnostic on a dropped provably-trapping computation.
    //= spec/capabilities/core-semantics.md#a-trap-occurs-only-where-its-computation-is-observed
    //# An implementation MAY decline to evaluate a computation whose value cannot affect the program's observable behavior — one whose result reaches neither the program's terminal value nor any host call — and so MAY elide a trap that computation would raise.
    //= spec/capabilities/core-semantics.md#a-trap-occurs-only-where-its-computation-is-observed
    //# Because eliding a computation the implementation can PROVE would trap is far more likely a program defect than an intent, an implementation SHOULD emit a diagnostic of non-error severity — one that leaves the build successful — when it drops a provably-trapping computation whose value is unobserved, so that a program does not silently discard a computation that could never have produced a value.
    let mut diagnostics = collect_dead_trap_warnings(&mut db);
    // Unused-binding warnings (a let binding / parameter / non-exported def nothing references, unless
    // `_`-prefixed) ride alongside the artifact too — well-formed, just likely a defect (CDZ0306).
    diagnostics.extend(collect_unused_binding_warnings(&mut db));
    // Redundant-match-arm warnings (an arm an earlier arm already covers — CDZ0211): dead code, like an
    // unused binding, so a warning that rides alongside the artifact without denying it.
    diagnostics.extend(collect_redundant_arm_warnings(&mut db));
    // Discarded-value warnings (a pure, non-Unit, non-final `do` form whose value is thrown away —
    // CDZ0307): the sequencing-block analogue of the unused-binding warning — the SHOULD-emit-a-diagnostic
    // on a pure non-final form whose value is discarded.
    //= spec/capabilities/core-semantics.md#a-discarded-pure-non-final-value-is-diagnosed
    //# An implementation SHOULD emit a diagnostic of non-error severity — one that leaves the build successful — for such a form, so that a program does not silently discard the value of a pure computation whose result it never observes.
    diagnostics.extend(collect_discarded_value_warnings(&mut db));

    // A run that emits BOTH a plain component (`Wasm`) AND a detached DWARF sidecar (`Dwarf`) links the
    // two: the component carries an `external_debug_info` custom section naming the sidecar file, so a
    // debugger auto-loads the symbols (`DESIGN-debug-info-rcdzc.md` §9.2, Mode S). The name is the
    // sidecar artifact's on-disk file (`<program>.dwarf`, matching the CLI's `ext_for_kind`). Only when
    // a LEAN `Wasm` is paired with a `Dwarf` — a `WasmDebug` embeds its own DWARF and needs no pointer.
    let external_debug_info =
        if emit_targets.contains(&Target::Wasm) && emit_targets.contains(&Target::Dwarf) {
            Some(format!("{}.dwarf", program_name(&db)))
        } else {
            None
        };

    // Clean: ask each requested target's backend to fill its artifact. The query artifacts (facts
    // read above) lead, then each emitted backend artifact — all one kinded-artifact list, selected by
    // kind (`build-tool-interface.md`).
    let mut artifacts = query_artifacts;
    for &target in &emit_targets {
        match backend::emit(
            target,
            &mut db,
            &layout,
            span_data.as_ref(),
            external_debug_info.as_deref(),
        ) {
            Ok(bytes) => artifacts.push(Artifact::new(
                target.artifact_kind(),
                program_name(&db),
                bytes,
            )),
            Err(mut r) => {
                trace!(target: "rcdzc::compile", ?target, reason = %r.message, "target emit declined");
                sanitize_origin(&db, &mut r);
                diagnostics.push(Diagnostic::from_reject(&r));
            }
        }
    }
    CompileOutput {
        artifacts,
        diagnostics,
    }
}

/// Drop a fault's origin node if it is NOT a user-program node — the diagnostic boundary. A fault may
/// be anchored (during the query) to a PRELUDE node or an evaluator-SYNTHESIZED node (a β-reduced
/// body, a built `(Int W)` module); such an id has no source position, so reporting it would map to
/// garbage in the consumer's span table. Here, at the edge where a `Reject` becomes a consumer-facing
/// diagnostic, a non-user origin is cleared to `None` (reported as unanchored) rather than leaked
/// (`query-engine.md` §Provenance Is Recovered By Back-Reference — only a real source node maps back).
fn sanitize_origin(db: &Db, reject: &mut Reject) {
    if let Some(id) = reject.at
        && !db.is_user_node(id)
    {
        reject.at = None;
    }
}

/// A convenience over [`compile`]: a lone canonical-AST byte string → the WebAssembly component bytes,
/// or the first error diagnostic. What the tests and simple callers use.
///
/// Establishes the compile-stack precondition (`crate::host::run_with_compiler_stack`) around the
/// `compile` call, exactly as the bin does at its top level (`cli.rs`, `cdz` main). The bin wraps
/// `compile` directly; the tests and simple callers reach `compile` THROUGH here, so wrapping here is
/// the one place that gives every such caller the guard-sized stack — without it, a deep-but-finite
/// recursion (e.g. a depth-25 β-inline chain, or a module sibling call) overflows a `cargo test` worker
/// thread's ≈2 MB stack in a debug build and ABORTS before the semantic depth guard can fire. See
/// `crate::host` for why the stack is sized from `DESCENT_DEPTH_LIMIT`.
pub fn compile_component(ast_bytes: &[u8]) -> Result<Vec<u8>, Diagnostic> {
    let out = crate::host::run_with_compiler_stack(|| {
        compile(
            &[Artifact::new(
                Artifact::KIND_AST,
                "main",
                ast_bytes.to_vec(),
            )],
            &[Target::Wasm],
        )
    });
    match out.artifact(Target::Wasm.artifact_kind()) {
        Some(bytes) => Ok(bytes.to_vec()),
        None => {
            // Prefer a CODED error (a rejection — an ill-formed program, the stronger, more actionable
            // "no") over an uncoded DECLINE (a construct not yet lowered), matching the safety ordering
            // (`reference-compiler.md` §Outcomes Are Ordered By Safety). A program that is both rejected
            // AND has a not-yet-lowered construct reports the rejection. (A single-error caller — the
            // tests, the pipe — wants the decision, not the incidental decline the same body may also
            // raise; e.g. an ungranted perform is CDZ0401, not the "no handler here" decline its standalone
            // lowering also emits.)
            let errors = || {
                out.diagnostics
                    .iter()
                    .filter(|d| d.severity == Severity::Error)
            };
            let chosen = errors()
                .find(|d| d.code.is_some())
                .or_else(|| errors().next())
                .cloned();
            Err(chosen.unwrap_or(Diagnostic {
                severity: Severity::Error,
                code: None,
                message: "compilation produced no component".into(),
                node: None,
                fix: None,
            }))
        }
    }
}

/// Every well-formedness fault of a loaded program, as consumer-facing diagnostics, WITHOUT requiring
/// the program to export anything or to emit — the total, layout-independent read an IDE wants for
/// "diagnostics as you type". This is exactly the fault set [`compile`] reports (it calls the same
/// [`collect_faults`]), minus the emission path that a missing export would otherwise decline at
/// `layout::compute` before any fault is seen. A bare expression, a mid-edit buffer, or a set of
/// sibling defs with no `(export …)` all yield their real type/shape faults here. Origins are
/// sanitized to user nodes (a prelude/synthesized node has no source span), so a consumer holding the
/// front-end span table maps every `node` to a text range.
///
/// The caller passes a `Db` it loaded from the SAME arena its span table was built from
/// ([`Db::load`]), so the diagnostics' node ids index that span table directly.
pub fn diagnostics(db: &mut Db) -> Vec<Diagnostic> {
    // `collect_faults` already sanitizes each fault's origin (stripping a synthesized-node anchor) and
    // dedups, so the faults are display-ready here.
    let faults = collect_faults(db);
    let mut out: Vec<Diagnostic> = faults.iter().map(Diagnostic::from_reject).collect();
    // WARNINGS ride alongside the faults so "diagnostics as you type" (`Query::Diagnostics` / `cdz
    // check`) surfaces them too — an unused binding and a dead-trap are exactly the kind of thing an
    // editor's inline lint should show. They are non-error severity, so they never deny an artifact.
    out.extend(collect_dead_trap_warnings(db));
    out.extend(collect_unused_binding_warnings(db));
    out.extend(collect_redundant_arm_warnings(db));
    out.extend(collect_discarded_value_warnings(db));
    out
}

/// The FIXED registry of module-directive keys the specification defines (`modules-and-namespaces.md` §A
/// Module Directive Is Drawn From A Fixed Set). The single source of truth for BOTH the `(pragma …)`
/// validation (a key not here is CDZ0601) and the "did you mean?" suggestion an unknown key gets — so the
/// suggestion can never drift from the accepted set. Small and closed today (`default-integer`); a new
/// spec directive adds its key here.
const PRAGMA_REGISTRY: &[&str] = &["default-integer"];

/// The numeric-domain check for a well-formed `(pragma default-integer <T>)`: the directive names the
/// type OTHERWISE-UNCONSTRAINED integer literals default to, so `<T>` MUST be an integer type
/// (`numeric-model.md` §A Module May Declare Its Default Integer Literal Type). `<T>` is reduced to a
/// type-VALUE by the ordinary evaluator (`eval::typeval_of`, the same path an annotation's type position
/// takes), and the integer-domain predicate is `Ty::Int` — the ONE representation every fixed-width and
/// deferred integer type shares. A non-integer type-value (`Float64` → `Ty::Float`, a record, …) is the
/// numeric-domain rejection CDZ0303, distinct from the structural CDZ0602 (wrong arity) / CDZ0601
/// (unknown key). The integer domain is `Ty::Int` (fixed-width + deferred) OR `Ty::BigInt` (the
/// arbitrary-precision integer, now modeled). A type argument that does NOT reduce to a concrete
/// type-value — an unbound name, a non-type expression — returns `None` here: NOT a domain violation
/// (absence of proof is not proof of non-integer), so the whole program declines downstream rather than
/// being falsely rejected. The predicate is CONSERVATIVE — it fires only on a type it can prove is
/// non-integer, never on absence of proof.
fn non_integer_default_fault(db: &mut Db, form: StructId, ty_expr: StructId) -> Option<Reject> {
    // An UNBOUND type name is the SAME CDZ0101 an annotation gives (`(: x Nope)`). Resolution — not the
    // `typeval_of` reduction — is what tells an unbound name apart from a BOUND type whose `typeval_of`
    // this compiler cannot yet reduce to a concrete `Ty`: such a bound type resolves to SOMETHING (a
    // `Ref`/`Record`) whose `typeval_of` is `None` (the conservative accept below), while an unbound name
    // resolves to a `Poison(CDZ0101)`. Surfacing that poison here closes the silent-drop hole (a meaning-changing
    // directive naming a nonexistent type must not be accepted) WITHOUT falsely rejecting a legitimate
    // unmodeled integer default — the exact distinction `numeric-model.md`'s conservatism turns on. Keyed
    // on the resolver's own `Code::Unbound`, not on the name string, so no name knowledge lives here.
    if let crate::resolved::Resolved::Poison(reject) = crate::resolve::resolved_of(db, ty_expr)
        && reject.code == Some(Code::Unbound)
    {
        return Some(reject);
    }
    let ty = crate::eval::typeval_of(db, ty_expr)?;
    // The integer domain is `Ty::Int` (every fixed-width + deferred integer) AND `Ty::BigInt` (the
    // arbitrary-precision integer) — both are integer types the numeric model admits as a declarable
    // default (`options/numeric-model/explicit-checked.md` §"Any integer type is declarable" lists
    // `BigInt`). A non-integer type-value (`Float64`, a record, …) is the CDZ0303 domain rejection.
    if matches!(ty, crate::ty::Ty::Int(_) | crate::ty::Ty::BigInt) {
        return None;
    }
    // No mechanical fix is offered here even though the domain error is clear: the `default-integer`
    // pragma's EFFECT is not yet modeled, so EVERY `(pragma default-integer <T>)` — even a well-formed
    // `Int64` one — declines downstream as an unmodeled top-level form. Suggesting `Int64` would merely
    // trade CDZ0303 for that decline (a cascade), which `--verify-fixes` rightly refuses. Honest-no-fix:
    // the prose already says "must name an integer type"; a fix waits until the pragma actually compiles.
    Some(
        Reject::coded(
            Code::NonIntegerDefault,
            format!(
                "`default-integer` must name an integer type, but `{}` is not an integer type \
                 (the default fixes the type otherwise-unconstrained integer literals take)",
                ty.render_name()
            ),
        )
        .at(form),
    )
}

/// Every fault across the program's definitions. Well-formedness — scope resolution and type
/// checking — is UNCONDITIONAL: it holds over EVERY top-level definition's body, not only the ones
/// reachable from an export, because a program is well-formed or not regardless of what is asked to
/// compile (`core-semantics.md` §Binding Is Lexical — the unbound-name rule is not gated on
/// reachability; an ill-formed uncalled sibling definition is still rejected). Emission stays
/// reachability-driven (the `layout` decides what is emitted); only well-formedness is total.
///
/// Every reachable expression is typed by inference before emission, and any type fault this collects
/// DENIES the component (`compile` returns the faults with no artifact) — so a program that is not
/// well-typed is rejected at compile time rather than emitted carrying a deferred type error:
///
/// This walk records a `Reject` for every faulting definition and keeps going over the well-formed
/// remainder rather than aborting at the first fault, so the maximal set of diagnostics is produced in
/// one pass (each `Reject` becomes one error diagnostic in `compile`'s output). The walk visits nodes in
/// a source-determined order (the arena's node ids), and the closing `dedup_faults` is stable — no clock,
/// environment, or nondeterministic iteration order enters — so the SEQUENCE of diagnostics is itself a
/// deterministic function of the source:
///
//= spec/capabilities/diagnostics.md#diagnostics-are-emitted-in-a-deterministic-order
//# The sequence of diagnostics the compiler emits for a program MUST be a deterministic function of the program's source.
///
//= spec/capabilities/compiler-pipeline.md#phases-recover-from-errors
//# A phase that encounters an error in one part of a program MUST record a diagnostic for that error.
///
//= spec/capabilities/compiler-pipeline.md#phases-recover-from-errors
//# A phase that encounters an error in one part of a program MUST continue processing the well-formed remainder rather than abort the whole compilation.
///
//= spec/capabilities/type-system.md#every-expression-has-a-static-type
//# Every expression in a well-formed program MUST have a type determined before the program is compiled to a component.
///
//= spec/capabilities/type-system.md#every-expression-has-a-static-type
//# A program that is not well-typed MUST be rejected at compile time rather than compiled to a component carrying a deferred type error.
///
//= constitution.md#vii-strong-static-typing-is-mandatory
//# Every expression in a well-formed program MUST have a statically determined type before the program is compiled to a component.
///
//= constitution.md#vii-strong-static-typing-is-mandatory
//# The compiler MUST reject a program that is not well-typed rather than emit a component carrying a deferred type error.
///
//= constitution.md#vii-strong-static-typing-is-mandatory
//# The seed compiler generation MUST realize the static-typing obligations of this section rather than defer them, because under the two-compiler bootstrap it compiles programs to components rather than evaluating them dynamically, so the dynamic-evaluation basis on which a seed generation formerly deferred typing no longer holds.
///
//= constitution.md#vii-strong-static-typing-is-mandatory
//# A program that is not well-typed MUST be rejected with the machine-readable diagnostic code for the type rule it violates, so that a type rejection is a compile-time event every generation makes rather than a runtime outcome only a dynamic evaluator would exhibit.
/// Collect the TYPE-EXPRESSION POSITIONS to validate within a variant payload type expression at `occ`,
/// pushing `(position, params)` for each. A `(Record (field Type)…)` form descends only into each field
/// pair's TYPE (the second element) — a record type's field NAME is a label, never a global type, so it
/// must not be validated (validating the whole record form out-of-context mis-resolves the field name as
/// unbound). Every OTHER form (a bare name, a `(Tuple …)`/`(List …)`/`(Option …)` application) is ONE
/// position validated whole — its inner unknown names surface via the ordinary resolver descent. This is
/// the validation-position twin of `db::collect_type_params`'s record-aware descent (which collects the
/// PARAMS from the same positions), so records are handled WITHOUT the field-name false positive.
fn push_payload_type_positions(
    db: &Db,
    occ: StructId,
    params: &[String],
    out: &mut Vec<(StructId, Vec<String>)>,
) {
    if db.ast.head_name(occ) == Some("Record")
        && let crate::ast::Struct::List(children) = db.ast.get(occ)
    {
        // Descend into each `(name Type)` field pair's TYPE (the second element), skipping the name.
        for &pair in children.iter().skip(1) {
            if let crate::ast::Struct::List(items) = db.ast.get(pair)
                && items.len() == 2
            {
                push_payload_type_positions(db, items[1], params, out);
            }
        }
        return;
    }
    // A non-record position: validate the whole expression. (A nested record inside a `(Tuple … (Record
    // …))` is reached because the ordinary resolver descent inside `type_errors` handles it — but a record
    // at the TOP of a payload, or as a tuple element, would carry its field names into that walk; splitting
    // record fields out HERE keeps the validated positions record-free. A `(Tuple (Record …) …)` element
    // that is itself a record is split by recursing on Tuple children.)
    if let crate::ast::Struct::List(children) = db.ast.get(occ)
        && children
            .iter()
            .skip(1)
            .any(|&c| db.ast.head_name(c) == Some("Record") || is_record_bearing(db, c))
        && matches!(db.ast.head_name(occ), Some("Tuple" | "List" | "Option"))
    {
        // A container whose elements may be records — descend so a record element's fields are split out,
        // not carried whole into the resolver walk.
        for &c in children.iter().skip(1) {
            push_payload_type_positions(db, c, params, out);
        }
        return;
    }
    out.push((occ, params.to_vec()));
}

/// Whether the type-expression subtree at `id` contains a `(Record …)` form at any depth — the guard the
/// payload-position collector uses to decide whether a container element needs record-splitting descent.
fn is_record_bearing(db: &Db, id: StructId) -> bool {
    if db.ast.head_name(id) == Some("Record") {
        return true;
    }
    match db.ast.get(id) {
        crate::ast::Struct::List(children) => children.iter().any(|&c| is_record_bearing(db, c)),
        _ => false,
    }
}

/// Validate ONE declaration-site type-expression position (a variant payload, an effect-operation
/// arg/result). `params` are the enclosing declaration's type parameters (empty for an effect op). Pushes
/// a reject to `out` iff the position is not a valid type: a genuinely-unknown Capitalized type name
/// (CDZ0101, `Nonesuch`) or a well-formed non-type (`5` → CDZ0203, `<what> requires a type`). A type
/// PARAMETER (a name in `params`, or — since a free lowercase name IS a type variable by the language
/// convention — any lowercase name) is valid and never faults; a param-parameterized application
/// (`(Option a)`) fails `typeval_of` out-of-context but is valid, so ONLY a real unknown Capitalized name
/// (not a param) survives the filter — every other artifact of out-of-context resolution (`cannot apply`,
/// a nested-param `unbound name`) is dropped. `what` names the position for the non-type message.
fn validate_type_position(
    db: &mut Db,
    pos: StructId,
    params: &[String],
    what: &str,
    out: &mut Vec<Reject>,
) {
    // A bare position that is a type PARAMETER (declared, or a free lowercase variable) is valid — skip it.
    if let Some(name) = db.ast.as_name(pos)
        && (params.iter().any(|p| p == name) || name.starts_with(|c: char| c.is_ascii_lowercase()))
    {
        return;
    }
    if crate::eval::typeval_of(db, pos).is_some() {
        return; // denotes a real type (self/mutual/forward refs + nested generics resolve)
    }
    let raw = type_errors(db, pos);
    let raw_count = raw.len();
    // KEEP only a genuinely-unknown type name: a CDZ0101 whose name is neither a declared param NOR a
    // lowercase type-variable. Drop every other out-of-context artifact.
    let kept: Vec<Reject> = raw
        .into_iter()
        .filter(|f| {
            f.code == Some(Code::Unbound)
                && unbacktick(&f.message).is_none_or(|n| {
                    !params.iter().any(|p| p == n)
                        && !n.starts_with(|c: char| c.is_ascii_lowercase())
                })
        })
        .collect();
    if !kept.is_empty() {
        out.extend(kept);
    } else if raw_count == 0 {
        // No faults AND not a type — a well-formed NON-type (a literal, a value). (When faults WERE
        // surfaced but were all param/variable artifacts, the position IS a valid parametric type.)
        out.push(
            Reject::coded(
                Code::TypeMismatch,
                format!("{what} requires a type, but found a non-type"),
            )
            .at(pos),
        );
    }
}

/// The first backtick-quoted substring of `msg` (`` unbound name `x` `` → `x`), or `None` if there is no
/// `` `…` `` pair. Used to read the offending NAME out of a coded message when the `Reject` carries only
/// the rendered string (the variant-payload check filters a type-parameter's unbound-name fault by name).
fn unbacktick(msg: &str) -> Option<&str> {
    let start = msg.find('`')? + 1;
    let rest = &msg[start..];
    let end = rest.find('`')?;
    Some(&rest[..end])
}

fn collect_faults(db: &mut Db) -> Vec<Reject> {
    let mut faults = Vec::new();
    // Fresh reached-poison walk state for this call: the visited-set (which lets the walk skip a shared
    // core DAG node instead of re-descending it as a tree) accumulates across the per-body walks BELOW
    // that all feed this one `faults` vec, and is stale from any prior `collect_faults` call — clear it.
    db.reached_visited.clear();
    db.reached_clipped = false;
    // NON-FINAL `,@` SPLICE in a QUOTE PATTERN — `` `(f ,@init ,last) `` puts a tail-binding `,@` before
    // a fixed element (`metaprogramming.md`: a `,@<name>` MUST appear only as the final element). Detected
    // at load by `quote::reify_quotes` (which leaves the offending quasiquote un-reified); reported here as
    // CDZ0221, the quote-pattern analogue of the binary-form CDZ0220 (an unsized `bytes` segment is legal
    // only last).
    for &occ in &db.nonfinal_splice_patterns.clone() {
        faults.push(
            Reject::coded(
                crate::diag::Code::NonFinalSplice,
                "a `,@` splice in a quote pattern binds the remaining elements, so it must be the \
                 final element of its template",
            )
            .at(occ),
        );
    }
    // UNMODELED TOP-LEVEL FORM. A top-level `(head …)` whose head resolves to NOTHING — neither a
    // recognized declaration (`def`/`export`/`type`/`effect`) nor a grammar head nor a bound name. Two
    // real cases reach here, and they are STRUCTURALLY INDISTINGUISHABLE (both a list led by an unbound
    // name): an ordinary APPLICATION of an unbound function (`foo(bar)` — by FAR the common case, an agent
    // calling a name it never defined) and a genuinely UNMODELED DECLARATION keyword (`(pragma …)`). The
    // old message ("`foo` is not a construct this compiler models") diagnosed only the rare case and
    // MISLED the common one — a plain unbound call read as an unsupported language feature. So LEAD with
    // the certain, actionable fact (the head is an unbound name — the same truth the nested `foo` in
    // `(def (g) (foo 1))` reports as CDZ0101), naming a near defined name when one is a plausible typo,
    // and note the unmodeled-declaration reading as the secondary possibility. It stays a DECLINE (not a
    // coded CDZ0101): a top-level unbound head still means the compiler cannot claim to compile the whole
    // program (decline-don't-miscompile — an unmodeled declaration's meaning would be silently dropped),
    // so the OUTCOME is unchanged; only the message stops misleading.
    let defined_names: Vec<String> = db.defs.iter().map(|d| d.name.clone()).collect();
    for (head, occ) in db.unknown_top_forms() {
        // `import` is a KNOWN surface keyword (the ML reader lexes `import { … } from "…"` → an
        // `(import …)` top-level form) that this compiler does NOT YET model — distinct from a typo. The
        // generic path below would suggest "did you mean `export`?" (import→export is only 2 edits), an
        // actively MISLEADING fix: an author who wrote `import` never meant its opposite. Name the real
        // situation — a recognized module form that is not yet supported — with NO swap fix.
        if head == "import" {
            faults.push(
                Reject::decline(
                    "`import` is a module form this compiler does not yet model (cross-module imports \
                     are not supported here) — the program cannot be compiled",
                )
                .at(occ),
            );
            continue;
        }
        // FIRST: is the head a plausible TYPO of a top-level DECLARATION KEYWORD (`exprot`→`export`,
        // `deff`→`def`)? That is the far likelier intent than a mistyped VALUE name — a top-level `(head
        // …)` form is a declaration position, so a near-miss for `def`/`export`/`type`/`effect`/`module`/
        // `pragma` (the closed keyword pool) is named AND carries a REPLACE fix on the HEAD occurrence (the
        // form's first child), the same closed-set "did you mean?"-with-fix the export-name / unbound-name
        // sites give (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A Fix). The pool
        // is the keyword set itself, so a suggestion can never name a keyword the grammar would reject.
        let keyword_hit =
            crate::diag::suggest::nearest(&head, crate::db::TOP_LEVEL_KEYWORDS.iter().copied());
        // The head NAME occurrence — the form's first child — is what a keyword fix rewrites (`(exprot f)`
        // → replace just `exprot`, not the whole form). Falls back to no fix if the shape is unexpected.
        let head_node = match db.ast.get(occ) {
            crate::ast::Struct::List(items) => items.first().copied(),
            _ => None,
        };
        // The hint text: prefer the keyword suggestion; else the two-tier defined-name hint (a confident
        // single typo, else the closest few — never nothing when defs exist), message-only.
        let hint = match &keyword_hit {
            Some(kw) => format!(" — did you mean `{kw}`?"),
            None => crate::diag::suggest::did_you_mean(&head, &defined_names, 3),
        };
        let mut reject = Reject::decline(format!(
            "unbound name `{head}` at the top level{hint} (if `{head}` is meant as a declaration, \
             it is not one this compiler models — the program cannot be compiled either way)"
        ))
        .at(occ);
        // Attach the keyword-swap fix only when both the near-miss keyword AND the head node are known.
        if let (Some(kw), Some(node)) = (keyword_hit, head_node) {
            reject = reject.with_fix(crate::diag::Fix::replace_heuristic(node, kw));
        }
        faults.push(reject);
    }
    // MODULE DIRECTIVE `(pragma <key> <arg>…)`. A directive's key must be drawn from the fixed registry
    // the specification defines (`modules-and-namespaces.md` §A Module Directive Is Drawn From A Fixed
    // Set), and its arguments must match the shape that key defines — so an unknown key is CDZ0601 and a
    // recognized key with the wrong argument shape is CDZ0602, rather than silently ignored (a dropped
    // meaning-changing directive would make one source mean two things on two toolchains). The registry
    // is small and fixed HERE (the spec's set); the ONLY key it defines today is `default-integer`, which
    // takes exactly one type argument. A WELL-FORMED directive (right key + arity) is NOT flagged here —
    // its semantic effect / domain check (`default-integer`'s integer-domain predicate → CDZ0303, and the
    // literal-defaulting behavior itself) is a separate, not-yet-built concern, so a well-formed pragma
    // still DECLINES downstream rather than being mistaken for compiled. Every `(pragma …)` in the arena
    // is checked (a top-level one or a module member alike); the fault anchors at the pragma form, which
    // sorts before a later reference, so it is the reported error.
    //
    // A `(pragma …)` is resolved entirely at compile time — it is validated here and never lowered to a
    // `Core` node, so it introduces NO runtime representation of its own into the emitted component (it
    // affects how the module is compiled without adding runtime cost or crossing the boundary).
    //= spec/capabilities/modules-and-namespaces.md#a-module-directive-is-compile-time-only
    //# A module directive MUST be resolved at compile time and MUST NOT introduce any runtime representation of its own into the emitted component, so that a directive affects how the module is compiled without adding runtime cost or crossing the boundary.
    for form in (0..db.ast.structure.len() as u32).map(StructId) {
        // OWN the tail + key before matching: the domain check below reduces the type argument via
        // `eval::typeval_of` (which needs `&mut Db`), and a borrowed `key: &str` / `ptail: &[StructId]`
        // into `db.ast` would pin `db` immutable for the whole match. `StructId` is `Copy`, so owning the
        // tail is cheap; the key is a short owned `String`.
        let Some(ptail) = db.ast.as_form(form, "pragma").map(<[_]>::to_vec) else {
            continue;
        };
        let key = ptail
            .first()
            .and_then(|&k| db.ast.as_name(k))
            .map(str::to_string);
        match key.as_deref() {
            // `default-integer <T>` — exactly one argument (the default type). Missing/extra → malformed;
            // a well-formed one whose type argument is not an integer type → the numeric-domain CDZ0303.
            Some("default-integer") => {
                if ptail.len() != 2 {
                    faults.push(
                        Reject::coded(
                            Code::MalformedDirective,
                            "`default-integer` takes exactly one type argument (e.g. `(pragma default-integer Int64)`)",
                        )
                        .at(form),
                    );
                } else if let Some(reject) = non_integer_default_fault(db, form, ptail[1]) {
                    faults.push(reject);
                }
            }
            // A key the fixed registry does not define — rejected, not ignored. If the typo'd key is a
            // near-miss for a registry key (`default-integr` → `default-integer`), name it AND carry a
            // replace fix on the KEY occurrence — the same closed-set "did you mean?" an unbound name /
            // absent field / undeclared handler op gets (`spec/capabilities/diagnostics.md` §A Diagnostic
            // Carries A Route To A Fix). The candidate pool is the registry itself, so a suggestion can
            // never name a key the validator would then reject.
            Some(other) => {
                // Name a near-miss registry key IN THE MESSAGE too (not only as a fix) — the same "did
                // you mean?" phrasing an unbound name / absent field / undeclared handler op carries, so
                // the human report is consistent and the suggestion is visible without `--json`.
                let candidate =
                    crate::diag::suggest::nearest(other, PRAGMA_REGISTRY.iter().copied());
                let hint = match &candidate {
                    Some(near) => format!(" — did you mean `{near}`?"),
                    None => String::new(),
                };
                let mut reject = Reject::coded(
                    Code::UnknownDirective,
                    format!(
                        "`{other}` is not a module directive this specification defines (the pragma \
                         registry is a fixed set; an unknown key is rejected, not ignored){hint}"
                    ),
                )
                .at(form);
                if let (Some(candidate), Some(&key_occ)) = (candidate, ptail.first()) {
                    reject =
                        reject.with_fix(crate::diag::Fix::replace_heuristic(key_occ, candidate));
                }
                faults.push(reject);
            }
            // `(pragma)` with no key at all — structurally malformed.
            None => {
                faults.push(
                    Reject::coded(
                        Code::MalformedDirective,
                        "a `(pragma …)` directive needs a key (e.g. `(pragma default-integer Int64)`)",
                    )
                    .at(form),
                );
            }
        }
    }
    // DUPLICATE DEFINITION. A module evaluates to a record of its definitions, and a record has a FIXED
    // SET of field names (core-semantics.md #A Record Has A Fixed Set Of Named Fields), so defining the
    // same name twice is the same ill-formedness `(record (a 1) (a 2))` is rejected for (CDZ0201) — not
    // resolved by an implicit first-wins precedence. Each definition after the first with a given name
    // is reported, anchored at its signature occurrence. (Checked here, not in the scan, so the reject
    // carries a node the diagnostic edge can map.)
    // A def with no extractable name (an as-yet-unmodeled shape — e.g. a value definition `(def x 1)`,
    // which the scan does not yet name) is SKIPPED here: it does not register a name, so it cannot
    // collide. Only genuinely NAMED definitions participate in the fixed-name-set check, so two
    // distinct un-named defs are not mistaken for a duplicate of the empty name.
    // An INTERNAL def (`Def::internal`) is compiler bookkeeping — a module-member / do-local FUNCTION
    // registered so a recursive call can lower to a `Core::Call`, keyed by its (possibly β-COPIED) body.
    // It does NOT declare a user name (its name resolves by lexical scope, not the fixed export set), and
    // the SAME source function may be registered more than once (once at load by its original body, again
    // per β-copy of an enclosing inlined helper). So it must NOT participate in the duplicate-name check —
    // else a legitimately-once-declared recursive function inlined at a call site would spuriously report
    // "defined more than once". Only genuine (non-internal) user definitions form the fixed name set.
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let dups: Vec<(String, StructId)> = db
        .defs
        .iter()
        .filter(|d| !d.internal && !d.name.is_empty() && !seen.insert(d.name.as_str()))
        .map(|d| (d.name.clone(), d.sig_occ))
        .collect();
    for (name, sig_occ) in dups {
        faults.push(
            Reject::coded(
                Code::Malformed,
                format!("`{name}` is defined more than once (a module has a fixed set of names)"),
            )
            .at(sig_occ),
        );
    }
    // DUPLICATE EXPORT. A module's exports are a record whose fields are the exported names
    // (core-semantics.md §A Module Evaluates To A Record Of Its Exports), and a record has a fixed set
    // of field names — so exporting the same name twice is the same ill-formedness as a duplicate
    // record field or a duplicate definition (CDZ0201). Two `(export a)` clauses would emit two export
    // entries named `a`, which the component binary format forbids, so the emitted bytes fail to parse:
    // reject BEFORE emitting rather than miscompile an invalid component (decline-don't-miscompile).
    // Each export clause after the first with a given name is reported, anchored at its clause.
    // How many exported names each `(export …)` CLAUSE contributes (keyed by the clause occurrence): a
    // single-name `(export a)` has one, a multi-name `(export a b)` has several. Used to pick what the
    // duplicate-export delete fix removes — the whole clause vs. just the redundant name.
    let mut names_per_clause: crate::fxhash::FxHashMap<StructId, usize> =
        crate::fxhash::FxHashMap::default();
    for e in &db.exports {
        *names_per_clause.entry(e.occ).or_insert(0) += 1;
    }
    let mut seen_exports: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let dup_exports: Vec<(String, StructId, StructId)> = db
        .exports
        .iter()
        .filter(|e| !seen_exports.insert(e.name.as_str()))
        .map(|e| (e.name.clone(), e.name_occ, e.occ))
        .collect();
    for (name, name_occ, clause_occ) in dup_exports {
        // `name_occ` is a REDUNDANT exported NAME (a later occurrence — the first is not in `dup_exports`).
        // Exporting a name is idempotent in intent: the earlier occurrence already makes it public, so the
        // direct repair is to DELETE the redundant export. WHAT to delete depends on the clause's arity:
        //   - a MULTI-name clause `(export a b a)` → delete just the redundant NAME atom, leaving `b` and
        //     the first `a` (`(export a b)`).
        //   - a SINGLE-name clause `(export a)` → delete the WHOLE clause. Deleting only the name would
        //     leave an empty `(export)`, itself now a CDZ0201 malformed-export reject (a self-defeating
        //     fix that fails `--verify-fixes` and never applies). Removing the clause leaves exactly the
        //     earlier `(export a)`.
        // Either way the public surface is unchanged (the name stays exported once), so the fix verifies.
        let delete_at = if names_per_clause.get(&clause_occ).copied().unwrap_or(1) > 1 {
            name_occ
        } else {
            clause_occ
        };
        faults.push(
            Reject::coded(
                Code::Malformed,
                format!("`{name}` is exported more than once (a module has a fixed set of names)"),
            )
            .at(name_occ)
            .with_fix(crate::diag::Fix::delete_heuristic(
                delete_at,
                format!("remove the duplicate export of `{name}`"),
            )),
        );
    }
    // DUPLICATE VARIANT. A sum type `(type T (A …) (A …))` declares its variant NAMES as a fixed SET
    // (core-semantics.md #The Structural Types Are Record, Tuple, And Sum: a sum's shape is its variant
    // names with their payload types), so naming a variant twice is the SAME duplicate-member
    // ill-formedness a record with a duplicate field, a module with a duplicate definition, and a
    // duplicate export are rejected for (CDZ0201) — the fourth closed name-set. Each variant after the
    // first with a given name (WITHIN one type declaration) is reported, anchored at its name
    // occurrence. (Two different types may reuse a variant name — the set is per-declaration.)
    // Collect (name, ty_name, name_occ) for each REDUNDANT variant first (the immutable `type_decls`
    // borrow), then resolve the delete-clause + push (which needs `&db`, a separate borrow). The clause is
    // the whole variant syntax to DELETE. A variant is written either as a BARE atom (`(type C A B)` — the
    // atom IS the clause, directly in the type form) or PARENTHESIZED (`(A)` / `(A Int64)` — the enclosing
    // list is the clause, so the fix removes the name AND any payloads). Distinguish by whether
    // `name_occ`'s parent is a list HEADED by `name_occ` (a `(name …)` wrapper) vs the `(type …)` form.
    let dup_variants: Vec<(String, String, StructId)> = db
        .type_decls
        .iter()
        .flat_map(|ty| {
            let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
            ty.variants
                .iter()
                .filter(move |v| !seen.insert(v.name.as_str()))
                .map(|v| (v.name.clone(), ty.name.clone(), v.name_occ))
        })
        .collect();
    for (name, ty_name, name_occ) in dup_variants {
        // The clause is `name_occ`'s parent when that parent is a `(name …)` wrapper (head == name_occ),
        // else `name_occ` itself (a bare-atom variant sitting directly in the `(type …)` form).
        let wraps_name = |parent: StructId| matches!(db.ast.get(parent), crate::ast::Struct::List(items) if items.first() == Some(&name_occ));
        let clause = match db.parent_of(name_occ) {
            Some(parent) if wraps_name(parent) => parent,
            _ => name_occ,
        };
        // Each variant after the first with a given name is a REDUNDANT declaration — delete it (the first
        // already binds the name; a sum's variant names are a fixed set). Anchor at the name occurrence.
        faults.push(
            Reject::coded(
                Code::Malformed,
                format!(
                    "variant `{name}` is declared more than once in sum `{ty_name}` (a sum has a \
                     fixed set of variant names)"
                ),
            )
            .at(name_occ)
            .with_fix(crate::diag::Fix::delete_heuristic(
                clause,
                format!("remove the duplicate `{name}` variant"),
            )),
        );
    }
    // DUPLICATE TYPE DECLARATION. A module's TYPE names are a fixed set exactly as its definition/export
    // names are: `(type T …) (type T …)` declares `T` twice, and the name resolves to the FIRST — so a
    // reference to a variant only the SECOND declares (`T.B` above) fails with a confusing "record has no
    // field `B`", the shadowed second declaration silently unreachable. That is the SAME closed-name-set
    // ill-formedness a duplicate def / export / variant / operation is rejected for (CDZ0201) — the sixth.
    // Reject each `(type …)` after the first with a given name, anchored at its declaration occurrence,
    // with a DELETE fix removing the redundant declaration (the first already binds the name — the same
    // repair the duplicate export/variant/op gets). A type whose reference the author actually meant to
    // point at the second declaration would RENAME it, but deleting the redundant same-named declaration
    // resolves the ambiguity in one shot; which the author meant is the heuristic. `ty.occ` is the whole
    // `(type …)` form (its nominal identity), so the fix removes the entire redundant declaration.
    // Only USER declarations participate: the built-in sums (`Option`/`Result`/…) are appended to
    // `type_decls` as ordinary entries but their `occ` is a SYNTHESIZED node (built after the user-node
    // snapshot), so `is_user_node` is false for them. This is exactly what lets a user `(type Option …)`
    // legitimately SHADOW the prelude sum (first-wins in `type_decl_by_name`) WITHOUT reading as a
    // duplicate: the prelude `Option` is filtered out, leaving the single user declaration.
    // The duplicate check is PER-MODULE (per-file), not global: a type-name set is fixed within ONE
    // module, but two SEPARATE modules of a linked package may each legitimately declare a type of the
    // same name (`(type L …)` in a lib AND in the importing entry — each module has its own type
    // namespace, and structural identity makes the two `L`s the same type). So key the seen-set on
    // `(file, name)`, using the same per-file identity the resolver scopes name visibility by (`file_of`;
    // `None` for a single-file program collapses to one bucket — the flat case is unchanged). Without the
    // file key, a global scan flagged a cross-module same-named type as a spurious duplicate (regressing
    // the cross-module recursive-sum case — modules-and-namespaces.md #Imports Are Explicit: a sibling
    // file's type is invisible unless imported, so re-declaring its name is not a redeclaration).
    let mut seen_types: std::collections::HashSet<(Option<usize>, &str)> =
        std::collections::HashSet::new();
    let dup_types: Vec<(String, StructId)> = db
        .type_decls
        .iter()
        .filter(|t| db.is_user_node(t.occ))
        .filter(|t| !t.name.is_empty() && !seen_types.insert((db.file_of(t.occ), t.name.as_str())))
        .map(|t| (t.name.clone(), t.occ))
        .collect();
    for (name, occ) in dup_types {
        faults.push(
            Reject::coded(
                Code::Malformed,
                format!(
                    "type `{name}` is declared more than once (a module has a fixed set of type names)"
                ),
            )
            .at(occ)
            .with_fix(crate::diag::Fix::delete_heuristic(
                occ,
                format!("remove the duplicate declaration of type `{name}`"),
            )),
        );
    }
    // DUPLICATE EFFECT OPERATION. An effect `(effect E (op f …) (op f …))` declares its operation NAMES
    // as a fixed SET (capabilities-and-effects.md §An Effect Declaration Names The Effect And Types Its
    // Operations: each name is bound to ONE operation type), so naming an operation twice is the SAME
    // duplicate-member ill-formedness a record field, a module definition, an export, and a sum variant
    // are rejected for (CDZ0201) — the fifth closed name-set. Each operation after the first with a given
    // name (WITHIN one effect declaration) is reported, anchored at its name occurrence. (Two different
    // effects may reuse an operation name — the set is per-declaration, since an operation is reached
    // through its declaring effect.)
    // Collect (name, eff_name, name_occ) for each redundant op first (immutable `effect_decls` borrow),
    // then resolve the clause + push. An op is ALWAYS parenthesized `(op NAME (-> …))`, so `name_occ` (the
    // NAME atom) is nested one level below the `(op …)` clause AND below the `op` head — the clause to
    // DELETE is the enclosing `(op …)` list, i.e. `parent_of(name_occ)` (whose head is `op`, not `name_occ`).
    let dup_ops: Vec<(String, String, StructId)> = db
        .effect_decls
        .iter()
        .flat_map(|eff| {
            let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
            eff.ops
                .iter()
                .filter(move |o| !seen.insert(o.name.as_str()))
                .map(|o| (o.name.clone(), eff.name.clone(), o.name_occ))
        })
        .collect();
    for (name, eff_name, name_occ) in dup_ops {
        let mut reject = Reject::coded(
            Code::Malformed,
            format!(
                "operation `{name}` is declared more than once in effect `{eff_name}` (an effect has \
                 a fixed set of operation names)"
            ),
        )
        .at(name_occ);
        // The `(op NAME …)` clause is `name_occ`'s parent (a list headed by `op`). Delete it.
        if let Some(clause) = db.parent_of(name_occ)
            && matches!(db.ast.get(clause), crate::ast::Struct::List(_))
        {
            reject = reject.with_fix(crate::diag::Fix::delete_heuristic(
                clause,
                format!("remove the duplicate `{name}` operation"),
            ));
        }
        faults.push(reject);
    }
    // Validate every definition's PARAMETER ANNOTATIONS — a garbage type in a `(: name T)` parameter
    // (an unbound name, a value, a malformed type application) is rejected, not silently typed `Any`.
    // The signature-side companion of the value-annotation check in `infer::collect_node`; walked here
    // because a signature parameter is not part of the def's body (which `type_errors` walks). Collected
    // per param across ALL defs so a garbage parameter type is caught whether or not the def is called.
    let all_params: Vec<StructId> = db.defs.iter().flat_map(|d| d.params.clone()).collect();
    for p in &all_params {
        crate::infer::param_annotation_faults(db, *p, &mut faults);
    }
    // Validate every VARIANT POSITION's SHAPE. A `(type …)` declaration's tail is its variants: a bare
    // NAME is a nullary variant (`Red`), a `(Name payload…)` LIST is a variant with payloads. `scan_type_decl`
    // SILENTLY DROPS any tail element that is neither — a literal `(type T 5)`, a list headed by a non-name
    // `(type T (5 Int64))`, an empty `()` — so `(type T Red 5 Blue)` becomes the two-variant `{Red, Blue}`
    // with the `5` invisibly gone, and a match on `Red`/`Blue` then wrongly type-checks as EXHAUSTIVE (a
    // silent correctness hazard). Reject a malformed variant position at the declaration (CDZ0201): a
    // variant is a name or a `(Name …)` form. Walked over the raw `(type …)` AST tail (the scanned
    // `variants` already dropped the bad ones), for USER type declarations only.
    let type_decl_occs: Vec<StructId> = db
        .type_decls
        .iter()
        .filter(|d| db.is_user_node(d.occ))
        .map(|d| d.occ)
        .collect();
    for occ in type_decl_occs {
        let Some(tail) = db.ast.as_form(occ, "type").map(|t| t.to_vec()) else {
            continue;
        };
        // tail[0] is the type NAME; tail[1..] are the variant positions.
        for &v in tail.iter().skip(1) {
            let well_formed = match db.ast.get(v) {
                // A bare NAME is a nullary variant; a bare literal (`5`, `"x"`) is not.
                crate::ast::Struct::Atom(_) => db.ast.as_name(v).is_some(),
                // A `(Name payload…)` variant — the head must be a NAME; `()` / `(5 …)` is malformed.
                crate::ast::Struct::List(children) => children
                    .first()
                    .is_some_and(|&h| db.ast.as_name(h).is_some()),
            };
            if !well_formed {
                faults.push(
                    Reject::coded(
                        Code::Malformed,
                        "a variant must be a name (a nullary variant `Red`) or a `(Name payload…)` form \
                         — this is neither, so it is not a variant of the type",
                    )
                    .at(v),
                );
            }
        }
    }
    // Validate every VARIANT PAYLOAD TYPE. A garbage type in a variant payload — `(type C (A Nonesuch))`,
    // `(type C (A (List Nonesuch)))` — was silently accepted: the unknown name resolved to nothing and the
    // variant was mis-typed (`A` treated as NULLARY, its payload dropped — `(C.A x)` then reports "a
    // nullary variant takes the unit value"), a correctness gap. This is the declaration-site companion of
    // the parameter/value-annotation type check: a payload type position REQUIRES a real type, exactly as
    // an annotation does. The resolver already handles self-recursion (`(Cons Int64 T)`), mutual/forward
    // references, and generic self-application (`(Node (Tuple Tree Tree))`) — those resolve to real types,
    // so only a genuinely UNKNOWN name faults. A bare type-PARAMETER name (`a` in `(Some a)`, recorded in
    // the decl's `params`) is a valid payload and is skipped — it is NOT a global type. Collected per
    // payload across ALL type declarations so a garbage payload type is caught whether or not constructed.
    // Each variant payload's TYPE-EXPRESSION POSITIONS to validate, paired with the declaration's type
    // params. A `(Record (field Type)…)` payload contributes each FIELD's Type — NOT the whole record form
    // or the field NAMES (a record type's field names are labels, not global types; validating the whole
    // form out-of-context mis-resolves them as unbound). So an unknown type INSIDE a record payload
    // (`(Record (val Nonesuch))`) IS caught, at the field-type position, with no field-name false positive.
    // The record-aware descent mirrors `db::collect_type_params` (which collects the params from the same
    // positions). A non-record payload validates as a single position (the whole payload expression).
    let mut type_positions: Vec<(StructId, Vec<String>)> = Vec::new();
    for d in &db.type_decls {
        let params = &d.params;
        for v in &d.variants {
            for &payload in &v.payloads {
                push_payload_type_positions(db, payload, params, &mut type_positions);
            }
        }
    }
    for (payload, params) in &type_positions {
        validate_type_position(db, *payload, params, "a variant payload", &mut faults);
    }
    // Validate every EFFECT OPERATION's declared TYPE — `(op e (-> ArgT ResultT))`. An unknown type in an
    // operation's arg/result (`(op e (-> Nonesuch Unit))`) was silently accepted, exactly as a variant
    // payload was: the name resolved to nothing and the op's `(meta t)` arrow was corrupted to `Any`, so
    // performing it reported a garbled "cannot apply a value of type (Record (apply Any) …)". The op TYPE
    // is a `(-> A B …)` arrow — every element past the `->` head is a type position; validate each with the
    // same record-aware walk. Effect ops carry no declared type params (an empty param set), so the walk's
    // lowercase-name leniency (a bare lowercase name is a type-variable artifact, not a global type) covers
    // a `(-> a a)`-style variable.
    let mut op_type_positions: Vec<(StructId, Vec<String>)> = Vec::new();
    // Op types that are well-formed TYPES but NOT arrows — `(op get Int64)` / `(op get (Option Int64))`.
    // An operation is PERFORMED (a function call), so its type MUST be an arrow `(-> Arg… Result)`; a
    // canonical nullary op is `(-> Result)`. A bare type was silently accepted, wrapped as `(fn () Int64)`,
    // then LEAKED the internal op-value record on perform ("type mismatch: Int64 and (Record (apply Any)
    // …)") — a non-canonical spelling that garbles downstream, so reject it AT THE DECLARATION with the
    // wrap fix (`garbage render = not canonical → fix the source`). Collected separately so the arrow-arm
    // type-position walk above stays the sole caller of `validate_type_position` for op types.
    // A NAMELESS operation. Each op clause is `(op NAME TYPE)`; its NAME must be a bare name. `(op (-> Unit
    // Int64))` has a TYPE where the name should be — `scan_effect_decl` recorded it with an empty name and a
    // `name_occ` pointing at the non-name element, silently registering a nameless operation (unreachable —
    // `E.` has nothing to project). Reject it CDZ0201 at the offending element: an operation must be named,
    // like a def or a variant. (Collected from `name_occ`, which is the op clause's first element.)
    let nameless_ops: Vec<StructId> = db
        .effect_decls
        .iter()
        .flat_map(|e| e.ops.iter())
        .filter(|op| db.ast.as_name(op.name_occ).is_none())
        .map(|op| op.name_occ)
        .collect();
    for occ in nameless_ops {
        faults.push(
            Reject::coded(
                Code::Malformed,
                "an effect operation must be named — an operation is `(op <name> (-> Arg… Result))`, \
                 e.g. `(op emit (-> Int64 Unit))`",
            )
            .at(occ),
        );
    }
    // A `handle` HEAD THAT NAMES A VALUE. `(handle foo 0 …)` where `foo` is a `(def foo …)` value — the head
    // must name an EFFECT (the arms ARE that effect's operations). The desugar folds the head into each
    // arm's `(. foo op)` projection, so a value head surfaces only as a LEAKY cascade — "member access
    // requires a record, found Int64" (from `(. foo op)` where foo is Int64) plus an uncoded fold-decline —
    // never naming the real problem. Scan each handle's arms; if an arm op's head names a value def
    // (`arm_op_head_names_a_value`, conservative — never a nested-module effect / unbound name), reject
    // CDZ0201 naming the head, and DROP the cascade faults anchored inside this handle so the report names
    // the root once. Collected first (a `&mut db` walk) to keep the borrow simple.
    let mut bad_handle_heads: Vec<(StructId, String)> = Vec::new();
    for id in (0..db.ast.structure.len() as u32).map(StructId) {
        if db.ast.head_name(id) != Some(crate::effects::HANDLE_INTERNAL) {
            continue;
        }
        // The arms list is the internal handle's 2nd tail element; each arm's op is its first child.
        let Some(tail) = db
            .ast
            .as_form(id, crate::effects::HANDLE_INTERNAL)
            .map(<[_]>::to_vec)
        else {
            continue;
        };
        let Some(&arms_occ) = tail.get(1) else {
            continue;
        };
        let Some(arm_ops): Option<Vec<StructId>> = (match db.ast.get(arms_occ) {
            crate::ast::Struct::List(arms) => Some(
                arms.iter()
                    .filter_map(|&a| match db.ast.get(a) {
                        crate::ast::Struct::List(parts) => parts.first().copied(),
                        _ => None,
                    })
                    .collect(),
            ),
            _ => None,
        }) else {
            continue;
        };
        for op in arm_ops {
            if let Some((head_occ, category)) = crate::effects::arm_op_head_names_a_value(db, op) {
                let named = db
                    .ast
                    .as_name(head_occ)
                    .map(|n| format!(" `{n}`"))
                    .unwrap_or_default();
                bad_handle_heads.push((
                    head_occ,
                    format!(
                        "a handle's head must name an EFFECT, but this head{named} is {category} \
                         — write `(handle <effect> <seed> (arms…) <body>)` over a declared \
                         `(effect …)`"
                    ),
                ));
                break; // one diagnostic per handle (all arms share the head)
            }
        }
    }
    for (head_occ, msg) in bad_handle_heads {
        faults.push(Reject::coded(Code::Malformed, msg).at(head_occ));
    }
    let mut non_arrow_op_types: Vec<StructId> = Vec::new();
    for e in &db.effect_decls {
        for op in &e.ops {
            let Some(ty) = op.ty else { continue };
            match db.ast.get(ty) {
                // `(-> A B …)` — each element after the arrow head is a type position (record-split).
                crate::ast::Struct::List(children)
                    if children.first().and_then(|&h| db.ast.as_name(h)) == Some("->") =>
                {
                    for &pos in children.iter().skip(1) {
                        push_payload_type_positions(db, pos, &[], &mut op_type_positions);
                    }
                }
                // A non-arrow op type (a bare type / malformed). Validate it as a type position FIRST: a
                // genuinely-unknown name (`(op get Nonesuch)`) keeps its CDZ0101 (more actionable — wrapping
                // an unknown name in `(-> …)` would not resolve it). A WELL-FORMED non-arrow type
                // (`Int64`, `(Option Int64)`) is the malformed-op-type case handled below.
                _ => {
                    op_type_positions.push((ty, Vec::new()));
                    non_arrow_op_types.push(ty);
                }
            }
        }
    }
    for (pos, params) in &op_type_positions {
        validate_type_position(db, *pos, params, "an operation type", &mut faults);
    }
    // An op type that is a WELL-FORMED type but not an arrow: reject it (unless `validate_type_position`
    // already faulted it — an unknown name / non-type — in which case that reject stands and this adds no
    // second "no" for the same op type). The fix wraps it into the canonical nullary arrow `(-> T)`.
    for &ty in &non_arrow_op_types {
        let already_faulted = faults.iter().any(|f| f.at == Some(ty));
        if already_faulted {
            continue;
        }
        if crate::eval::typeval_of(db, ty).is_some() {
            faults.push(
                Reject::coded(
                    Code::Malformed,
                    "an operation's type must be an arrow `(-> Arg… Result)` — an operation is performed \
                     like a function; a nullary operation is `(-> Result)`",
                )
                .at(ty)
                .with_fix(crate::diag::Fix::wrap_heuristic(
                    ty,
                    "(-> ",
                    ")",
                    "make it a nullary operation arrow",
                )),
            );
        }
    }
    // DUPLICATE PARAMETER NAME. A function's parameter list is a BINDER POSITION, so it must be LINEAR
    // exactly as a pattern is (core-semantics.md §Patterns Compose: "A pattern MUST bind each name at
    // most once … rather than silently shadowing an earlier binder"). `(def (f x x) …)` binds `x` twice;
    // accepting it last-wins makes the FIRST parameter — and any argument passed to it — silently
    // unreachable (its value, and any trap it would raise, dropped). Reject the SECOND+ occurrence of a
    // name (CDZ0102, the non-linear-binder code the spec assigns), anchored at the repeated binder. Per
    // def (a name may of course repeat ACROSS defs); the binder NAME sees through a `(: name T)` binder.
    // MALFORMED PARAMETER POSITION. A parameter is a binder: a bare NAME (`x`), a wildcard `_`, an
    // annotated `(: name T)`, or a destructuring PATTERN (a compound list — tuple/record/ctor, validated
    // by the binding-pattern path). A bare LITERAL (`(def (f 5) …)`, `(def (f true) …)`) is NONE of these
    // — it binds nothing, so the parameter is dead and any argument passed to it is silently ignored. It
    // was accepted with no diagnostic (the scan reads `children[1..]` without validating each is a binder,
    // and `param_name_occ` just returns the literal node). Reject a bare-atom parameter that is not a name
    // (CDZ0201): a parameter must name something. A COMPOUND (list) parameter is a destructuring pattern —
    // left to the binding-pattern path, which rejects a refutable/ill-formed one with its own coded fault.
    let malformed_params: Vec<StructId> = db
        .defs
        .iter()
        .flat_map(|d| d.params.clone())
        .filter(|&p| {
            matches!(db.ast.get(p), crate::ast::Struct::Atom(_)) && db.ast.as_name(p).is_none()
        })
        .collect();
    for p in malformed_params {
        faults.push(
            Reject::coded(
                Code::Malformed,
                "a parameter must be a name, a `(: name Type)` binder, or a destructuring pattern — a \
                 literal binds nothing",
            )
            .at(p),
        );
    }
    let param_lists: Vec<Vec<StructId>> = db.defs.iter().map(|d| d.params.clone()).collect();
    for params in &param_lists {
        // All param names of this list — the set the rename fix must avoid so a fresh name collides with
        // neither an earlier NOR a later parameter (renaming `x` in `(f x x)` to `x2` must dodge a real `x2`).
        let all_names: std::collections::HashSet<String> = params
            .iter()
            .filter_map(|&p| {
                db.ast
                    .as_name(crate::eval::param_name_occ(db, p))
                    .map(str::to_string)
            })
            .collect();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for &p in params {
            let name_occ = crate::eval::param_name_occ(db, p);
            let Some(name) = db.ast.as_name(name_occ).map(|s| s.to_string()) else {
                continue; // a param with no extractable name (a malformed binder) — not a dup check
            };
            if !seen.insert(name.clone()) {
                // RENAME the repeated occurrence to a fresh non-colliding name (`x` → `x2`), making the
                // parameter list linear (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route
                // To A Fix). Heuristic: the rename clears the hard error, but the fresh binder is then
                // unused (a CDZ0306 warning) until the author wires it up — renaming vs. dropping the
                // duplicate (which changes arity) is the author's call. Anchored at the repeated binder.
                let fresh = crate::diag::suggest::fresh_suffixed_name(&name, &all_names);
                faults.push(
                    Reject::coded(
                        Code::NonLinearBinder,
                        format!(
                            "parameter `{name}` is bound more than once (a parameter list must be \
                             linear, like a pattern)"
                        ),
                    )
                    .at(name_occ)
                    .with_fix(crate::diag::Fix::replace_heuristic(name_occ, fresh)),
                );
            }
        }
    }
    // Check EVERY definition's body — reachable or not. (The demand is still lazy per node; this just
    // demands each definition once, which is what well-formedness requires.)
    // A def is an ENTRYPOINT if it is exported — the only context where a nullary body is lowered
    // STANDALONE as the emitted artifact. A non-exported nullary def is always inlined at its call sites
    // (or dead), so its standalone lowering is not what ships; its reached-poison walk would fault on a
    // decline that the inline at the call site resolves (e.g. a library def that performs an effect whose
    // home is its caller's handler). So run the reached-poison walk only on EXPORTED nullary bodies.
    let exported_bodies: std::collections::HashSet<StructId> = db
        .exports
        .iter()
        .filter_map(|e| e.def.and_then(|d| db.defs[d].body))
        .collect();
    // A SELF-REFERENTIAL VALUE DEFINITION — `(def (g) g)`, or a mutual cycle `(def (a) b) (def (b) a)` —
    // is a value defined in terms of itself with no base case: it names nothing (`g = g`), and the
    // reduction spins until the depth guard fires, mislabeling it "expression nests too deeply (a resource
    // limit)". Detect the `Ref` cycle STRUCTURALLY (no reduction) and reject it CDZ0201 with the real
    // cause — a value cannot be defined in terms of itself — so the message says what is wrong + how to
    // fix it (give it a base value / break the cycle), not a misleading resource-limit decline. Only a
    // NULLARY def (a value; a def WITH params is a function, whose self-reference is legitimate recursion
    // lowered to a `Core::Call`). Checked here, before the body walk, so the clear reject precedes the
    // depth-limit decline. `value_ref_cycle` reports only a bare-`Ref` cycle (never a computing body or a
    // recursive function), so it is false-alarm-free.
    let value_cycles: Vec<(String, StructId, StructId)> = db
        .defs
        .iter()
        .filter(|d| !d.internal && d.params.is_empty())
        .filter_map(|d| d.body.map(|b| (d.name.clone(), b, d.sig_occ)))
        .collect();
    // The body nodes proven a value cycle — SKIPPED by the body-check loop below (their `type_errors` /
    // reached-poison reduction would spin into the depth guard, deep-recursing on a smaller stack; the
    // cycle reject already names the fault, so there is nothing more to learn from reducing them).
    let mut cyclic_bodies: std::collections::HashSet<StructId> = std::collections::HashSet::new();
    for (name, body, sig_occ) in value_cycles {
        if crate::resolve::value_ref_cycle(db, body) {
            cyclic_bodies.insert(body);
            faults.push(
                Reject::coded(
                    Code::Malformed,
                    format!(
                        "`{name}` is defined in terms of itself with no base value — a value definition \
                         cannot reference itself (give it a concrete value, or make it a function if the \
                         recursion is intended)"
                    ),
                )
                .at(sig_occ),
            );
        }
    }
    let bodies: Vec<(StructId, bool)> = db
        .defs
        .iter()
        .filter(|d| d.body.is_none_or(|b| !cyclic_bodies.contains(&b)))
        .filter_map(|d| {
            d.body
                .map(|b| (b, d.params.is_empty() && exported_bodies.contains(&b)))
        })
        .collect();
    for (body, nullary) in bodies {
        // TYPE CHECKING FIRST, then the reached-poison (lowering) walk — the safety ordering
        // (`reference-compiler.md` §Outcomes Are Ordered By Safety). A CODED rejection (an ill-typed
        // program, CDZ####) is a stronger, more actionable "no" than an uncoded DECLINE (a construct
        // the compiler does not yet lower), so when a body is BOTH ill-typed and not-yet-lowerable
        // (e.g. `(< 1 true)` — a type mismatch whose lowering would also decline as "runtime
        // comparison"), the rejection is reported, not the decline. Collecting type faults before the
        // lowering walk puts the rejection ahead of the decline in the fault list.
        //
        // `type_errors` applies to EVERY body — a function body's free parameters are bound (a `Param`
        // types fine), so an unbound name or type fault in it is still caught. The reached-POISON walk
        // lowers the body, which only makes sense for a VALUE: a FUNCTION body (a def with parameters)
        // is not lowered standalone — its params are unsubstituted until it is applied — so run the
        // trap walk only on a nullary def's body. A function body's traps surface at its call site.
        faults.extend(type_errors(db, body));
        if nullary {
            collect_reached_poisons(db, body, &mut faults);
        }
    }
    // ENTRYPOINT NO-HOME CHECK (CDZ0401). An effect operation reached from an ENTRYPOINT with neither an
    // enclosing handler nor a host delegation escapes ungranted — the merged "no home" check
    // (`capabilities-and-effects.md` §An Ungranted Effect Is A Compile-Time Error). This is an
    // ENTRYPOINT-level property (a library def that performs an effect is fine — its home is its callers'
    // context), so it is checked over each EXPORT's body, following the call graph, rather than per-def.
    // A perform enclosed by a `handle` that discharges its effect, or a `host` that delegates it, has a
    // home and is skipped; one that reaches the entrypoint top with no home is CDZ0401. This is the
    // no-ambient-authority floor: a program that reaches a host operation it never declared is REJECTED
    // here rather than compiled to a component carrying that undeclared (latent) import.
    //
    // Delegation is an ENTRYPOINT's prerogative: this check (and the `host` delegation it honors) is
    // scoped to each EXPORT's body — a library def that performs an effect is fine, its home being its
    // callers' context — so authority enters a program only from the top and no interior function routes
    // an effect to the boundary, keeping "no ambient authority" transitive.
    //= spec/capabilities/capabilities-and-effects.md#host-delegation-is-an-entrypoint-s-prerogative
    //# Only an entrypoint MUST be able to delegate an effect to the host, so that authority enters a program from the top and no interior function can route an effect to the boundary, keeping "no ambient authority" transitive: a library performs and handles effects but never grants host access.
    //= constitution.md#iv-no-ambient-authority
    //# A program that reaches a host operation it does not declare MUST be rejected at compile time rather than compiled to a component carrying a latent import.
    let export_bodies: Vec<StructId> = db
        .exports
        .iter()
        .filter_map(|e| e.def.and_then(|d| db.defs[d].body))
        .collect();
    for body in export_bodies.iter().copied() {
        crate::effects::check_no_home(db, body, &mut faults);
    }
    // STRAY `resume`. A `resume` hands a value back to the point that performed a handler arm's operation,
    // so it is meaningful ONLY inside a handler arm's BODY. A `resume` anywhere else — a top-level def body,
    // a plain expression — has no arm to return into: it is a malformed use of the control form, not a
    // not-yet-supported gap. Without this check a stray `resume` resolved to a valid `Resolved::Resume`,
    // type-checked leniently (a resume is `Ty::Any`), and only DECLINED at lowering with NO coded
    // diagnostic — so `cdz check` reported nothing and only the backend refused it (a `check`≡`compile`
    // gap). Reject each stray `resume` with a coded CDZ0201 (Malformed — it is structurally well-formed but
    // in an invalid position), anchored at the `resume` occurrence.
    for id in (0..db.ast.structure.len() as u32).map(StructId) {
        if db.ast.head_name(id) == Some("resume") && crate::resolve::is_stray_resume(db, id) {
            faults.push(
                Reject::coded(
                    Code::Malformed,
                    "a `resume` is only meaningful inside a handler arm's body — this one has no \
                     enclosing handler arm to resume into",
                )
                .at(id),
            );
        }
    }
    // EXPORTED-CLOSURE BODY TYPE-CHECK. An export whose body is a bare `(fn …)` crosses the host boundary
    // as a closure — it is NEVER applied in-guest, so its body is never β-reduced, and `collect_node`'s
    // `Resolved::Lambda` arm is a no-op (a called def's body is checked on β-reduction at its call site,
    // so the arm deliberately does not descend). That leaves an EXPORTED closure's body UNCHECKED: an
    // ill-typed body (`(fn ((: x Int64)) (: x Bool))`) escaped the type-checker and emitted an invalid
    // component, where an ordinary def / an in-guest-applied `(fn …)` rejects CDZ0203. Run the same
    // `type_errors` over the closure body here — its params are bound (a `(: x Int64)` param types as
    // Int64), so an annotation/unification fault in the body surfaces exactly as it does for an ordinary
    // definition. (SUBSUMES the narrow-arg-wide-result invalid-component case: `(fn ((: x Int8)) (: (+ x
    // 100) Int64))`'s body is ill-typed — the `(+ x 100)` is Int8, not Int64 — so it rejects CDZ0203
    // instead of emitting invalid wasm.) A non-lambda export body is already fully checked above.
    for body in export_bodies {
        if let crate::resolved::Resolved::Lambda {
            body: closure_body, ..
        } = crate::resolve::resolved_of(db, body)
        {
            faults.extend(type_errors(db, closure_body));
        }
    }
    // AN EXPORT NAMING NO DEFINITION is ill-formed — `(export nope)` with no `(def nope …)`. This is a
    // well-formedness fault (the public surface must name real definitions), so it belongs in
    // `collect_faults` where BOTH `cdz check` and `compile` see it — not only in the emit-path layout
    // (which `compile` runs but `check`'s Diagnostics query does not, so `check` used to MISS it). Coded
    // CDZ0101 (an export names an unbound definition — the export-position analogue of an unbound
    // reference), anchored at the `(export …)` clause, with a "did you mean?" over the defined names.
    let defined_names: Vec<String> = db.defs.iter().map(|d| d.name.clone()).collect();
    // Capture the NAME occurrence (the `(export NAME)` clause's first element), not just the clause, so a
    // "did you mean?" can attach a REPLACE fix on the name atom — the same closed-set repair the pragma-key
    // typo carries above. `occ` is the clause (where the error anchors); `name_occ` is the atom the fix edits.
    let missing_exports: Vec<(String, StructId, Option<StructId>)> = db
        .exports
        .iter()
        .filter(|e| e.def.is_none())
        // `name_occ` is the specific exported-name atom — correct even for the 2nd+ name of a multi-name
        // `(export a b)` clause, where reading the clause's `tail.first()` would mis-anchor to `a`.
        .map(|e| (e.name.clone(), e.occ, Some(e.name_occ)))
        .collect();
    for (name, occ, name_occ) in missing_exports {
        match crate::diag::suggest::nearest(&name, &defined_names) {
            // A near-miss for a defined name — almost always a typo (`computee` for `compute`). Name it AND
            // carry the concrete repair: REPLACE the export's name atom with the real definition's name. The
            // candidate pool is the defined names, so the suggestion can never name a def the check would
            // then reject. Heuristic — the nearest-name match is a guess at intent, not a proof.
            Some(near) => {
                let mut reject = Reject::coded(
                    Code::Unbound,
                    format!("export `{name}` names no definition — did you mean `{near}`?"),
                )
                .at(occ);
                if let Some(at) = name_occ {
                    reject = reject.with_fix(crate::diag::Fix::replace_heuristic(at, near));
                }
                faults.push(reject);
            }
            None => {
                // No near-miss DEFINITION. But the name may be a declared TYPE or EFFECT — a real
                // declaration, just not an exportable one: a module's exports are its DEFINITIONS' values
                // (core-semantics.md §Evaluating a module produces a record of its definitions' values), and
                // a type/effect is not a value. Saying "names no definition" misleads (the name IS
                // declared); name the real reason instead, so the author knows the export is not a typo but
                // a category error (there is no mechanical fix — removing the export or defining a value of
                // that name is the author's choice).
                let kind = if db.type_decl_by_name(&name).is_some() {
                    Some("a type")
                } else if db.effect_decl_by_name(&name).is_some() {
                    Some("an effect")
                } else {
                    None
                };
                let message = match kind {
                    Some(k) => format!(
                        "export `{name}` names {k}, not a value definition — only definitions are exported \
                         (a module's exports are the values its definitions bind)"
                    ),
                    None => format!("export `{name}` names no definition"),
                };
                faults.push(Reject::coded(Code::Unbound, message).at(occ));
            }
        }
    }
    // A MALFORMED EXPORT CLAUSE — `(export (g x))`, `(export 5)`, `(export)` — whose argument is not a bare
    // name. The module scan only registers an `Export` when the argument `as_name`s, so a malformed clause
    // is otherwise SILENTLY DROPPED (no export recorded, `unknown_top_forms` skips it since its head is the
    // known `export`), and the program compiles as if the author never wrote the export — the emit path
    // then reports a misleading "nothing is public". Reject it here (CDZ0201) so BOTH `check` and `compile`
    // name the real fault + how to fix it: an export names a single definition, `(export g)`. A scan-and-
    // drop correctness hazard closed at the chokepoint. (A WELL-FORMED `(export g)` naming a missing/typo'd
    // or non-value def is handled above as CDZ0101 — this is only the STRUCTURALLY malformed argument.)
    for (occ, bad_arg) in db.malformed_exports() {
        // Anchor at the BAD ELEMENT when there is one (so a `(export a 5)` points at `5`, not the whole
        // clause); fall back to the clause for an empty `(export)`.
        let anchor = bad_arg.unwrap_or(occ);
        let mut reject = Reject::coded(
            Code::Malformed,
            "an export names a definition — write `(export <name>)`, e.g. `(export main)` \
             (an export clause is one or more bare definition names)"
                .to_string(),
        )
        .at(anchor);
        // When the bad element is a compound whose HEAD is a name — `(export (g x))` — the author most
        // likely meant to export `g`; offer replacing that element with the bare `<head>`. A non-name
        // atom (`(export 5)`, the `5` in `(export a 5)`) or an empty `(export)` has no name to recover →
        // message only (the author drops or replaces it).
        if let Some(arg) = bad_arg
            && let Some(head) = db.ast.head_name(arg)
        {
            reject = reject.with_fix(crate::diag::Fix::replace_heuristic(arg, head.to_string()));
        }
        faults.push(reject);
    }
    // AN EXPORT WHOSE RESULT IS A NON-REPRESENTABLE CLOSURE — e.g. an entrypoint returning a PARTIAL
    // APPLICATION `(f 1)` for a two-parameter `f`, whose residual parameter type inference never fixed
    // (`Any`) — cannot cross the component boundary. The backend rejects it deep in closure-resource
    // emit (an uncoded decline `cdz check`'s Diagnostics query never runs), so `check` used to accept it
    // while `compile` failed. Detect it here from the export's SOLVED result type (a `Ty::Fn` whose
    // parameter or result has no `abi_val_type`) so BOTH surfaces report it, coded CDZ0201 (ill-formed:
    // the public surface is not boundary-representable), anchored at the export clause. A REPRESENTABLE
    // closure export (`(-> Int64 Int64)` — the C-HOST feature) is fine and NOT flagged.
    // An UNCONSTRAINED (`Any`) parameter/result in an exported closure's type — inference never fixed
    // it, the partial-application / unannotated-closure case. This is the genuinely-unrepresentable
    // signal, NARROWER than "not host-ABI-representable": the closure boundary supports every aliased
    // scalar width (Float32, Int8/16/…), which `abi_val_type` (the host-CALL table) does NOT model, so
    // keying on `abi_val_type` would over-reject a REPRESENTABLE closure export (the C-HOST feature). A
    // concrete-but-unrepresentable component is the backend's own decline, not this well-formedness fault.
    fn arrow_has_unconstrained(ty: &crate::ty::Ty) -> bool {
        match ty {
            crate::ty::Ty::Fn(p, r) => {
                matches!(p.as_ref(), crate::ty::Ty::Any) || arrow_has_unconstrained(r)
            }
            _ => false,
        }
    }
    // Collect (body, name, occ) FIRST (immutable borrow), then read each body's type with `&mut db`.
    let export_results: Vec<(StructId, String, StructId)> = db
        .exports
        .iter()
        .filter_map(|e| {
            let body = e.def.and_then(|d| db.defs[d].body)?;
            Some((body, e.name.clone(), e.occ))
        })
        .collect();
    for (body, name, occ) in export_results {
        let ty = crate::infer::type_of(db, body);
        // A TYPE-VALUED export — `(def (main) Int64)` exports a bare type name. A type is a COMPILE-TIME
        // value with no runtime form (the erasure fence), so it cannot be an entrypoint's result. The emit
        // path declines this through FOUR different downstream paths (type-value-has-no-runtime-form,
        // nullary-lambda-no-closure, closure-param-no-repr, built-in-op-as-value) — a 4-error cascade for
        // one root cause. Report it ONCE here, coded CDZ0201 at the export clause with a clear message;
        // `dedup_faults` drops the downstream declines. `Ty::Type` is the type of a type-value — the
        // authoritative signal (an ordinary runtime value never has it).
        if matches!(ty, crate::ty::Ty::Type) {
            faults.push(
                Reject::coded(
                    Code::Malformed,
                    format!(
                        "export `{name}` is a TYPE, not a runtime value — a type is compile-time only \
                         and cannot cross the component boundary (export a value of the type, or a \
                         function, not the type itself)"
                    ),
                )
                .at(occ),
            );
            continue;
        }
        // A type-value NESTED in a compound result — `(def (main) (tuple Int64 5))` returns `(Tuple Type
        // Int64)`. A type-value is compile-time-only (`type-system.md §226`: a type-value never flows from
        // runtime data), so a compound CARRYING one cannot cross the boundary either. Without this the emit
        // path declines through the same uncoded no-runtime-form cascade with no coded, actionable reject.
        // Report ONE coded CDZ0201 here; the message embeds `TYPE_EXPORT_MARKER` ("is a TYPE, not a runtime
        // value") so `dedup_faults` drops the downstream declines exactly as for the bare type-valued
        // export. The bare `Ty::Type` case above is handled first, so this is only a NESTED occurrence.
        if ty.has_type_value() {
            faults.push(
                Reject::coded(
                    Code::Malformed,
                    format!(
                        "export `{name}`: a {} is a TYPE, not a runtime value — the {} carries a type, \
                         which is compile-time only and cannot cross the component boundary (a type-value \
                         never flows into runtime data; store a value of the type, not the type itself)",
                        ty.render_name(),
                        ty.render_name()
                    ),
                )
                .at(occ),
            );
            continue;
        }
        // An EFFECT-VALUED export — `(def (main) E)` exports a bare effect name. An effect is not a runtime
        // value; its body's type is the effect's SYNTHESIZED record, so evaluating it leaked a 4-error
        // cascade of internal errors ("unknown intrinsic", unbound `effect-op`/`effect`, nullary-lambda-no-
        // closure) — the effect analogue of the type-valued export cascade above. Detect it by the body
        // being a bare name that `effect_decl_by_name` resolves (the same category check the M74 export-an-
        // effect and M75 apply-an-effect messages use), report ONE clean coded reject naming the category
        // (carrying `TYPE_EXPORT_MARKER` so `dedup_faults` drops the downstream leaky declines the same way
        // it does for a type-valued export), and skip the downstream checks.
        if db
            .ast
            .as_name(body)
            .is_some_and(|n| db.effect_decl_by_name(n).is_some())
        {
            faults.push(
                Reject::coded(
                    Code::Malformed,
                    format!(
                        "export `{name}` is an effect, not a runtime value — an effect is a capability \
                         an operation performs, not a value that crosses the component boundary; export a \
                         function that performs the effect, not the effect itself"
                    ),
                )
                .at(occ),
            );
            continue;
        }
        if arrow_has_unconstrained(&ty) {
            faults.push(
                Reject::coded(
                    Code::Malformed,
                    format!(
                        "export `{name}` returns a closure that cannot cross the component boundary: its \
                         type `{}` has a parameter inference never fixed to a concrete scalar (a partial \
                         application like `(f 1)` for a two-parameter `f`, or an unannotated closure \
                         parameter) — a closure crosses only with concrete aliased-width scalar arguments",
                        ty.render_name()
                    ),
                )
                .at(occ),
            );
        }
    }
    // AN EXPORTED DEFINITION WITH AN UNANNOTATED (AMBIGUOUS-TYPE) PARAMETER — `(def (f x) …)` exported.
    // An exported parameter must have a concrete machine type to cross the boundary; an unannotated one
    // whose inference never fixed a scalar (`Any`, no `valtype`) has none. The EMIT path declines this in
    // `layout::export_params` (an uncoded decline `cdz check` never runs, so `check` used to accept it
    // while `compile` failed — the check-vs-emit gap). Detect it HERE so BOTH surfaces report it, coded
    // CDZ0201 anchored at the parameter, WITH the rustc-gold "add a type annotation" fix: WRAP the bare
    // param `x` in `(: x Int64)` (`Int64` = the numeric-model default; heuristic — the annotation resolves
    // the ambiguity but the concrete type is the author's to confirm). Only a BARE param binder is
    // flagged/fixed; an already-annotated `(: x T)` whose T is non-representable is a different fault.
    let export_param_defs: Vec<(usize, String)> = db
        .exports
        .iter()
        .filter_map(|e| e.def.map(|d| (d, e.name.clone())))
        .collect();
    for (def, name) in export_param_defs {
        let params = db.defs[def].params.clone();
        for p in params {
            // Skip an already-annotated binder `(: a T)` — this fault is the BARE, unannotated param.
            if db.ast.as_form(p, ":").is_some() {
                continue;
            }
            let ty = crate::infer::type_of(db, p);
            if crate::backend::wasm::lir::valtype_of(&ty).is_none() {
                let mut reject = Reject::coded(
                    Code::Malformed,
                    format!(
                        "export `{name}`: parameter type is ambiguous — annotate it (a boundary \
                         parameter needs a concrete type; inference never fixed one)"
                    ),
                )
                .at(p);
                if let Some(nm) = db.ast.as_name(p) {
                    reject = reject.with_fix(crate::diag::Fix::wrap_heuristic(
                        p,
                        "(: ",
                        " Int64)",
                        format!("annotate `{nm}` with a type, e.g. `(: {nm} Int64)`"),
                    ));
                }
                faults.push(reject);
            }
        }
    }
    // SANITIZE ORIGINS BEFORE DEDUP. A fault anchored at a SYNTHESIZED/non-user node has its origin
    // stripped to `None` (the front-end span table only covers user nodes). This must run BEFORE
    // `dedup_faults` so the SAME fault reported once at a user node and once at a synthesized node — e.g.
    // `(record (a 1) (a 2))`'s duplicate-field CDZ0201, collected at two nodes — collapses: after
    // stripping, the synthesized copy is unanchored and `dedup_faults` drops it against the anchored
    // twin. (Done here, not only in `diagnostics()`, so `compile()`'s error path dedups identically.)
    for f in &mut faults {
        sanitize_origin(db, f);
    }
    // UNIT-DEFINITION CONFLICTS (CDZ0502). A `(Unit.define #"name" …)` form is a top-level DECLARATION
    // (not a def body `type_errors` walks), so its uniqueness is checked here: a name declared with a
    // conversion conflicting with the built-in family table or an earlier declaration is CDZ0502
    // (`units-of-measure.md` §A Named Unit's Conversion Is Unique). An agreeing redeclaration is fine.
    crate::infer::check_unit_defines(db, &mut faults);
    // UNKNOWN UNITS. A quantity literal / `(Unit.of #"name")` naming a unit that is neither a built-in
    // family nor a user `Unit.define` (`5zorks`, `5gram`) fails to reduce and otherwise surfaces only as a
    // generic "no machine representation" decline — name the unknown unit (CDZ0201) with a did-you-mean.
    crate::infer::check_unknown_units(db, &mut faults);
    dedup_faults(db, faults)
}

/// Collapse duplicate faults — the SAME issue reported by more than one collection pass. A fault is
/// keyed by `(code, anchor node)`: the type-check walk and the reached-poison walk both visit an
/// unconditionally-evaluated position, so an unbound name (or any fault) in a REACHABLE spot is found
/// by both and would otherwise be reported twice at the same spot. Two faults with the same code AND
/// the same anchor are the one issue bubbling up along two paths — keep the first (stable order),
/// drop the rest. DISTINCT occurrences (same code, DIFFERENT node — e.g. two separate unbound uses)
/// are NOT duplicates and both survive. An UNANCHORED fault (`at == None`) dedups by code+message, so
/// two different unanchored declines still both show.
fn dedup_faults(db: &Db, faults: Vec<Reject>) -> Vec<Reject> {
    // If any CDZ0401 (an ungranted effect reached with no home) was produced, the emit path's UNCODED
    // "performed with no enclosing handler here" DECLINE is the same root cause reported more weakly —
    // drop it so one ungranted effect yields ONE primary `error:` (the coded CDZ0401), not a coded
    // rejection shadowed by an `error:` decline (`reference-compiler.md` §Outcomes Are Ordered By
    // Safety). Only suppressed WHEN a CDZ0401 exists — a standalone perform with no entrypoint check
    // covering it (should not happen for an exported body, but defensively) keeps its decline.
    let has_no_home_reject = faults.iter().any(|r| r.code == Some(Code::EffectNoHome));
    // A SELF-REFERENTIAL VALUE cycle (`(def (g) g)`) is reported with the clear coded CDZ0201 "defined in
    // terms of itself with no base value". The reduction of that same body ALSO spins into the depth guard
    // and emits the uncoded "expression nests too deeply (a recursion/resource limit)" decline — the same
    // root cause reported more weakly (and misleadingly, as a resource limit). Drop the decline whenever
    // the clear cycle reject is present, so a value cycle is ONE primary `error:` naming the real cause.
    let has_value_cycle_reject = faults.iter().any(|r| {
        r.code.is_some()
            && r.message
                .contains("defined in terms of itself with no base value")
    });
    // Likewise: the emit path's uncoded "value is not applyable" DECLINE is redundant when `infer`
    // proved the head a definite non-function (the coded `cannot apply a value of type … — it is not a
    // function` reject). Drop the weaker decline so applying a non-function is ONE primary `error:`.
    let has_not_a_function_reject = faults
        .iter()
        .any(|r| r.code.is_some() && r.message.starts_with(crate::diag::NOT_A_FUNCTION_PREFIX));
    // Likewise: the evaluator's uncoded "applied more arguments than the function accepts" DECLINE is
    // redundant when `infer` proved the over-application (the coded CDZ0203 `applied N arguments to a
    // function of arity M …` reject). Drop the weaker decline so over-application is ONE primary error.
    let has_over_application_reject = faults.iter().any(|r| {
        r.code.is_some()
            && (r.message.contains(crate::diag::OVER_APPLICATION_MARKER)
                || r.message
                    .contains(crate::diag::MEMBER_OVER_APPLICATION_MARKER))
    });
    // Likewise: a MALFORMED handler (an arm naming an undeclared op — CDZ0403 — or one not discharging
    // every operation — CDZ0405) cannot fold, so `lower` returns the uncoded "not yet reducible by the
    // tail-resumptive fold" DECLINE alongside the coded reject. The decline is a CONSEQUENCE of the very
    // defect the coded reject reports (with its fix), not an independent limitation — drop it so a
    // misspelled/missing arm yields ONE primary `error:` carrying the actionable fix, not a coded reject
    // shadowed by an emit-path decline. Only suppressed WHEN such a reject exists — a WELL-FORMED handler
    // that is genuinely not-yet-reducible (a real cross-function / non-tail resume) keeps its honest
    // decline (there is no coded reject to defer to).
    let has_malformed_handler_reject = faults.iter().any(|r| {
        matches!(
            r.code,
            Some(Code::HandlerUndeclaredOp) | Some(Code::HandlerNotExhaustive)
        )
    });
    // A resume-value/result-type mismatch (CDZ0201) ALSO makes the handler unfoldable — same relationship
    // the malformed-handler rejects have with the "not yet reducible" decline. Suppress the decline when
    // such a reject is present so a mistyped resume reports ONE primary error (with its coercion fix).
    let has_resume_result_reject = faults.iter().any(|r| {
        r.code == Some(Code::Malformed)
            && r.message
                .contains(crate::diag::RESUME_RESULT_MISMATCH_MARKER)
    });
    // Likewise: a NON-CANONICAL handle (the retired effect-name-less shape) is rejected at resolve time
    // (`resolve_noncanonical_handle`, a CDZ0201). Because the handle never resolved as a handler, its
    // body's perform is seen by the entrypoint no-home walk as reached with NO enclosing handler → a
    // CONSEQUENT CDZ0401. That misdirects (the author DID write a handler — it is just not canonical), so
    // drop the CDZ0401 whenever the non-canonical reject is present, keeping the CDZ0201 that says how to
    // fix the handle as the ONE primary error. Matched by message prefix (the reject reuses `Malformed`).
    let has_noncanonical_handle_reject = faults.iter().any(|r| {
        r.code == Some(Code::Malformed)
            && r.message
                .starts_with(crate::diag::HANDLE_NONCANONICAL_PREFIX)
    });
    // Likewise: a NON-ARROW op type (`(op get Int64)`) is rejected CDZ0201 at the declaration. Its op-value
    // `(meta t)` is wrapped `(fn () Int64)`, so PERFORMING the op leaks the internal op-record in a
    // consequent CDZ0203 ("type mismatch: Int64 and (Record (apply Any) (effect-op Any) …)"). That leak is
    // a CONSEQUENCE of the malformed declaration — drop it (a fault naming the internal op-record) whenever
    // the malformed-op-type reject is present, keeping the declaration-site reject (with its wrap fix) as
    // the ONE primary, actionable error.
    let has_non_arrow_op_type_reject = faults.iter().any(|r| {
        r.code == Some(Code::Malformed)
            && r.message.starts_with(crate::diag::NON_ARROW_OP_TYPE_PREFIX)
    });
    // Likewise: a `handle` whose HEAD names a VALUE (`(handle foo …)`) is rejected CDZ0201 at the head. The
    // head is desugared into each arm's `(. foo op)` projection, so the value head ALSO leaks a CDZ0201
    // "member access requires a record, found <T>" (from `(. foo op)`) and the uncoded fold-decline — both
    // CONSEQUENCES of the value head. Drop them when the clean head reject is present, keeping the one
    // primary that names the real problem.
    let has_value_head_reject = faults.iter().any(|r| {
        r.code == Some(Code::Malformed)
            && r.message.starts_with(crate::diag::HANDLE_VALUE_HEAD_PREFIX)
    });
    // Likewise: an exported closure with a non-representable part (an `Any` param/result, a captured
    // value with no machine type) is reported as the coded CDZ0201 "cannot cross the component boundary"
    // at the export clause. The emit path ALSO returns an uncoded "a closure's <part> has no machine
    // representation" decline at the closure BODY — a DIFFERENT node, so the same-node general rule below
    // does not catch it; drop the decline program-wide when the CDZ0201 is present, keeping the coded
    // reject (which names the concrete cause — unannotated param / partial application) as the ONE "no".
    let has_closure_boundary_reject = faults
        .iter()
        .any(|r| r.code.is_some() && r.message.contains(crate::diag::CLOSURE_BOUNDARY_MARKER));
    // Likewise: a TYPE-VALUED export is reported as the coded CDZ0201 "export `<name>` is a TYPE …" at the
    // export clause. The emit path declines the SAME body through several no-runtime-form paths (type
    // value / nullary lambda / bare prim / closure param) — all UNANCHORED, so neither the same-node rule
    // nor node-keyed dedup collapses them. Drop the whole decline family program-wide when the CDZ0201 is
    // present, keeping the coded reject as the ONE "no".
    let has_type_export_reject = faults
        .iter()
        .any(|r| r.code.is_some() && r.message.contains(crate::diag::TYPE_EXPORT_MARKER));
    // The EFFECT-valued-export analogue: exporting a bare effect name evaluates its synthesized record,
    // leaking a cascade (an "unknown intrinsic" decline, a nullary-lambda-no-closure decline, and two
    // `unbound name effect-op`/`effect` CDZ0101s from the internal `(meta …)` field names). Drop that
    // cascade when the clean effect-export reject is present, keeping the one category message.
    let has_effect_export_reject = faults
        .iter()
        .any(|r| r.code.is_some() && r.message.contains(crate::diag::EFFECT_EXPORT_MARKER));
    // A "record has no field `k`" fault reported by BOTH the infer member check (with a did-you-mean fix)
    // AND the emit-side member fold (fix-less) is ONE fault shown twice. Where the member node is a USER
    // node, both copies now anchor at that SAME node (`lower`'s `Member::NoField` poison carries an
    // explicit `.at(id)`, symmetric with `infer::no_field_reject`), so a NODE-keyed drop collapses them
    // without touching a genuinely-distinct absent-field fault on ANOTHER record that happens to name the
    // same missing field (`(. r fild)` near-miss + `(. s fild)` far-miss sit at DIFFERENT nodes). But when
    // the member access is INLINED (a helper `(def (getx a) (. a k))` β-reduced at its call site), the
    // emit copy's member node is SYNTHESIZED — `sanitize_origin` clears its anchor to `None` at the ABI
    // edge — so it is NOT at infer's user-node and the node-keyed rule cannot see it; the fix suffix also
    // makes its (code, message) differ from infer's, so the general anchored/unanchored dedup misses it
    // too. A NAME-keyed fallback catches that copy, but ONLY when it is UNANCHORED (a located far fault on
    // a distinct record keeps its own node, so it is never in this branch). Together: node-keyed for the
    // ordinary same-node case (no false-merge), name-keyed-when-unanchored for the inlined/synthesized twin.
    fn no_field_key(msg: &str) -> Option<&str> {
        // The invariant core is `record has no field \`k\`` — strip an optional ` — did you mean …?` tail.
        msg.strip_prefix(crate::diag::NO_FIELD_PREFIX)
            .map(|rest| rest.split(" — ").next().unwrap_or(rest))
    }
    // The NODES at which a no-field fault carries a fix (the infer did-you-mean copy) — a fix-less no-field
    // fault at one of these same nodes is that copy's twin and is dropped below (keep the richer copy).
    let fixed_field_nodes: std::collections::HashSet<u32> = faults
        .iter()
        .filter(|r| r.fix.is_some() && no_field_key(&r.message).is_some())
        .filter_map(|r| r.at.map(|s| s.0))
        .collect();
    // The field-name CORES a no-field fault carries a fix for — used ONLY to drop an UNANCHORED fix-less
    // twin (the inlined/synthesized emit copy, sanitized to no location); a located far fault on a
    // different record is never unanchored, so this name-level set cannot false-merge it.
    let fixed_field_cores: std::collections::HashSet<&str> = faults
        .iter()
        .filter(|r| r.fix.is_some())
        .filter_map(|r| no_field_key(&r.message))
        .collect();
    // A defect INSIDE a called function's body — a non-exhaustive `(match c …)` (CDZ0210) on a parameter —
    // is reported TWICE: once at the DEFINITION (the def's own body check, anchored at the real match node,
    // its fix editing that match) and once re-anchored to a CALL SITE (`collect_reached_poisons` inlines the
    // callee and re-reaches the same poison; its fix targets the SYNTHESIZED reduced-match node). Both are
    // coded, both carry a fix HERE (the call-site copy's fix is stripped only LATER, at the ABI edge, when
    // its synthesized target is found non-user) — so a plain (code, message) or fix-vs-no-fix test cannot
    // tell them apart yet. The discriminator is the FIX'S EDIT TARGET: the authoritative copy edits a USER
    // node (the source match); the duplicate edits a synthesized node. Collect the (code, message) of every
    // fault whose fix targets a USER node; drop a same-(code,message) copy whose fix targets a NON-user one.
    // SAFE: two genuinely-distinct same-(code,message) matches each edit their OWN user node, so neither is
    // dropped (both fixes target user nodes → neither is the "non-user" copy).
    let user_fix_keys: std::collections::HashSet<(Option<Code>, &str)> = faults
        .iter()
        .filter(|r| {
            r.fix
                .as_ref()
                .is_some_and(|f| db.is_user_node(f.edit.target()))
        })
        .map(|r| (r.code, r.message.as_str()))
        .collect();
    // OVER-APPLICATION OF A FIXED-ARITY OPERATOR reports TWICE: the authoritative CDZ0201 "+ takes exactly
    // 2 operands" (the grammar/lower arity reject) AND the generic CDZ0203 "applied N arguments to a
    // function of arity M" — both now carrying a delete fix on the SAME surplus operand node. Keep the
    // operator-specific CDZ0201, drop the CDZ0203 whose delete fix targets a node a CDZ0201 delete fix also
    // targets. Collect the surplus nodes a `Malformed` (CDZ0201) delete fix targets; the over-application
    // `TypeMismatch` copy pointing at one of them is the sibling to drop. (A ctor/user-fn over-application
    // has NO CDZ0201, so its CDZ0203 — the only report — is never in this set and survives with its fix.)
    let operator_arity_fix_nodes: std::collections::HashSet<u32> = faults
        .iter()
        .filter(|r| r.code == Some(Code::Malformed))
        .filter_map(|r| r.fix.as_ref())
        .filter(|f| matches!(f.edit, crate::diag::Edit::Delete { .. }))
        .map(|f| f.edit.target().0)
        .collect();
    // The SAME fault reported once ANCHORED (a node stamped by the reached-poison walk) and once
    // UNANCHORED (the resolve-level poison surfaced with no `at`) — e.g. `(record (a 1) (a 2))`'s
    // duplicate-field CDZ0201 — has two DIFFERENT dedup keys below (one by node, one by message), so
    // both slip through as two `error:` lines for one issue. Collect the (code, message) of every
    // ANCHORED fault; an unanchored fault whose (code, message) matches one is that same fault minus its
    // location — drop it, keeping the anchored copy (which carries a line:col).
    let anchored_keys: std::collections::HashSet<(Option<Code>, &str)> = faults
        .iter()
        .filter(|r| r.at.is_some())
        .map(|r| (r.code, r.message.as_str()))
        .collect();
    // The no-field ANALOGUE of `anchored_keys`, keyed by the INVARIANT CORE (not the full message): an
    // absent-field miss can surface ANCHORED with one tier-suffix (`— closest matches: …`) AND UNANCHORED
    // bare (a second desugar/reduction path — e.g. a handler op `(. E k)` reaches both the handle's
    // op-resolution and the perform), so the two carry DIFFERENT messages and slip past `anchored_keys`.
    // Collect the no-field cores that appear ANCHORED; an unanchored no-field fault sharing a core is that
    // same miss minus its location + suffix — drop it, keeping the LOCATED (richer) copy. A distinct
    // field-miss keeps its own core, so this never over-merges.
    let anchored_no_field_cores: std::collections::HashSet<&str> = faults
        .iter()
        .filter(|r| r.at.is_some())
        .filter_map(|r| no_field_key(&r.message))
        .collect();
    // The GENERAL shadowing rule (subsumes the specific ones above for the same-node case): a node that
    // carries a CODED reject already has its authoritative, actionable "no"; an UNCODED decline anchored
    // at that SAME node is the weaker consequence of the same defect (`reference-compiler.md` §Outcomes
    // Are Ordered By Safety — a coded rejection dominates a decline). Drop it. E.g. `(Symbol.of 5)`: the
    // CDZ0203 type error and the emit path's "runtime string" decline both anchor the call node → keep
    // only the CDZ0203. A decline at a node with NO coded reject (a genuinely-unbuilt construct) survives.
    let coded_nodes: std::collections::HashSet<u32> = faults
        .iter()
        .filter(|r| r.code.is_some())
        .filter_map(|r| r.at.map(|s| s.0))
        .collect();
    let mut seen: std::collections::HashSet<(Option<Code>, Option<u32>, Option<String>)> =
        std::collections::HashSet::new();
    faults
        .iter()
        .filter(|r| {
            if has_no_home_reject
                && r.is_decline()
                && r.message == crate::diag::NO_HOME_STANDALONE_DECLINE
            {
                return false;
            }
            // Drop the "nests too deeply (resource limit)" decline that a value cycle's reduction spins
            // into — the clear CDZ0201 cycle reject names the same fault correctly.
            if has_value_cycle_reject && r.is_decline() && r.message.contains("nests too deeply") {
                return false;
            }
            if has_not_a_function_reject
                && r.is_decline()
                && r.message == crate::diag::NOT_APPLYABLE_DECLINE
            {
                return false;
            }
            if has_over_application_reject
                && r.is_decline()
                && r.message == crate::diag::OVER_APPLICATION_DECLINE
            {
                return false;
            }
            // The BUILT-IN-OPERATION wrong-arity decline (`<op> is applied at the wrong arity …`, from
            // `lower`) fires on both an under- and an OVER-application. On an over-application `infer`'s
            // coded CDZ0203 is the primary "no" (carrying the delete-surplus fix), so drop this weaker
            // decline — one primary error for `(Map.size m x)`, not a coded reject shadowed by a decline.
            // On an UNDER-application there is no such coded reject, so `has_over_application_reject` is
            // false and the decline is KEPT (it is the only report of the missing argument).
            if has_over_application_reject
                && r.is_decline()
                && r.message.contains(crate::diag::BUILTIN_WRONG_ARITY_DECLINE)
            {
                return false;
            }
            if (has_malformed_handler_reject || has_resume_result_reject)
                && r.is_decline()
                && r.message == crate::diag::HANDLER_NOT_REDUCIBLE_DECLINE
            {
                return false;
            }
            if has_closure_boundary_reject
                && r.is_decline()
                && matches!(
                    r.message.as_str(),
                    crate::diag::CLOSURE_PARAM_NO_REPR_DECLINE
                        | crate::diag::CLOSURE_RESULT_NO_REPR_DECLINE
                        | crate::diag::CLOSURE_CAPTURE_NO_REPR_DECLINE
                )
            {
                return false;
            }
            if has_type_export_reject
                && r.is_decline()
                && matches!(
                    r.message.as_str(),
                    crate::diag::TYPE_VALUE_NO_RUNTIME_DECLINE
                        | crate::diag::NULLARY_LAMBDA_NO_CLOSURE_DECLINE
                        | crate::diag::PRIM_AS_VALUE_DECLINE
                        | crate::diag::CLOSURE_PARAM_NO_REPR_DECLINE
                        | crate::diag::CLOSURE_RESULT_NO_REPR_DECLINE
                        | crate::diag::CLOSURE_CAPTURE_NO_REPR_DECLINE
                )
            {
                return false;
            }
            // The EFFECT-valued-export cascade: the two DECLINES (`unknown intrinsic`, nullary-lambda-no-
            // closure) AND the two `unbound name effect-op`/`effect` CDZ0101s (the effect record's internal
            // `(meta …)` field names, never user-written) are all consequences of evaluating the effect
            // value the clean reject already reports. Drop them when the effect-export reject is present.
            if has_effect_export_reject
                && (r.is_decline()
                    && matches!(
                        r.message.as_str(),
                        crate::diag::UNKNOWN_INTRINSIC_DECLINE
                            | crate::diag::NULLARY_LAMBDA_NO_CLOSURE_DECLINE
                    )
                    || (r.code == Some(Code::Unbound)
                        && matches!(
                            r.message.as_str(),
                            "unbound name `effect-op`" | "unbound name `effect`"
                        )))
            {
                return false;
            }
            // A CDZ0401 (no home) that is the CONSEQUENCE of a non-canonical handle failing to resolve as
            // a handler — drop it in favor of the CDZ0201 that reports the real, fixable defect.
            if has_noncanonical_handle_reject && r.code == Some(Code::EffectNoHome) {
                return false;
            }
            // A perform-site type mismatch that LEAKS the internal op-value record (`(effect-op Any)` — a
            // synthesized meta-channel field no user type spells) is the CONSEQUENCE of a malformed
            // non-arrow op type; drop it in favor of the declaration-site CDZ0201 (with its wrap fix).
            if has_non_arrow_op_type_reject && r.message.contains(crate::diag::OP_VALUE_RECORD_LEAK)
            {
                return false;
            }
            // A `handle` whose HEAD names a value leaks two consequents from the desugared `(. foo op)`
            // projections: the uncoded fold-decline and a "member access requires a record" CDZ0201 (foo is
            // a scalar). Both are consequences of the value head — drop them for the clean head reject.
            if has_value_head_reject
                && (r.message == crate::diag::HANDLER_NOT_REDUCIBLE_DECLINE
                    || r.message.contains("member access requires a record"))
            {
                return false;
            }
            // Likewise: a CDZ0401 (no home) that is the CONSEQUENCE of a MALFORMED HANDLER — a misspelled
            // arm op (CDZ0403) or a missing arm (CDZ0405) — leaves the effect's operation set only
            // partly discharged, so the handled body's perform spuriously looks home-less. `(handle E …
            // ((emitt …)) (E.emit …))` reports the arm-typo CDZ0403 ("did you mean `emit`?", with its
            // fix) AND a derived CDZ0401 on `(E.emit …)`; fixing the arm spelling clears BOTH. Drop the
            // CDZ0401 in favor of the CDZ0403/CDZ0405 that names the actual, fixable defect (one primary
            // "no" per root cause — `reference-compiler.md` §Outcomes Are Ordered By Safety).
            if has_malformed_handler_reject && r.code == Some(Code::EffectNoHome) {
                return false;
            }
            // The over-application CDZ0203 sibling of an operator-arity CDZ0201 (see `operator_arity_fix_nodes`):
            // drop it when its delete fix targets the same surplus operand the authoritative CDZ0201's fix does.
            if r.code == Some(Code::TypeMismatch)
                && r.message.contains(crate::diag::OVER_APPLICATION_MARKER)
                && r.fix.as_ref().is_some_and(|f| {
                    matches!(f.edit, crate::diag::Edit::Delete { .. })
                        && operator_arity_fix_nodes.contains(&f.edit.target().0)
                })
            {
                return false;
            }
            // An unanchored fault that also appears ANCHORED (same code + message) is that fault minus
            // its location — drop it, the anchored copy already carries the issue with a line:col.
            if r.at.is_none() && anchored_keys.contains(&(r.code, r.message.as_str())) {
                return false;
            }
            // The no-field CORE analogue: an unanchored "record has no field `k`" whose core ALSO appears
            // anchored is that same miss minus its location + tier-suffix — drop it, keep the located copy.
            // (Full-message `anchored_keys` misses it when the two carry different tier-suffixes.)
            if r.at.is_none()
                && no_field_key(&r.message).is_some_and(|k| anchored_no_field_cores.contains(k))
            {
                return false;
            }
            // A DECLINE anchored at a node that also carries a CODED reject is shadowed by it — drop it.
            if r.is_decline() && r.at.is_some_and(|s| coded_nodes.contains(&s.0)) {
                return false;
            }
            // A FIX-LESS "record has no field `k`" copy that is the emit-side duplicate of infer's
            // did-you-mean copy — drop it, keep the fix. Two shapes: (1) at the SAME member NODE as a fixed
            // twin (the ordinary case, both anchored at the user member node); (2) UNANCHORED with the same
            // field-name core as a fixed twin (the INLINED case — the synthesized member node was
            // sanitized to no location). Both are keyed so a genuinely-distinct absent-field fault on
            // another record — which stays ANCHORED at its OWN node — is never dropped by either branch.
            if r.fix.is_none()
                && no_field_key(&r.message).is_some_and(|k| match r.at {
                    Some(s) => fixed_field_nodes.contains(&s.0),
                    None => fixed_field_cores.contains(k),
                })
            {
                return false;
            }
            // The re-anchored body-defect duplicate (see `user_fix_keys`): a coded fault whose fix targets a
            // SYNTHESIZED (non-user) node, when the SAME (code, message) is ALSO reported with a fix editing a
            // USER node, is the call-site copy of a callee body defect — drop it, keeping the authoritative
            // definition-anchored copy whose fix edits the real source. E.g. a non-exhaustive `(match c …)` in
            // a CALLED `f`: CDZ0210 at the def (fix edits the match) + the inlined copy at the `(f …)` call
            // (fix edits the reduced match, a synthesized node).
            if r.fix
                .as_ref()
                .is_some_and(|f| !db.is_user_node(f.edit.target()))
                && user_fix_keys.contains(&(r.code, r.message.as_str()))
            {
                return false;
            }
            // An anchored fault is identified by (code, node); an unanchored one by (code, message)
            // so distinct declines with no node still both appear. EXCEPTION: an unanchored "record has
            // no field `k`" is keyed by its INVARIANT CORE (`no_field_key` strips the ` — did you mean …`
            // / ` — closest matches: …` tail), NOT the full message — because ONE absent-field miss can
            // surface via TWO unanchored reduction paths (a handler op `(. E k)` is desugared into both
            // the handle's op-resolution AND the perform) whose tier-suffixes DIFFER (one drew a
            // closest-matches list, the other stayed bare), so a full-message key lets both through as a
            // double report. Keying by the core collapses the two copies of the SAME field-miss while
            // leaving a DISTINCT field-miss (different `k`, or anchored at its own node) its own key.
            let msg_key = r.at.is_none().then(|| {
                no_field_key(&r.message)
                    .map(str::to_string)
                    .unwrap_or_else(|| r.message.clone())
            });
            seen.insert((r.code, r.at.map(|s| s.0), msg_key))
        })
        .cloned()
        .collect()
}

/// Collect poisons reached UNCONDITIONALLY from `id`. Descends the core form into positions a value is
/// unconditionally used (an `if` CONDITION), but NOT into a conditional's branches — a poison shielded
/// by an untaken branch is not a build failure. Reads the core column on demand.
///
/// This is the RECURSIVE-DESCENT DEPTH GUARD in front of the walk: β-reduction can leave a MEMOIZED core
/// chain thousands of nodes deep (a non-normalizing self-application bottoms out in a `RecursionBound`
/// poison only after the reduction budget clips it — e.g. `((fn v (tuple (v v) 1)) (fn v (tuple (v v) 1)))`
/// leaves a `Tuple[Tuple[…poison…, 1], 1]` chain ~`REDUCE_NODE_BUDGET`-deep). That chain is built bottom-up
/// at shallow demand depths, so `core_of`'s own descent guard never fires on it — but the poison walk then
/// descends the whole pre-built chain in one native recursion and OVERFLOWS THE STACK (a process abort) on
/// a small valid-to-parse program. Past [`DESCENT_DEPTH_LIMIT`] surface the reduction-bound poison (CDZ0999,
/// the same code the chain's innermost node carries) instead of recursing — a compiler must never crash on
/// well-formed input, only decline or complete. The guard sits at the ONE recursive entry and the walk
/// dispatches structurally, so it covers the whole compound-construction class (tuple/record/list/sum/map/
/// set/…) at once, not one syntactic wrapper.
fn collect_reached_poisons(db: &mut Db, id: StructId, out: &mut Vec<Reject>) {
    if db.descent_depth >= crate::db::DESCENT_DEPTH_LIMIT {
        let mut r = Reject::coded(
            Code::RecursionBound,
            "an expression does not reduce to a value within the compiler's reduction limits (a call chain nested too deeply, or a non-terminating / explosively-growing reduction)",
        );
        r.set_origin_if_absent(id);
        out.push(r);
        // This subtree was clipped at the depth backstop — mark it so the ancestor does NOT record itself
        // (or `id`) as fully walked; a shallower path must still be free to walk it unclipped.
        db.reached_clipped = true;
        return;
    }
    // VISITED-SET: the walk follows `core_of`, which resolves a `Ref` to its target's body, so a value
    // used in two operand positions is reached from both — a repeated-squaring `BigInt` chain (`(* a a)`
    // over NON-folding operands) is a shared DAG the naive recursion walks as a TREE, O(2^depth). A node's
    // reached-poison contribution is a pure function of the node (its poison's origin is its own id), and
    // `dedup_faults` collapses duplicates, so skipping an already-walked node changes no reported fault.
    if db.reached_visited.contains(&id) {
        return;
    }
    db.descent_depth += 1;
    // Track whether THIS subtree clips: save the ancestor's flag, clear it for our own recursion, then
    // OR it back so a clip still propagates upward. Only an UNCLIPPED (complete) subtree is memoized —
    // a partial one must be re-walkable from a shallower entry (the `collect_limited`/`collect_cache`
    // discipline).
    let outer_clipped = db.reached_clipped;
    db.reached_clipped = false;
    collect_reached_poisons_at(db, id, out);
    let this_clipped = db.reached_clipped;
    if !this_clipped {
        db.reached_visited.insert(id);
    }
    db.reached_clipped = outer_clipped || this_clipped;
    db.descent_depth -= 1;
}

fn collect_reached_poisons_at(db: &mut Db, id: StructId, out: &mut Vec<Reject>) {
    // A `do` SEQUENCING block resolves to a `Ref` to its LAST form, so `core_of` follows only that. But
    // every INTERMEDIATE form is UNCONDITIONALLY evaluated (its value discarded), so a provable trap in
    // one is a build failure — descend into every form here (the raw AST head, since the core collapsed
    // to the last form's ref). The last form is also covered, harmlessly.
    if db.ast.head_name(id) == Some("do")
        && let Some(forms) = db.ast.as_form(id, "do")
    {
        let forms: Vec<StructId> = forms.to_vec();
        for f in forms {
            // A do-local `(def …)` is a BINDING, not an unconditionally-evaluated statement: its value is
            // computed only where the name is used, so a provable trap in an UNUSED declaration is not a
            // build failure (it is the CDZ0305 "always traps but never used" warning a `let` binding
            // gets, raised by the DCE pass — not a `collect_reached_poisons` fault). A USED declaration's
            // trap is reached through the reference site (the value is inlined there), so it is caught
            // there. So a def-form is skipped here; only a pure STATEMENT form is unconditional.
            if db.ast.head_name(f) == Some("def") {
                continue;
            }
            // A do-local `(type …)` / `(effect …)` / `(module …)` is a DECLARATION (its record is
            // synthesized at load), not an evaluated statement — skip it like a `def` (resolving it as a
            // value would decline).
            if matches!(
                db.ast.head_name(f),
                Some("type") | Some("effect") | Some("module")
            ) {
                continue;
            }
            collect_reached_poisons(db, f, out);
        }
        return;
    }
    match core_of(db, id) {
        // Stamp the poison's origin with this node if it carries none — a poison produced without a
        // precise anchor is at least attributed to the node it was reached at. (`sanitize_origin` at
        // the ABI edge later drops it if this node turns out to be prelude/synthesized.)
        Core::Poison(mut r) => {
            r.set_origin_if_absent(id);
            out.push(r);
        }
        Core::If { cond, .. } => {
            // The condition is unconditionally evaluated; the branches are not (they are guarded).
            collect_reached_poisons(db, cond, out);
        }
        // A short-circuiting connective: the LEFT operand is unconditionally evaluated; the RIGHT is
        // guarded (reached only on the non-short-circuit branch), so a provable trap in `rhs` is NOT a
        // build failure — the same reachability rule as an `if`'s branches.
        Core::And { lhs, .. } => {
            collect_reached_poisons(db, lhs, out);
        }
        // A record's field values are all unconditionally part of the value — descend into each. (A
        // record used only to read a field folds away before reaching here; one that survives is a
        // runtime value whose fields are all reached.)
        Core::Record { fields } => {
            for (_, &value) in fields.iter() {
                collect_reached_poisons(db, value, out);
            }
        }
        // Both operands of a runtime arithmetic, comparison, or structural-equality op are
        // unconditionally evaluated.
        Core::Arith { lhs, rhs, .. }
        | Core::Compare { lhs, rhs, .. }
        | Core::ValueEq { lhs, rhs } => {
            collect_reached_poisons(db, lhs, out);
            collect_reached_poisons(db, rhs, out);
        }
        // A conversion's or a boolean negation's operand is unconditionally evaluated.
        Core::Convert { operand, .. } | Core::Not { operand } => {
            collect_reached_poisons(db, operand, out);
        }
        // An A-normal `let`: every bound value is unconditionally computed (a kept binding names a
        // value used more than once, always evaluated), and the body is unconditionally reached — so a
        // provable trap in either is a build failure. Descend into each.
        Core::Let { bindings, body } => {
            for (_, value) in bindings {
                collect_reached_poisons(db, value, out);
            }
            collect_reached_poisons(db, body, out);
        }
        // A runtime call: its arguments are unconditionally evaluated, so descend into each. The
        // CALLEE's own body faults surface when it is collected (a reachable def is checked on its own
        // — `collect_faults` covers every def body), so we do not re-enter the callee here.
        Core::Call { args, .. } => {
            for arg in args {
                collect_reached_poisons(db, arg, out);
            }
        }
        // A host call unconditionally evaluates its arguments before crossing the boundary — descend into
        // each (the call itself is a boundary import, not a def whose body could fault).
        Core::HostCall { args, .. } => {
            for arg in args {
                collect_reached_poisons(db, arg, out);
            }
        }
        // A sequencing block unconditionally evaluates every statement AND the tail — descend into each.
        Core::Seq { stmts, tail } => {
            for s in stmts {
                collect_reached_poisons(db, s, out);
            }
            collect_reached_poisons(db, tail, out);
        }
        // A match: the scrutinee is unconditionally evaluated (descend), but each arm BODY is guarded
        // (only the matching arm runs) — so a provable trap inside an arm is NOT a build failure, the
        // same reachability rule as an `if`'s branches. Do not descend into the arm bodies.
        Core::Match { scrutinee, .. } => {
            collect_reached_poisons(db, scrutinee, out);
        }
        // A tuple's elements are all unconditionally part of the value; a projection's operand is
        // unconditionally evaluated. Descend into each.
        Core::Tuple { elems } | Core::ListNew { elems } | Core::BytesOf { elems } => {
            for e in elems {
                collect_reached_poisons(db, e, out);
            }
        }
        // A runtime `(bin …)` unconditionally evaluates every segment value.
        Core::BinBuild { segs } => {
            for s in segs {
                collect_reached_poisons(db, s.value, out);
            }
        }
        Core::BinBitsBuild { fields } => {
            for f in fields {
                collect_reached_poisons(db, f.value, out);
            }
        }
        Core::BinIntRead { bytes, .. } | Core::BinRestRead { bytes, .. } => {
            collect_reached_poisons(db, bytes, out)
        }
        Core::Proj { operand, .. } | Core::ListLen { operand } | Core::BytesLen { operand } => {
            collect_reached_poisons(db, operand, out)
        }
        // `List.push`/`concat` unconditionally evaluate both operands — descend into each.
        Core::ListPush { list, elem } => {
            collect_reached_poisons(db, list, out);
            collect_reached_poisons(db, elem, out);
        }
        Core::ListConcat { lhs, rhs } => {
            collect_reached_poisons(db, lhs, out);
            collect_reached_poisons(db, rhs, out);
        }
        // `List.update` unconditionally evaluates all three operands — descend into each.
        Core::ListUpdate { list, index, elem } => {
            collect_reached_poisons(db, list, out);
            collect_reached_poisons(db, index, out);
            collect_reached_poisons(db, elem, out);
        }
        // `List.at` unconditionally evaluates the list and index (the bounds check reads both) — descend.
        Core::ListAt { list, index, .. } => {
            collect_reached_poisons(db, list, out);
            collect_reached_poisons(db, index, out);
        }
        Core::BytesAt { bytes, index, .. } => {
            collect_reached_poisons(db, bytes, out);
            collect_reached_poisons(db, index, out);
        }
        Core::StrAt { string, index, .. } => {
            collect_reached_poisons(db, string, out);
            collect_reached_poisons(db, index, out);
        }
        Core::BytesConcat { lhs, rhs } => {
            collect_reached_poisons(db, lhs, out);
            collect_reached_poisons(db, rhs, out);
        }
        Core::BigIntBinOp { lhs, rhs, .. }
        | Core::BigIntCmp { lhs, rhs, .. }
        | Core::RationalOfInts { num: lhs, den: rhs }
        | Core::RationalBinOp { lhs, rhs, .. }
        | Core::RationalCmp { lhs, rhs, .. } => {
            collect_reached_poisons(db, lhs, out);
            collect_reached_poisons(db, rhs, out);
        }
        Core::BigIntOfI64 { value } => collect_reached_poisons(db, value, out),
        Core::BigIntToI64 { operand } => collect_reached_poisons(db, operand, out),
        Core::RationalOfIntWiden { value } => collect_reached_poisons(db, value, out),
        Core::BytesSlice {
            bytes, start, len, ..
        } => {
            collect_reached_poisons(db, bytes, out);
            collect_reached_poisons(db, start, out);
            collect_reached_poisons(db, len, out);
        }
        Core::BytesCompact { operand } => collect_reached_poisons(db, operand, out),
        // A map construction's entry keys AND values are all unconditionally part of the value — descend
        // into each `(key, value)` pair.
        Core::MapNew { entries, .. } => {
            for (k, v) in entries {
                collect_reached_poisons(db, k, out);
                collect_reached_poisons(db, v, out);
            }
        }
        // `Map.insert` unconditionally evaluates the map, key, and value — descend into each.
        Core::MapInsert { map, key, val, .. } => {
            collect_reached_poisons(db, map, out);
            collect_reached_poisons(db, key, out);
            collect_reached_poisons(db, val, out);
        }
        // `Map.lookup`/`Map.remove` unconditionally evaluate the map and key — descend into both.
        Core::MapLookup { map, key, .. } | Core::MapRemove { map, key, .. } => {
            collect_reached_poisons(db, map, out);
            collect_reached_poisons(db, key, out);
        }
        // `Map.size` unconditionally evaluates the map operand — descend.
        Core::MapSize { map } => collect_reached_poisons(db, map, out),
        // A set construction's elements are all unconditionally part of the value — descend into each.
        Core::SetOf { elems, .. } => {
            for e in elems {
                collect_reached_poisons(db, e, out);
            }
        }
        // `Set.contains`/`insert`/`remove` unconditionally evaluate the set and element — descend into both.
        Core::SetContains { set, elem, .. }
        | Core::SetInsert { set, elem, .. }
        | Core::SetRemove { set, elem, .. } => {
            collect_reached_poisons(db, set, out);
            collect_reached_poisons(db, elem, out);
        }
        // `Set.len` unconditionally evaluates the set operand — descend.
        Core::SetLen { set } => collect_reached_poisons(db, set, out),
        // A set-algebra op unconditionally evaluates both operand sets — descend into each.
        Core::SetAlgebra { lhs, rhs, .. } => {
            collect_reached_poisons(db, lhs, out);
            collect_reached_poisons(db, rhs, out);
        }
        // A sum construction's payloads are all unconditionally part of the value — descend into each.
        Core::SumNew { payloads, .. } => {
            for p in payloads {
                collect_reached_poisons(db, p, out);
            }
        }
        // A sum match: the scrutinee is unconditionally evaluated (descend); each arm BODY is guarded
        // (only the matching arm runs), so a trap inside an arm is NOT a build failure — same as `Match`
        // and `if`. Do not descend into arm bodies. A sum-payload read evaluates the scrutinee.
        Core::MatchSum { scrutinee, .. } => collect_reached_poisons(db, scrutinee, out),
        // A list match: like `MatchSum`, the scrutinee is unconditionally evaluated (descend) but each arm
        // body is guarded by its length condition, so a trap in an arm is not a build failure.
        Core::MatchList { scrutinee, .. } => collect_reached_poisons(db, scrutinee, out),
        Core::SumPayload { scrutinee, .. } => collect_reached_poisons(db, scrutinee, out),
        // `expect` unconditionally evaluates its scrutinee (descend); the absent-variant trap is a RUNTIME
        // trap on a runtime discriminant, not a compile-time provable poison — nothing to collect there.
        Core::SumExpect { scrutinee, .. } => collect_reached_poisons(db, scrutinee, out),
        // A closure's captured values are unconditionally part of the value; a closure application
        // unconditionally evaluates the closure and its argument. Descend for their provable faults.
        Core::Closure { captures, .. } => {
            for c in captures {
                collect_reached_poisons(db, c, out);
            }
        }
        Core::CallClosure { closure, args } => {
            collect_reached_poisons(db, closure, out);
            for arg in args {
                collect_reached_poisons(db, arg, out);
            }
        }
        // A parameter, a let-binding reference, or a CAPTURED-variable read is a runtime read — no
        // sub-poison to collect.
        // `trap` is an EXPLICIT runtime divergence (`Core::Trap` → `unreachable`), not a compile-provable
        // trap the build must reject — the honest "this halts here" primitive whose defined outcome IS the
        // runtime trap (like `expect`'s absent branch), so it carries no poison to collect.
        Core::Captured { .. }
        | Core::LocalRef { .. }
        | Core::Param { .. }
        | Core::ConstInt(_)
        | Core::ConstRational(_, _)
        | Core::ConstBool(_)
        | Core::ConstStr(_)
        | Core::ConstChar(_)
        | Core::ConstFloat(_)
        | Core::ConstFloatNan
        | Core::Trap
        | Core::Unit => {}
    }
}

/// How much of the scrutinee an arm PATTERN covers, for redundant-arm detection. Only the shapes whose
/// coverage is DECIDABLE from the pattern alone are represented; anything subtler is not classified (its
/// arm is treated as covering nothing and shadowing nothing), so the check stays conservative — it never
/// warns where it cannot prove redundancy.
#[derive(PartialEq, Eq, Hash, Clone)]
enum ArmCover {
    /// A bare binder / `_` — matches EVERY value, so it shadows all arms after it.
    CatchAll,
    /// A scalar literal (`0`, `true`, `"x"`) — covers exactly that value. A duplicate literal is dead.
    Lit(String),
    /// A variant constructor whose payload sub-patterns are ALL bare binders/wildcards — covers the WHOLE
    /// variant (identified by its discriminant). A second full-cover arm for the same variant is dead. A
    /// variant arm that REFINES its payload (a nested literal/constructor) is NOT this — it covers only
    /// part of the variant, so it never shadows and is never classified (returns `None`).
    Variant(u32),
}

/// Classify a match arm's pattern into the part of the scrutinee it covers, for redundant-arm detection —
/// `None` when the coverage is not decidable from the pattern alone (a guarded arm, a payload-refining
/// variant, a tuple/list/record pattern), so such an arm neither shadows nor is flagged. A GUARDED arm
/// `(guard <pat> <cond>)` is always `None`: its match is conditional on a runtime boolean, so it can
/// never be proven to cover its shape (nor proven dead by an earlier arm).
fn arm_cover(db: &mut Db, pat: StructId) -> Option<ArmCover> {
    // A guarded arm is conditional — never a full cover, never provably dead.
    if db.ast.as_form(pat, "guard").is_some() {
        return None;
    }
    // A bare NAME is a wildcard/binder UNLESS it is a nullary-variant constructor used bare (`None`,
    // `C.Red` reached as a member is a list, but a bare prelude `None` resolves to a variant). A variant
    // name covers only its variant; a true binder covers everything.
    if db.ast.as_name(pat).is_some() {
        if let Some(disc) = crate::eval::variant_disc_of(db, pat) {
            return Some(ArmCover::Variant(disc));
        }
        return Some(ArmCover::CatchAll);
    }
    // A scalar literal pattern (`0`, `true`, `"x"`) covers exactly its value.
    match crate::resolve::resolved_of(db, pat) {
        crate::resolved::Resolved::Int(v) => {
            // A value-unique key: sign + big-endian magnitude bytes (total for any width, unlike to_i64).
            return Some(ArmCover::Lit(format!("i{}:{:x?}", v.negative, v.magnitude)));
        }
        crate::resolved::Resolved::Bool(b) => return Some(ArmCover::Lit(format!("b{b}"))),
        crate::resolved::Resolved::Str(s) => return Some(ArmCover::Lit(format!("s{s}"))),
        _ => {}
    }
    // A constructor-headed pattern `(C.Red)`, `(Some x)`, `((. Sum V) x)`. It covers the WHOLE variant
    // only when every payload sub-pattern is a bare binder/wildcard; a refining sub-pattern (a nested
    // literal/constructor) covers only part, so it is not classified.
    if let crate::ast::Struct::List(children) = db.ast.get(pat) {
        let children = children.to_vec();
        // The ctor head: a bare member `(. Sum V)` used whole is the pattern itself; else the first child.
        let (head, payload_start) = match children.first().copied() {
            Some(first) if db.ast.as_name(first) == Some(".") => (pat, children.len()),
            Some(first) => (first, 1),
            None => return None,
        };
        let disc = crate::eval::variant_disc_of(db, head)?;
        // Every payload sub-pattern must be a bare binder/wildcard for this to be a FULL-variant cover; a
        // refining sub-pattern (not a bare name) covers only part of the variant, so bail to `None`.
        for &sub in &children[payload_start.min(children.len())..] {
            db.ast.as_name(sub)?;
        }
        return Some(ArmCover::Variant(disc));
    }
    None
}

/// Collect REDUNDANT-ARM warnings (CDZ0211) across every `match` in every def body — an arm an EARLIER
/// arm already fully covers, so first-match-wins makes it dead. Walks all user nodes (like the unused-
/// binding pass) rather than only reached bodies, so a redundant arm in an uncalled helper is surfaced
/// too. For each match, scan arms left to right keeping the set of already-covered keys plus whether a
/// catch-all has appeared; an arm whose cover is already subsumed (a prior catch-all shadows anything, a
/// prior identical literal/variant shadows its repeat) warns. Conservative: an unclassifiable arm
/// (`arm_cover` → `None`) neither shadows nor is flagged, so a guarded/refining/tuple arm never yields a
/// false positive.
fn collect_redundant_arm_warnings(db: &mut Db) -> Vec<Diagnostic> {
    use crate::resolved::Resolved;
    let node_count = db.ast.structure.len();
    let mut out = Vec::new();
    for i in 0..node_count {
        let id = StructId(i as u32);
        if !db.is_user_node(id) {
            continue;
        }
        let Resolved::Match { arms, .. } = crate::resolve::resolved_of(db, id) else {
            continue;
        };
        let mut catch_all_seen = false;
        // A HASH SET of already-covered literal/variant keys — an O(1) membership probe per arm. A `Vec`
        // + `contains` was O(covered) per arm → O(arms²) for a match over an N-variant sum (each of N
        // distinct-variant arms scanned the growing covered list).
        let mut covered: std::collections::HashSet<ArmCover> = std::collections::HashSet::new();
        for (pat, _) in &arms {
            let cover = arm_cover(db, *pat);
            let redundant = match &cover {
                // Any arm after a catch-all is unreachable.
                _ if catch_all_seen => true,
                // A repeat of an already-covered literal / full-variant cover.
                Some(c) => covered.contains(c),
                // Unclassifiable — not provably redundant.
                None => false,
            };
            if redundant && db.is_user_node(*pat) {
                let mut diag = Diagnostic::warning(
                    crate::diag::Code::RedundantArm,
                    "this match arm is unreachable — an earlier arm already covers every value it \
                     would match (a duplicate or a pattern shadowed by an earlier catch-all)",
                    Some(*pat),
                );
                // The rustc-gold repair: DELETE the whole `(<pattern> <body>)` arm. An unreachable arm
                // never matches, so removing it is behaviour-preserving (it cannot change which arm runs
                // or any value) — but heuristic, not verified: a redundant arm is often a PATTERN BUG (the
                // author meant a different, reachable pattern), so an agent confirms the delete rather than
                // applying it blind. The delete targets the ARM node (`(pattern body)` list = the pattern's
                // parent), not the pattern alone, so pattern AND body go together. Only when that arm node
                // is itself a user node (an editable source span).
                if let Some(arm) = db.parent_of(*pat)
                    && db.is_user_node(arm)
                {
                    diag = diag.with_fix(&crate::diag::Fix::delete_heuristic(
                        arm,
                        "remove this unreachable arm",
                    ));
                }
                out.push(diag);
            }
            match cover {
                Some(ArmCover::CatchAll) => catch_all_seen = true,
                Some(c) => {
                    covered.insert(c);
                }
                _ => {}
            }
        }
    }
    out
}

/// Collect DEAD-TRAP warnings across the reachable definitions — the non-error diagnostics that ride
/// alongside a produced artifact (`core-semantics.md` §A Trap Occurs Only Where Its Computation Is
/// Observed). Called only on a CLEAN program: `collect_faults` has already proven every REACHED
/// provable trap is a build error (CDZ0304), so any computation that folds to a `ConstTrap` yet
/// survives here was dropped by dead-code elimination — its value is unobserved. Warn at each such
/// drop so a program does not silently discard a computation that could never have produced a value.
///
/// Mirrors `collect_faults`' root set: every def body (a nullary body is walked directly; a
/// parameterized body's drops surface at its nullary call site through the inlining the walk follows).
fn collect_dead_trap_warnings(db: &mut Db) -> Vec<Diagnostic> {
    let bodies: Vec<StructId> = db.defs.iter().filter_map(|d| d.body).collect();
    let mut warnings = Vec::new();
    let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
    for body in bodies {
        walk_for_dead_traps(db, body, &mut warnings, &mut seen);
    }
    warnings
}

/// Walk the RESOLVED tree from `id` looking for a computation the fold DROPPED that PROVABLY traps —
/// an unprojected tuple element, an unreferenced `let` binding, an argument bound to an unused
/// parameter. At each VALUE-DISCARDING position (a tuple element, a record field, a `let` initializer,
/// a call argument) test whether the child folds to a `ConstTrap`; if it does, its trap was elided, so
/// warn (and do NOT descend into it — the outermost dropped trap is the one to report). Elsewhere
/// recurse into the positions a value flows through, so a drop nested inside them is still found.
///
/// CONTROL-FLOW branches (an `if`'s branches, a `match` arm's body) are NOT descended: a trap there is
/// the language's sanctioned laziness (`core-semantics.md` §Conditionals Evaluate One Branch), not a
/// discarded-value defect — warning on it would relitigate a laziness the spec already grants. `seen`
/// dedups (a shared occurrence reached by two paths warns once).
fn walk_for_dead_traps(
    db: &mut Db,
    id: StructId,
    out: &mut Vec<Diagnostic>,
    seen: &mut std::collections::HashSet<u32>,
) {
    use crate::resolved::Resolved;
    if !seen.insert(id.0) {
        return;
    }
    // A child in a value-DISCARDING position: warn if it folds to a provable trap (its value is
    // dropped, so the trap was elided), else recurse to find a drop nested deeper inside it.
    let discarded = |db: &mut Db, child: StructId, out: &mut Vec<Diagnostic>, seen: &mut _| {
        if is_dropped_const_trap(db, child) {
            // Attribute the warning to a USER node — a synthesized/prelude origin has no span. Prefer
            // the dropped computation's own anchor; fall back to the discarding child occurrence.
            let at = dropped_trap_anchor(db, child).filter(|&n| db.is_user_node(n));
            // A dropped computation that has NO VALUE was elided by the fold: either it always TRAPS
            // (`ConstTrap`) or it does not REDUCE to a value — a non-normalizing / explosively-growing
            // term the reduction-work budget stopped (`RecursionBound`). The SAME term whose value is
            // USED is a hard error (CDZ0304 / CDZ0999); dropped, it is dead code, so warn (the likely
            // bug) rather than reject — the DCE consistency the dead-trap warning already applies, now
            // extended to a dead non-normalizing binding (an unused `let` init / discarded argument
            // whose term diverges). One message covers both "no value" reasons.
            let msg = match core_of(db, child) {
                Core::Poison(r) if r.code == Some(Code::RecursionBound) => {
                    "this computation does not reduce to a value (a non-terminating or \
                     explosively-growing reduction) but its value is never used, so it was \
                     eliminated (an unused element, binding, or argument) — likely a bug"
                }
                _ => {
                    "this computation always traps but its value is never used, so it was eliminated \
                     (an unused element, binding, or argument) — likely a bug"
                }
            };
            out.push(Diagnostic::warning(Code::DeadTrap, msg, at));
        } else {
            walk_for_dead_traps(db, child, out, seen);
        }
    };
    match crate::resolve::resolved_of(db, id) {
        // Value-discarding positions: each constituent whose value may be dropped.
        Resolved::Tuple { elems } | Resolved::List { elems } => {
            for e in elems.iter() {
                discarded(db, *e, out, seen);
            }
        }
        // A map literal — each entry's key AND value is a value-flowing position (consumed into the map).
        Resolved::Map { entries } => {
            for &(k, v) in entries.iter() {
                discarded(db, k, out, seen);
                discarded(db, v, out, seen);
            }
        }
        // A `(bin …)` construction — each segment value (and dependent size) is a value-flowing position.
        Resolved::Bin { segs } => {
            for s in segs.iter() {
                discarded(db, s.slot, out, seen);
                match &s.kind {
                    crate::resolved::SegKind::Bytes { size: Some(n) } => {
                        discarded(db, *n, out, seen)
                    }
                    crate::resolved::SegKind::Utf8 { size } => discarded(db, *size, out, seen),
                    _ => {}
                }
            }
        }
        Resolved::Record { fields } => {
            for v in fields.values().copied().collect::<Vec<_>>() {
                discarded(db, v, out, seen);
            }
        }
        Resolved::Let { bindings, body } => {
            for (_, init) in &bindings {
                discarded(db, *init, out, seen);
            }
            walk_for_dead_traps(db, body, out, seen);
        }
        Resolved::Apply { head, args } => {
            walk_for_dead_traps(db, head, out, seen);
            for a in args.iter() {
                discarded(db, *a, out, seen);
            }
        }
        // Value-flowing positions: recurse (a reached trap here already faulted; a nested DROP is still
        // worth finding). A projection reads its operand; an annotation erases; a member reads its
        // operand; a ref follows to its value.
        Resolved::Proj { operand, .. } | Resolved::Member { operand, .. } => {
            walk_for_dead_traps(db, operand, out, seen);
        }
        Resolved::Annot { expr, .. } => walk_for_dead_traps(db, expr, out, seen),
        Resolved::Ref { value } => walk_for_dead_traps(db, value, out, seen),
        // CONTROL FLOW — a branch/arm is sanctioned laziness; do not descend into the guarded parts.
        // The condition/scrutinee IS unconditionally evaluated, so a drop there is worth finding.
        Resolved::If { cond, .. } => walk_for_dead_traps(db, cond, out, seen),
        Resolved::Match { scrutinee, .. } => walk_for_dead_traps(db, scrutinee, out, seen),
        // A short-circuiting connective SHIELDS its right operand exactly as a branch does (sanctioned
        // laziness) — the LEFT operand is unconditionally evaluated (descend), the RIGHT is not. A
        // negation's single operand is unconditionally evaluated.
        Resolved::And { lhs, .. } => walk_for_dead_traps(db, lhs, out, seen),
        Resolved::Not { operand } => walk_for_dead_traps(db, operand, out, seen),
        // Effect control forms decline at lowering (E1a), so no `handle`/`host`/`resume` reaches
        // emission — the dead-trap walk over one is moot (it emits nothing). Treated as non-descending,
        // like any form that lowers to a poison; when E1 lowers them, revisit whether an unconditionally-
        // evaluated sub-position (a handler's init/body) warrants descent.
        Resolved::Handle { .. } | Resolved::Host { .. } | Resolved::Resume { .. } => {}
        // Leaves and non-descending forms.
        Resolved::Int(_)
        | Resolved::Bool(_)
        | Resolved::Str(_)
        | Resolved::SymbolConst(_)
        | Resolved::Bytes(_)
        | Resolved::Char(_)
        | Resolved::Float(_)
        | Resolved::Unit
        | Resolved::Prim(_)
        | Resolved::Param { .. }
        | Resolved::TypeVal(_)
        | Resolved::Lambda { .. }
        | Resolved::SumPayload { .. }
        | Resolved::BinField { .. }
        | Resolved::MapField { .. }
        | Resolved::Poison(_) => {}
    }
}

/// Whether the node at `id` folds to a compile-provable trap (a `ConstTrap` poison). This is the
/// discarded-value test: a child in a value-dropping position that folds to a `ConstTrap` had its trap
/// eliminated (a reached one would have faulted the build in `collect_faults`).
fn is_dropped_const_trap(db: &mut Db, id: StructId) -> bool {
    matches!(
        core_of(db, id),
        Core::Poison(r) if matches!(r.code, Some(Code::ConstTrap) | Some(Code::RecursionBound))
    )
}

/// The node a dropped `ConstTrap` at `id` should be attributed to — the trap's own recorded anchor if
/// it carries one (the precise faulting operation), else the discarded occurrence itself.
fn dropped_trap_anchor(db: &mut Db, id: StructId) -> Option<StructId> {
    match core_of(db, id) {
        Core::Poison(r) => r.at.or(Some(id)),
        _ => Some(id),
    }
}

/// Warn on a binding that is DECLARED but never referenced — a `let` binding, a `fn`/`def` parameter,
/// or a non-exported top-level definition (CDZ0306). Suppressed when the name begins with `_` (the
/// "intentionally unused" convention). A WARNING, never a rejection: an unused binding is well-formed.
///
/// The used-set is the same resolution-column read `UsesOf` performs: a reference resolves to
/// `Resolved::Ref { value }` pointing at the binding's TARGET occurrence (a `let` binding's initializer,
/// a parameter's own occurrence, a def's body). A binder whose target is not in that set is unused.
/// One left-to-right walk of the user nodes collects both the binders (from `let`/scope forms) and the
/// references (`Ref`), so the pass is O(nodes).
fn collect_unused_binding_warnings(db: &mut Db) -> Vec<Diagnostic> {
    use crate::resolved::Resolved;

    // The candidate binders: (name-occ to anchor the warning, TARGET occ a reference resolves to, a
    // human label for the message). Collected across all user nodes.
    struct Binder {
        name_occ: StructId,
        target: StructId,
        name: String,
        kind: &'static str,
        // True when unused-ness is ALREADY decided (params, via `param_is_used`) — reported directly,
        // bypassing the `used`-occ-set gate (a param's occ can spuriously enter `used` when a same-named
        // inner `let` binding's name resolves to it). False for let/def binders (gated on `used`).
        precomputed_unused: bool,
    }
    let mut binders: Vec<Binder> = Vec::new();
    // The occurrences that ARE referenced — every `Ref { value }` a user node resolves to.
    let mut used: std::collections::HashSet<u32> = std::collections::HashSet::new();

    // DECLARATION sites are not uses. A def's NAME occurrence (the `helper` in `(def (helper) …)`)
    // resolves to a `Ref` at its own body — but it is where the def is DECLARED, not a reference to it
    // (the same exclusion `uses_of` makes). Skip a `Ref` originating at one so a def isn't marked used
    // by its own signature.
    let decl_sites: std::collections::HashSet<u32> = db
        .defs
        .iter()
        .filter_map(|d| match db.ast.get(d.sig_occ) {
            crate::ast::Struct::List(kids) => kids.first().map(|k| k.0),
            _ => None,
        })
        .collect();

    let node_count = db.ast.structure.len();
    for i in 0..node_count {
        let id = StructId(i as u32);
        if !db.is_user_node(id) {
            continue;
        }
        if decl_sites.contains(&id.0) {
            continue; // a declaration site's `Ref`-to-own-body is not a use
        }
        match crate::resolve::resolved_of(db, id) {
            // A `let`: each `(name-occ, init-occ)` binding is a binder whose target is its initializer
            // (what a reference resolves to). The binder is used iff some `Ref` points at that init.
            Resolved::Let { bindings, .. } => {
                for (name_occ, init) in bindings.iter() {
                    if let Some(name) = db.ast.as_name(*name_occ) {
                        binders.push(Binder {
                            name_occ: *name_occ,
                            target: *init,
                            name: name.to_string(),
                            kind: "binding",
                            precomputed_unused: false,
                        });
                    }
                }
            }
            // A reference: mark its target used. (A parameter used as a formal is `Param { binder }`
            // at its OWN declaration — that is the declaration, not a use; only a `Ref` counts.)
            Resolved::Ref { value } => {
                used.insert(value.0);
            }
            _ => {}
        }
    }

    // Function/def PARAMETERS. A parameter is NOT checked via the `used`-occ set: for a RECURSIVE
    // function, resolving the body freshens the parameter binder, so a reference resolves to a
    // SYNTHESIZED (non-user) param COPY rather than the original occurrence — the `used` set would miss
    // it and flag a genuinely-used param as unused (the `sm(n)` false positive). Instead a parameter is
    // used iff its NAME appears as a reference IN the def's body that lexically resolves to a parameter
    // — a synthesis-independent, scope-correct check (`param_is_used`).
    for di in 0..db.defs.len() {
        let params = db.defs[di].params.clone();
        let body = db.defs[di].body;
        // The SET of parameter NAMES the body references — collected in ONE walk of the body, not one
        // per-parameter walk (which was O(params × body) = O(N²) for a wide-param def). `param_is_used`'s
        // verdict is purely NAME-based (`resolves_to_param` accepts ANY param, so a body reference marks
        // its parameter used by matching NAME), and a def's parameter names are unique (CDZ0102 rejects a
        // repeated one), so a name-keyed set reproduces the per-parameter check EXACTLY. Skipped entirely
        // when the def has no user parameters to check.
        let mut referenced: std::collections::HashSet<String> = match body {
            Some(b) if !params.is_empty() => used_param_names(db, b),
            _ => std::collections::HashSet::new(),
        };
        // A TYPE-VALUED PARAMETER is used in a SIBLING parameter's ANNOTATION, not the body — `(def (unbox
        // (: t Type) (: b (Box t))) …)` uses `t` only in `(Box t)`. So also scan each parameter's
        // annotation type-expression for param references (`used_param_names` over the `(: name T)`'s `T`),
        // else a genuinely-used type parameter warns spuriously CDZ0306. Cheap: the annotations are tiny.
        if !params.is_empty() {
            for &p in &params {
                if let Some(ty_expr) = db.ast.as_form(p, ":").and_then(|t| t.get(1).copied()) {
                    referenced.extend(used_param_names(db, ty_expr));
                }
            }
        }
        for p in params {
            // The parameter's NAME occurrence (a bare `a` or the inner name of `(: a T)`).
            let name_occ = param_name_occ(db, p);
            let Some(name) = db.ast.as_name(name_occ).map(str::to_string) else {
                continue;
            };
            // A `_`-prefixed param never warns — skip the usage scan (the shared loop also filters `_`,
            // but skipping here avoids the scan cost).
            if name.starts_with('_') {
                continue;
            }
            if !referenced.contains(&name) {
                binders.push(Binder {
                    name_occ,
                    target: name_occ,
                    name,
                    kind: "parameter",
                    precomputed_unused: true, // decided by the reference-name set, not the `used` set
                });
            }
        }
    }

    // A non-exported top-level DEFINITION that nothing references is unused (an exported def is part of
    // the interface — used by definition). A def's target is its body (a `Ref` to a nullary def points
    // at the body) OR — for a def-with-params — the reference resolves to a `Lambda { body }`, which is
    // not a plain `Ref`, so def-with-params usage is tracked via the body appearing... to keep this
    // simple and avoid false "unused" on called functions, only flag NULLARY value defs here; a
    // function's usage is subtle (Lambda) and better covered once needed.
    let exported: std::collections::HashSet<&str> =
        db.exports.iter().map(|e| e.name.as_str()).collect();
    let def_binders: Vec<Binder> = db
        .defs
        .iter()
        .filter(|d| d.params.is_empty()) // nullary value defs only (see note above)
        .filter(|d| !exported.contains(d.name.as_str()))
        .filter_map(|d| {
            let body = d.body?;
            // The def NAME occurrence — the signature's first child (for the warning anchor).
            let name_occ = match db.ast.get(d.sig_occ) {
                crate::ast::Struct::List(kids) => kids.first().copied()?,
                _ => return None,
            };
            Some(Binder {
                name_occ,
                target: body,
                name: d.name.clone(),
                kind: "definition",
                precomputed_unused: false,
            })
        })
        .collect();

    // Emit a warning per unused, non-`_`-prefixed binder, anchored at its name occurrence.
    let mut out = Vec::new();
    for b in binders.into_iter().chain(def_binders) {
        // A precomputed-unused binder (a param) is emitted directly; others are gated on the used set.
        let is_used = !b.precomputed_unused && used.contains(&b.target.0);
        if b.name.starts_with('_') || is_used {
            continue;
        }
        if !db.is_user_node(b.name_occ) {
            continue;
        }
        // The silencing rule is spec-defined and behaviour-preserving — a `_`-prefixed name is the SAME
        // binder, just marked intentionally-unused, and the CDZ0306 check itself suppresses it — so the
        // "prefix with `_`" edit is a VERIFIED fix an agent applies without review (the first
        // machine-applicable fix; `spec/capabilities/diagnostics.md` §A Confirmed Fix Is Marked
        // Verified). It renames the binder's NAME occurrence to `_<name>`.
        let fix = crate::diag::Fix::replace_verified(
            b.name_occ,
            format!("_{}", b.name),
            "prefix with `_` to mark intentionally unused",
        );
        out.push(
            Diagnostic::warning(
                Code::UnusedBinding,
                format!(
                    "unused {}: `{}` is never used (prefix with `_` to silence)",
                    b.kind, b.name
                ),
                Some(b.name_occ),
            )
            .with_fix(&fix),
        );
    }
    out
}

/// Warn on a NON-FINAL form of a sequencing block whose computed value is silently DISCARDED (CDZ0307).
/// A `(do S… tail)` yields ONLY its last form (`core-semantics.md` §A Sequencing Block Evaluates Its Forms
/// In Order), so every earlier form is evaluated for a thrown-away value. In a pure language, dropping the
/// value of a PURE statement that produced one is almost always a bug — the author forgot to bind it, or
/// misplaced an expression. So warn when a non-final statement is (1) a user node, (2) not a declaration
/// (a `def`/`type`/`effect`/`module` binds a name — it is not an evaluated statement), (3) PURE (reaches
/// no host call — nothing observable to sequence for; the SAME `subtree_reaches_host_call` the `do`
/// lowering uses to decide whether to KEEP the statement, so the warning fires on exactly what DCE drops),
/// and (4) has a concrete NON-Unit type (a real value is discarded). A `Unit`-typed statement discards
/// nothing; an effectful one is kept by the `Core::Seq` lowering and is not dead. A WARNING (not a
/// rejection): the block is well-formed and runs correctly. The repair is to DELETE the dead statement
/// (removing a pure, value-discarded, non-final form is behaviour-preserving — the block still yields its
/// last form); if the author meant to observe the value, they bind it with a `let`.
fn collect_discarded_value_warnings(db: &mut Db) -> Vec<Diagnostic> {
    let node_count = db.ast.structure.len();
    let mut out = Vec::new();
    for i in 0..node_count {
        let id = StructId(i as u32);
        if db.ast.head_name(id) != Some("do") {
            continue;
        }
        let Some(forms) = db.ast.as_form(id, "do") else {
            continue;
        };
        // Only the NON-FINAL forms — the last form IS the block's value and is never discarded.
        let Some((_, stmts)) = forms.split_last() else {
            continue;
        };
        for &s in &stmts.to_vec() {
            // A declaration form binds a name; it is not an evaluated statement (its value flows only to a
            // reference, checked there). Skip `def`/`type`/`effect`/`module` — the same forms the `do`
            // lowering and the poison walk skip.
            if matches!(
                db.ast.head_name(s),
                Some("def") | Some("type") | Some("effect") | Some("module")
            ) {
                continue;
            }
            // Only a USER node has a span the warning can anchor to.
            if !db.is_user_node(s) {
                continue;
            }
            // A concrete non-Unit type means a real value was thrown away. `Ty::Unit` discards nothing; a
            // poison (`Ty::Any`) already faulted elsewhere; a free type variable is unresolved — stay
            // conservative on both and do not warn (a false "discarded value" is worse than a missed one).
            // CHECK THIS FIRST (before the host-call walk): the type is already solved by the preceding
            // check, so this is O(1), whereas `subtree_reaches_host_call` recursively walks the statement's
            // whole subtree. Most non-final statements are Unit-typed (a `(host …)`/sequencing effect, an
            // assignment-like op) and short-circuit here — so the expensive walk only runs on a statement
            // that IS a discarded non-Unit value candidate, not on every statement. (Order-independent: both
            // are `continue` guards, so a warning fires iff BOTH pass — reordering changes nothing.)
            let ty = crate::infer::type_of(db, s);
            if matches!(ty, crate::ty::Ty::Unit | crate::ty::Ty::Any) || ty.has_free_var() {
                continue;
            }
            // Effectful statements are KEPT by the lowering (their host call is observable and must run) —
            // sequencing them for effect is exactly why a non-final form is allowed to have a value at all,
            // so they are not dead. Only a PURE statement's discarded value is the defect.
            if crate::lower::subtree_reaches_host_call(db, s) {
                continue;
            }
            // Deleting a pure, value-discarded, non-final form preserves the block's meaning (it still
            // yields its last form) — a heuristic delete because the author may instead have meant to
            // OBSERVE the value (bind it with a `let`), which the compiler cannot decide for them.
            let fix = crate::diag::Fix::delete_heuristic(
                s,
                "remove the discarded statement (or bind its value with a `let`)",
            );
            out.push(
                Diagnostic::warning(
                    Code::DiscardedValue,
                    format!(
                        "this `{}`-typed value is computed but discarded — a non-final form of a \
                         sequencing block is evaluated only for its effect, and this form has none \
                         (bind it with a `let` if you meant to use it, or remove it)",
                        ty.render_name()
                    ),
                    Some(s),
                )
                .with_fix(&fix),
            );
        }
    }
    out
}

/// A parameter occurrence's NAME occurrence — the bare name, or the inner name of an annotated
/// `(: name T)` binder. Mirrors `resolve::param_name` / `db::build_scope_binders` (kept in sync).
fn param_name_occ(db: &Db, param: StructId) -> StructId {
    if db.ast.as_name(param).is_some() {
        return param;
    }
    if let Some(tail) = db.ast.as_form(param, ":")
        && let Some(&name_occ) = tail.first()
    {
        return name_occ;
    }
    param
}

/// Whether the parameter named `name` (declared at `param_occ`) is REFERENCED anywhere in the def
/// body subtree at `body`. Walks the body's USER nodes for a name occurrence equal to `name` (other
/// than the parameter's own declaration) that lexically resolves to a `Param` — confirming it is a
/// use OF THIS parameter, not a same-named inner binding that shadows it. This is synthesis-INDEPENDENT
/// (it keys on the reference's own resolution kind, not the resolved-to occurrence id), so it is not
/// fooled by a recursive function freshening the parameter binder into a synthesized copy.
/// The SET of parameter NAMES that the def body subtree at `body` references — ONE structural walk
/// collecting every name occurrence that resolves to a parameter, so the wide-param unused check is O(body)
/// once, not O(body) per parameter (which was O(params × body) = O(N²) for a def with many parameters). A
/// name is collected iff it is a genuine REFERENCE resolving to a parameter — NOT a `let` binding's name
/// position (a same-named inner `let` binder resolves to the outer param but is a declaration, not a use,
/// so a param shadowed by it is NOT used). `resolves_to_param` accepts a `Param` or a `Ref`-to-`Param`
/// (recursion may freshen the binder → the resolution KIND is the signal, not the target occ). This
/// reproduces the old per-parameter `param_is_used` verdict EXACTLY: it was name-based (matched a param's
/// NAME anywhere in the body), and a def's parameter names are unique (CDZ0102). The old `id != param_occ`
/// declaration guard is subsumed — a parameter's own declaration occurrence lives in the SIGNATURE, not in
/// the body walked here.
fn used_param_names(db: &mut Db, body: StructId) -> std::collections::HashSet<String> {
    fn walk(db: &mut Db, id: StructId, out: &mut std::collections::HashSet<String>) {
        if let Some(name) = db.ast.as_name(id).map(str::to_string)
            && !is_let_binding_name(db, id)
            && resolves_to_param(db, id)
        {
            out.insert(name);
        }
        // Recurse into children (a user node's subtree). Clone the child list to avoid holding a
        // borrow across the recursive `&mut` call.
        if let crate::ast::Struct::List(kids) = db.ast.get(id) {
            for c in kids.clone() {
                walk(db, c, out);
            }
        }
    }
    let mut out = std::collections::HashSet::new();
    walk(db, body, &mut out);
    out
}

/// Whether `id` resolves to a parameter — its own `Param`, or a `Ref` to one (a body reference is a
/// `Ref`; recursion may freshen the binder, so the resolution KIND is the signal, not the target occ).
fn resolves_to_param(db: &mut Db, id: StructId) -> bool {
    match crate::resolve::resolved_of(db, id) {
        crate::resolved::Resolved::Param { .. } => true,
        crate::resolved::Resolved::Ref { value } => {
            matches!(
                crate::resolve::resolved_of(db, value),
                crate::resolved::Resolved::Param { .. }
            )
        }
        _ => false,
    }
}

/// Whether `id` is a `let` binding's NAME position — the first element of a 2-element `(name init)`
/// pair whose parent is a `let`'s bindings-list. Such an occurrence is a BINDER (a declaration), not a
/// reference, so it must not count as a use of an outer same-named parameter.
fn is_let_binding_name(db: &Db, id: StructId) -> bool {
    let Some(pair) = db.parent_of(id) else {
        return false;
    };
    // `id` is the pair's first child.
    let is_first =
        matches!(db.ast.get(pair), crate::ast::Struct::List(kv) if kv.first() == Some(&id));
    if !is_first {
        return false;
    }
    // The pair's parent is a bindings-list; that list's parent is a `let` whose first tail element it is.
    let Some(list) = db.parent_of(pair) else {
        return false;
    };
    db.parent_of(list)
        .and_then(|lt| {
            db.ast
                .as_form(lt, "let")
                .map(|t| t.first().copied() == Some(list))
        })
        .unwrap_or(false)
}

/// The program's name for artifact labelling — the first exported name, or "main". (A cosmetic label;
/// the artifact's identity is its kind + bytes.)
/// Decode the `ast` input artifact(s) into ONE arena plus optional LINKAGE — the front-end link step
/// (`DESIGN-package-linking.md`). A SINGLE `ast` decodes directly with `None` linkage, byte-identically
/// to the pre-linking path (flat namespace, no synthesized-root splice, no entry needed). TWO OR MORE
/// `ast` artifacts, OR a single `ast` accompanied by an explicit `entry` marker, is a PACKAGE: the files
/// are spliced by `link()` under a synthesized `(do …)` root, `entry` names which file's exports form
/// the component boundary, and the returned `Some(Linkage)` makes name resolution FILE-SCOPED.
///
/// A package with no named entry declines (there is no rule to pick one — reject, don't guess), except
/// the degenerate single-file package, whose lone file IS the entry. A decode failure of any file, or
/// an entry naming no supplied file, declines with a specific diagnostic.
fn link_inputs(
    ast_arts: &[&Artifact],
    entry_name: Option<&str>,
) -> Result<(crate::ast::Arenas, Option<crate::link::Linkage>), Reject> {
    match ast_arts {
        // No `ast` artifact in the input list — the source tree the tool requires to derive a component
        // is absent, so this is a diagnostic (`compile` turns the `Reject` into an error `Diagnostic`),
        // never an empty or arbitrary component.
        //= spec/contracts/build-tool-interface.md#the-tool-s-inputs-are-a-kinded-artifact-list
        //# An input artifact list that omits the source tree the tool requires to derive a component MUST be reported as a diagnostic rather than producing an empty or arbitrary output.
        [] => Err(Reject::decline("no `ast` input artifact")),
        // The overwhelmingly common case: exactly one file, no package framing. Decode it as-is — flat
        // namespace, no linkage — so a one-file program compiles through the identical path it always
        // did.
        [only] if entry_name.is_none() => crate::codec::decode(&only.bytes)
            .map(|a| (a, None))
            .ok_or_else(|| Reject::decline("binary AST failed to decode")),
        // A package: decode every file, then splice. The entry defaults to the sole file's name when
        // exactly one file was supplied (a single-file package needs no explicit entry); otherwise the
        // caller must name the entry. A single-file package still carries linkage (its `(import …)`
        // clauses, if any, are validated), but with one file there is no cross-file scoping to enforce.
        _ => {
            let mut files = Vec::with_capacity(ast_arts.len());
            for art in ast_arts {
                let arena = crate::codec::decode(&art.bytes).ok_or_else(|| {
                    Reject::decline(format!("binary AST for `{}` failed to decode", art.name))
                })?;
                files.push((art.name.clone(), arena));
            }
            let entry = match entry_name {
                Some(e) => e.to_string(),
                None if files.len() == 1 => files[0].0.clone(),
                None => {
                    return Err(Reject::decline(
                        "a multi-file package needs an `entry` input artifact naming the entry file",
                    ));
                }
            };
            let linked = crate::link::link(&files, &entry)?;
            let linkage = linked.linkage();
            Ok((linked.arenas, Some(linkage)))
        }
    }
}

fn program_name(db: &Db) -> String {
    db.exports
        .first()
        .map(|e| e.name.clone())
        .unwrap_or_else(|| "main".to_string())
}

/// A failed compilation: no artifacts, one error diagnostic per reject.
fn fail(rejects: Vec<Reject>) -> CompileOutput {
    fail_with(Vec::new(), rejects)
}

/// A failed emit that STILL carries any query artifacts computed before the failure. A query is a
/// pure, total fact read that never denies an artifact (`DESIGN-sidecar-api.md`), so a `TypeOf` /
/// `UsesOf` answer rides alongside the failure diagnostics — the caller gets the facts it asked for
/// even for a program that does not compile. With no query artifacts this is exactly `fail`.
fn fail_with(query_artifacts: Vec<Artifact>, rejects: Vec<Reject>) -> CompileOutput {
    CompileOutput {
        artifacts: query_artifacts,
        diagnostics: rejects.iter().map(Diagnostic::from_reject).collect(),
    }
}
