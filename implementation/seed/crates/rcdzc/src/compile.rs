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
use crate::layout::{self, Layout};
use crate::lower::core_of;
use tracing::trace;

/// Compile a set of kinded input artifacts to the requested targets. The `ast` input is decoded into
/// a `Db`; each target's backend fills its artifact from the shared columns. Targets default to
/// `[Wasm]` at the CLI, not here — this entry emits exactly what it is asked for.
pub fn compile(inputs: &[Artifact], targets: &[Target]) -> CompileOutput {
    trace!(target: "rcdzc::compile", inputs = inputs.len(), targets = targets.len(), "compile requested");
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
    trace!(target: "rcdzc::compile", defs = db.defs.len(), exports = db.exports.len(), "loaded program");

    // Compute the boundary layout once (target-neutral). A program with no export declines.
    let layout = match layout::compute(&mut db) {
        Ok(l) => l,
        Err(r) => {
            trace!(target: "rcdzc::compile", reason = %r.message, "layout declined");
            return fail(vec![r]);
        }
    };

    // Collect every reached fault across the reachable definitions, module-wide (report ALL, not just
    // the first — `compiler-pipeline.md` §Phases Recover From Errors).
    let mut faults = collect_faults(&mut db, &layout);
    if !faults.is_empty() {
        trace!(target: "rcdzc::compile", faults = faults.len(), "compilation FAILED (faults reported, no artifact)");
        for f in &mut faults {
            sanitize_origin(&db, f);
        }
        return fail(faults);
    }
    trace!(target: "rcdzc::compile", targets = targets.len(), "program clean — emitting artifacts");

    // WARNINGS (non-error; ride alongside the artifact). The program is clean — every REACHED provable
    // trap already faulted above — so a computation that PROVES it traps yet survives to here was
    // dropped by dead-code elimination (its value is unobserved: an unprojected element, an unreferenced
    // binding, an unused argument). That is conformant (`core-semantics.md` §A Trap Occurs Only Where
    // Its Computation Is Observed) but almost always a defect, so warn — the build still succeeds.
    let mut diagnostics = collect_dead_trap_warnings(&mut db);

    // Clean: ask each requested target's backend to fill its artifact.
    let mut artifacts = Vec::new();
    for &target in targets {
        match backend::emit(target, &mut db, &layout) {
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

/// Every fault across the program's definitions. Well-formedness — scope resolution and type
/// checking — is UNCONDITIONAL: it holds over EVERY top-level definition's body, not only the ones
/// reachable from an export, because a program is well-formed or not regardless of what is asked to
/// compile (`core-semantics.md` §Binding Is Lexical — the unbound-name rule is not gated on
/// reachability; an ill-formed uncalled sibling definition is still rejected). Emission stays
/// reachability-driven (the `layout` decides what is emitted); only well-formedness is total.
fn collect_faults(db: &mut Db, _layout: &Layout) -> Vec<Reject> {
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
        Core::Tuple { elems } | Core::ListNew { elems } => {
            for e in elems {
                collect_reached_poisons(db, e, out);
            }
        }
        Core::Proj { operand, .. } | Core::ListLen { operand } => {
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
        // A parameter or let-binding reference is a runtime local read — no sub-poison to collect.
        Core::LocalRef { .. }
        | Core::Param { .. }
        | Core::ConstInt(_)
        | Core::ConstBool(_)
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
        // Leaves and non-descending forms.
        Resolved::Int(_)
        | Resolved::Bool(_)
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
fn program_name(db: &Db) -> String {
    db.exports
        .first()
        .map(|e| e.name.clone())
        .unwrap_or_else(|| "main".to_string())
}

/// A failed compilation: no artifacts, one error diagnostic per reject.
fn fail(rejects: Vec<Reject>) -> CompileOutput {
    CompileOutput {
        artifacts: Vec::new(),
        diagnostics: rejects.iter().map(Diagnostic::from_reject).collect(),
    }
}
