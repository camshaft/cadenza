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
    // the first — `compiler-pipeline.md` §Phases Recover From Errors).
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
pub fn compile_component(ast_bytes: &[u8]) -> Result<Vec<u8>, Diagnostic> {
    let out = compile(
        &[Artifact::new(
            Artifact::KIND_AST,
            "main",
            ast_bytes.to_vec(),
        )],
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
                node: None,
            })),
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
    let mut faults = collect_faults(db);
    for f in &mut faults {
        sanitize_origin(db, f);
    }
    faults.iter().map(Diagnostic::from_reject).collect()
}

/// Every fault across the program's definitions. Well-formedness — scope resolution and type
/// checking — is UNCONDITIONAL: it holds over EVERY top-level definition's body, not only the ones
/// reachable from an export, because a program is well-formed or not regardless of what is asked to
/// compile (`core-semantics.md` §Binding Is Lexical — the unbound-name rule is not gated on
/// reachability; an ill-formed uncalled sibling definition is still rejected). Emission stays
/// reachability-driven (the `layout` decides what is emitted); only well-formedness is total.
fn collect_faults(db: &mut Db) -> Vec<Reject> {
    let mut faults = Vec::new();
    // UNMODELED TOP-LEVEL FORM. A top-level declaration the compiler does not model — `(effect …)`,
    // `(pragma …)` — makes the whole program decline (decline-don't-miscompile): compiling the rest as
    // if the declaration were absent would silently drop its meaning (e.g. an `(effect E …)` whose
    // duplicate operation goes unchecked, then `main` runs). Reported here so the reject anchors a node.
    for (head, occ) in db.unknown_top_forms() {
        faults.push(
            Reject::decline(format!(
                "`{head}` is not a construct this compiler models — the program cannot be compiled"
            ))
            .at(occ),
        );
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
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for &p in params {
            let name_occ = crate::eval::param_name_occ(db, p);
            let Some(name) = db.ast.as_name(name_occ).map(|s| s.to_string()) else {
                continue; // a param with no extractable name (a malformed binder) — not a dup check
            };
            if !seen.insert(name.clone()) {
                faults.push(
                    Reject::coded(
                        Code::NonLinearBinder,
                        format!(
                            "parameter `{name}` is bound more than once (a parameter list must be \
                             linear, like a pattern)"
                        ),
                    )
                    .at(name_occ),
                );
            }
        }
    }
    // Check EVERY definition's body — reachable or not. (The demand is still lazy per node; this just
    // demands each definition once, which is what well-formedness requires.)
    let bodies: Vec<(StructId, bool)> = db
        .defs
        .iter()
        .filter_map(|d| d.body.map(|b| (b, d.params.is_empty())))
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
    faults
}

/// Collect poisons reached UNCONDITIONALLY from `id`. Descends the core form into positions a value is
/// unconditionally used (an `if` CONDITION), but NOT into a conditional's branches — a poison shielded
/// by an untaken branch is not a build failure. Reads the core column on demand.
fn collect_reached_poisons(db: &mut Db, id: StructId, out: &mut Vec<Reject>) {
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
        // Both operands of a runtime arithmetic or comparison op are unconditionally evaluated.
        Core::Arith { lhs, rhs, .. } | Core::Compare { lhs, rhs, .. } => {
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
        // A parameter or let-binding reference is a runtime local read — no sub-poison to collect.
        Core::LocalRef { .. }
        | Core::Param { .. }
        | Core::ConstInt(_)
        | Core::ConstBool(_)
        | Core::ConstStr(_)
        | Core::Unit => {}
    }
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
        | Resolved::Unit
        | Resolved::Prim(_)
        | Resolved::Param { .. }
        | Resolved::TypeVal(_)
        | Resolved::Lambda { .. }
        | Resolved::SumPayload { .. }
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
