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
//! failing the build (`reference-compiler.md` §Reachability Is A Consequence Of Reduction).

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

    // Compute the boundary layout once (target-neutral). A program with no export declines.
    let layout = match layout::compute(&mut db) {
        Ok(l) => l,
        Err(r) => {
            trace!(target: "rcdzc::compile", reason = %r.message, "layout declined");
            return fail_with(query_artifacts, vec![r]);
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
    // binding, an unused argument). That is conformant (`core-semantics.md` §A Trap Occurs Only Where
    // Its Computation Is Observed) but almost always a defect, so warn — the build still succeeds.
    let mut diagnostics = collect_dead_trap_warnings(&mut db);
    // Unused-binding warnings (a let binding / parameter / non-exported def nothing references, unless
    // `_`-prefixed) ride alongside the artifact too — well-formed, just likely a defect (CDZ0306).
    diagnostics.extend(collect_unused_binding_warnings(&mut db));
    // Redundant-match-arm warnings (an arm an earlier arm already covers — CDZ0211): dead code, like an
    // unused binding, so a warning that rides alongside the artifact without denying it.
    diagnostics.extend(collect_redundant_arm_warnings(&mut db));

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
    out
}

/// The FIXED registry of module-directive keys the specification defines (`modules-and-namespaces.md` §A
/// Module Directive Is Drawn From A Fixed Set). The single source of truth for BOTH the `(pragma …)`
/// validation (a key not here is CDZ0601) and the "did you mean?" suggestion an unknown key gets — so the
/// suggestion can never drift from the accepted set. Small and closed today (`default-integer`); a new
/// spec directive adds its key here.
const PRAGMA_REGISTRY: &[&str] = &["default-integer"];

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
fn collect_faults(db: &mut Db) -> Vec<Reject> {
    let mut faults = Vec::new();
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
        let hint = match crate::diag::suggest::nearest(&head, &defined_names) {
            Some(near) => format!(" — did you mean `{near}`?"),
            None => String::new(),
        };
        faults.push(
            Reject::decline(format!(
                "unbound name `{head}` at the top level{hint} (if `{head}` is meant as a declaration, \
                 it is not one this compiler models — the program cannot be compiled either way)"
            ))
            .at(occ),
        );
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
    for form in (0..db.ast.structure.len() as u32).map(StructId) {
        let Some(ptail) = db.ast.as_form(form, "pragma") else {
            continue;
        };
        let key = ptail.first().and_then(|&k| db.ast.as_name(k));
        match key {
            // `default-integer <T>` — exactly one argument (the default type). Missing/extra → malformed.
            Some("default-integer") => {
                if ptail.len() != 2 {
                    faults.push(
                        Reject::coded(
                            Code::MalformedDirective,
                            "`default-integer` takes exactly one type argument (e.g. `(pragma default-integer Int64)`)",
                        )
                        .at(form),
                    );
                }
            }
            // A key the fixed registry does not define — rejected, not ignored. If the typo'd key is a
            // near-miss for a registry key (`default-integr` → `default-integer`), name it AND carry a
            // replace fix on the KEY occurrence — the same closed-set "did you mean?" an unbound name /
            // absent field / undeclared handler op gets (`spec/capabilities/diagnostics.md` §A Diagnostic
            // Carries A Route To A Fix). The candidate pool is the registry itself, so a suggestion can
            // never name a key the validator would then reject.
            Some(other) => {
                let mut reject = Reject::coded(
                    Code::UnknownDirective,
                    format!(
                        "`{other}` is not a module directive this specification defines (the pragma \
                         registry is a fixed set; an unknown key is rejected, not ignored)"
                    ),
                )
                .at(form);
                if let Some(candidate) =
                    crate::diag::suggest::nearest(other, PRAGMA_REGISTRY.iter().copied())
                    && let Some(&key_occ) = ptail.first()
                {
                    reject = reject.with_fix(crate::diag::Fix::replace_heuristic(key_occ, candidate));
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
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let dups: Vec<(String, StructId)> = db
        .defs
        .iter()
        .filter(|d| !d.name.is_empty() && !seen.insert(d.name.as_str()))
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
    let mut seen_exports: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let dup_exports: Vec<(String, StructId)> = db
        .exports
        .iter()
        .filter(|e| !seen_exports.insert(e.name.as_str()))
        .map(|e| (e.name.clone(), e.occ))
        .collect();
    for (name, occ) in dup_exports {
        faults.push(
            Reject::coded(
                Code::Malformed,
                format!("`{name}` is exported more than once (a module has a fixed set of names)"),
            )
            .at(occ),
        );
    }
    // DUPLICATE VARIANT. A sum type `(type T (A …) (A …))` declares its variant NAMES as a fixed SET
    // (core-semantics.md #The Structural Types Are Record, Tuple, And Sum: a sum's shape is its variant
    // names with their payload types), so naming a variant twice is the SAME duplicate-member
    // ill-formedness a record with a duplicate field, a module with a duplicate definition, and a
    // duplicate export are rejected for (CDZ0201) — the fourth closed name-set. Each variant after the
    // first with a given name (WITHIN one type declaration) is reported, anchored at its name
    // occurrence. (Two different types may reuse a variant name — the set is per-declaration.)
    for ty in &db.type_decls {
        let mut seen_variants: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for variant in &ty.variants {
            if !seen_variants.insert(variant.name.as_str()) {
                faults.push(
                    Reject::coded(
                        Code::Malformed,
                        format!(
                            "variant `{}` is declared more than once in sum `{}` (a sum has a \
                             fixed set of variant names)",
                            variant.name, ty.name
                        ),
                    )
                    .at(variant.name_occ),
                );
            }
        }
    }
    // DUPLICATE EFFECT OPERATION. An effect `(effect E (op f …) (op f …))` declares its operation NAMES
    // as a fixed SET (capabilities-and-effects.md §An Effect Declaration Names The Effect And Types Its
    // Operations: each name is bound to ONE operation type), so naming an operation twice is the SAME
    // duplicate-member ill-formedness a record field, a module definition, an export, and a sum variant
    // are rejected for (CDZ0201) — the fifth closed name-set. Each operation after the first with a given
    // name (WITHIN one effect declaration) is reported, anchored at its name occurrence. (Two different
    // effects may reuse an operation name — the set is per-declaration, since an operation is reached
    // through its declaring effect.)
    for eff in &db.effect_decls {
        let mut seen_ops: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for op in &eff.ops {
            if !seen_ops.insert(op.name.as_str()) {
                faults.push(
                    Reject::coded(
                        Code::Malformed,
                        format!(
                            "operation `{}` is declared more than once in effect `{}` (an effect has \
                             a fixed set of operation names)",
                            op.name, eff.name
                        ),
                    )
                    .at(op.name_occ),
                );
            }
        }
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
    // DUPLICATE PARAMETER NAME. A function's parameter list is a BINDER POSITION, so it must be LINEAR
    // exactly as a pattern is (core-semantics.md §Patterns Compose: "A pattern MUST bind each name at
    // most once … rather than silently shadowing an earlier binder"). `(def (f x x) …)` binds `x` twice;
    // accepting it last-wins makes the FIRST parameter — and any argument passed to it — silently
    // unreachable (its value, and any trap it would raise, dropped). Reject the SECOND+ occurrence of a
    // name (CDZ0102, the non-linear-binder code the spec assigns), anchored at the repeated binder. Per
    // def (a name may of course repeat ACROSS defs); the binder NAME sees through a `(: name T)` binder.
    let param_lists: Vec<Vec<StructId>> = db.defs.iter().map(|d| d.params.clone()).collect();
    for params in &param_lists {
        // All param names of this list — the set the rename fix must avoid so a fresh name collides with
        // neither an earlier NOR a later parameter (renaming `x` in `(f x x)` to `x2` must dodge a real `x2`).
        let all_names: std::collections::HashSet<String> = params
            .iter()
            .filter_map(|&p| db.ast.as_name(crate::eval::param_name_occ(db, p)).map(str::to_string))
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
    let bodies: Vec<(StructId, bool)> = db
        .defs
        .iter()
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
        if let crate::resolved::Resolved::Lambda { body: closure_body, .. } =
            crate::resolve::resolved_of(db, body)
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
    let missing_exports: Vec<(String, StructId)> = db
        .exports
        .iter()
        .filter(|e| e.def.is_none())
        .map(|e| (e.name.clone(), e.occ))
        .collect();
    for (name, occ) in missing_exports {
        let msg = match crate::diag::suggest::nearest(&name, &defined_names) {
            Some(near) => format!("export `{name}` names no definition — did you mean `{near}`?"),
            None => format!("export `{name}` names no definition"),
        };
        faults.push(Reject::coded(Code::Unbound, msg).at(occ));
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
    dedup_faults(faults)
}

/// Collapse duplicate faults — the SAME issue reported by more than one collection pass. A fault is
/// keyed by `(code, anchor node)`: the type-check walk and the reached-poison walk both visit an
/// unconditionally-evaluated position, so an unbound name (or any fault) in a REACHABLE spot is found
/// by both and would otherwise be reported twice at the same spot. Two faults with the same code AND
/// the same anchor are the one issue bubbling up along two paths — keep the first (stable order),
/// drop the rest. DISTINCT occurrences (same code, DIFFERENT node — e.g. two separate unbound uses)
/// are NOT duplicates and both survive. An UNANCHORED fault (`at == None`) dedups by code+message, so
/// two different unanchored declines still both show.
fn dedup_faults(faults: Vec<Reject>) -> Vec<Reject> {
    // If any CDZ0401 (an ungranted effect reached with no home) was produced, the emit path's UNCODED
    // "performed with no enclosing handler here" DECLINE is the same root cause reported more weakly —
    // drop it so one ungranted effect yields ONE primary `error:` (the coded CDZ0401), not a coded
    // rejection shadowed by an `error:` decline (`reference-compiler.md` §Outcomes Are Ordered By
    // Safety). Only suppressed WHEN a CDZ0401 exists — a standalone perform with no entrypoint check
    // covering it (should not happen for an exported body, but defensively) keeps its decline.
    let has_no_home_reject = faults.iter().any(|r| r.code == Some(Code::EffectNoHome));
    // Likewise: the emit path's uncoded "value is not applyable" DECLINE is redundant when `infer`
    // proved the head a definite non-function (the coded `cannot apply a value of type … — it is not a
    // function` reject). Drop the weaker decline so applying a non-function is ONE primary `error:`.
    let has_not_a_function_reject = faults
        .iter()
        .any(|r| r.code.is_some() && r.message.starts_with(crate::diag::NOT_A_FUNCTION_PREFIX));
    // Likewise: the evaluator's uncoded "applied more arguments than the function accepts" DECLINE is
    // redundant when `infer` proved the over-application (the coded CDZ0203 `applied N arguments to a
    // function of arity M …` reject). Drop the weaker decline so over-application is ONE primary error.
    let has_over_application_reject = faults
        .iter()
        .any(|r| r.code.is_some() && r.message.contains(crate::diag::OVER_APPLICATION_MARKER));
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
    // Likewise: a NON-CANONICAL handle (the retired effect-name-less shape) is rejected at resolve time
    // (`resolve_noncanonical_handle`, a CDZ0201). Because the handle never resolved as a handler, its
    // body's perform is seen by the entrypoint no-home walk as reached with NO enclosing handler → a
    // CONSEQUENT CDZ0401. That misdirects (the author DID write a handler — it is just not canonical), so
    // drop the CDZ0401 whenever the non-canonical reject is present, keeping the CDZ0201 that says how to
    // fix the handle as the ONE primary error. Matched by message prefix (the reject reuses `Malformed`).
    let has_noncanonical_handle_reject = faults.iter().any(|r| {
        r.code == Some(Code::Malformed)
            && r.message.starts_with(crate::diag::HANDLE_NONCANONICAL_PREFIX)
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
    // A "record has no field `k`" fault reported by BOTH the infer member check (with a did-you-mean
    // fix) AND the emit-side member fold, at two DIFFERENT nodes (the `.k` projection vs an enclosing
    // `(R.k …)` apply), is ONE fault shown twice. The messages are IDENTICAL up to the fix suffix, so key
    // the duplicate by the `record has no field \`k\`` core (prefix + the backticked key, which both
    // versions carry): if ANY copy carries a fix (the infer one), drop the OTHER anchored copies of the
    // same field-fault. Keeps the richer, fix-bearing report; collapses the bare duplicate. Narrow to the
    // no-field family so two genuinely-distinct same-message faults elsewhere are never merged.
    fn no_field_key(msg: &str) -> Option<&str> {
        // The invariant core is `record has no field \`k\`` — strip an optional ` — did you mean …?` tail.
        msg.strip_prefix(crate::diag::NO_FIELD_PREFIX)
            .map(|rest| rest.split(" — ").next().unwrap_or(rest))
    }
    // A field-fault CORE the program reports WITH a fix (the infer did-you-mean copy) — its fix-less twin
    // is dropped below (keep the richer copy).
    let fixed_field_cores: std::collections::HashSet<&str> = faults
        .iter()
        .filter(|r| r.fix.is_some())
        .filter_map(|r| no_field_key(&r.message))
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
            if has_malformed_handler_reject
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
            // A CDZ0401 (no home) that is the CONSEQUENCE of a non-canonical handle failing to resolve as
            // a handler — drop it in favor of the CDZ0201 that reports the real, fixable defect.
            if has_noncanonical_handle_reject && r.code == Some(Code::EffectNoHome) {
                return false;
            }
            // An unanchored fault that also appears ANCHORED (same code + message) is that fault minus
            // its location — drop it, the anchored copy already carries the issue with a line:col.
            if r.at.is_none() && anchored_keys.contains(&(r.code, r.message.as_str())) {
                return false;
            }
            // A DECLINE anchored at a node that also carries a CODED reject is shadowed by it — drop it.
            if r.is_decline() && r.at.is_some_and(|s| coded_nodes.contains(&s.0)) {
                return false;
            }
            // A FIX-LESS "record has no field `k`" copy whose SAME field-fault appears WITH a fix
            // elsewhere (infer's did-you-mean copy) is the emit-side duplicate — drop it, keep the fix.
            // (Scoped to the fix-vs-no-fix pair, so two DISTINCT same-name field faults — neither with a
            // fix — are never merged; the `R.make`-with-no-near-field duplicate is handled at its source
            // in `lower`, not here.)
            if r.fix.is_none()
                && no_field_key(&r.message).is_some_and(|k| fixed_field_cores.contains(k))
            {
                return false;
            }
            // An anchored fault is identified by (code, node); an unanchored one by (code, message)
            // so distinct declines with no node still both appear.
            let msg_key = r.at.is_none().then(|| r.message.clone());
            seen.insert((r.code, r.at.map(|s| s.0), msg_key))
        })
        .cloned()
        .collect()
}

/// Collect poisons reached UNCONDITIONALLY from `id`. Descends the core form into positions a value is
/// unconditionally used (an `if` CONDITION), but NOT into a conditional's branches — a poison shielded
/// by an untaken branch is not a build failure. Reads the core column on demand.
fn collect_reached_poisons(db: &mut Db, id: StructId, out: &mut Vec<Reject>) {
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
            for (_, value) in fields {
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
        Core::BytesConcat { lhs, rhs } => {
            collect_reached_poisons(db, lhs, out);
            collect_reached_poisons(db, rhs, out);
        }
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
#[derive(PartialEq, Eq)]
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
        let mut covered: Vec<ArmCover> = Vec::new();
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
                out.push(Diagnostic::warning(
                    crate::diag::Code::RedundantArm,
                    "this match arm is unreachable — an earlier arm already covers every value it \
                     would match (a duplicate or a pattern shadowed by an earlier catch-all)",
                    Some(*pat),
                ));
            }
            match cover {
                Some(ArmCover::CatchAll) => catch_all_seen = true,
                Some(c) if !covered.contains(&c) => covered.push(c),
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
            // the trap's own anchor; fall back to the discarding child occurrence.
            let at = dropped_trap_anchor(db, child).filter(|&n| db.is_user_node(n));
            out.push(Diagnostic::warning(
                Code::DeadTrap,
                "this computation always traps but its value is never used, so it was eliminated \
                 (an unused element, binding, or argument) — likely a bug",
                at,
            ));
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
                if let crate::resolved::SegKind::Bytes { size: Some(n) } = &s.kind {
                    discarded(db, *n, out, seen);
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
    matches!(core_of(db, id), Core::Poison(r) if r.code == Some(Code::ConstTrap))
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
            let used_in_body = body.is_some_and(|b| param_is_used(db, b, name_occ, &name));
            if !used_in_body {
                binders.push(Binder {
                    name_occ,
                    target: name_occ,
                    name,
                    kind: "parameter",
                    precomputed_unused: true, // decided by `param_is_used`, not the `used` set
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
fn param_is_used(db: &mut Db, body: StructId, param_occ: StructId, name: &str) -> bool {
    // The body subtree's node range: a def body and everything under it. The arena is built so a
    // subtree's descendants are not contiguous by id, so walk structurally.
    fn walk(db: &mut Db, id: StructId, param_occ: StructId, name: &str) -> bool {
        // A matching name occurrence (not the declaration) that is a genuine REFERENCE — not itself a
        // BINDER position (a `let` binding's name resolves to the outer param too, but it is a
        // declaration, not a use — so a param shadowed by a same-named inner `let` is NOT "used"). A
        // reference resolves to a `Param` (its own) or a `Ref`-to-`Param` (recursion may freshen the
        // binder → the KIND, not the target occ, is the signal).
        if id != param_occ
            && db.ast.as_name(id) == Some(name)
            && !is_let_binding_name(db, id)
            && resolves_to_param(db, id)
        {
            return true;
        }
        // Recurse into children (a user node's subtree). Clone the child list to avoid holding a
        // borrow across the recursive `&mut` call.
        if let crate::ast::Struct::List(kids) = db.ast.get(id) {
            for c in kids.clone() {
                if walk(db, c, param_occ, name) {
                    return true;
                }
            }
        }
        false
    }
    walk(db, body, param_occ, name)
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
