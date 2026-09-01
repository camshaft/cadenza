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
use crate::ast::{CompoundCtor, StructId};
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
    // The default optimization level (`OptLevel::default()` = `O1`) — the fast-ish common case. A caller
    // that wants a specific level (the `cdz compile --opt-level` flag, a `Project.cdz` release profile)
    // uses `compile_with_opt` instead; this thin wrapper keeps every existing caller unchanged.
    compile_with_opt(inputs, targets, crate::opt::OptLevel::default())
}

/// The optimization-level-parameterized compile entry — identical to [`compile`] but the caller chooses
/// the [`crate::opt::OptLevel`]. This is the sink for v-cdz-tooling's `cdz compile --opt-level` flag and
/// a `Project.cdz` profile: the requested level selects which backend-independent Core passes run (via
/// the [`crate::opt::PassManager`]), with observably-IDENTICAL behavior at every level — only compile
/// time / output speed differ. `compile(inputs, targets)` is exactly `compile_with_opt(inputs, targets,
/// OptLevel::default())`.
pub fn compile_with_opt(
    inputs: &[Artifact],
    targets: &[Target],
    opt_level: crate::opt::OptLevel,
) -> CompileOutput {
    // The default GLOBAL overflow policy (`OverflowSpec::default()` = None/None → the built-in `Trap`, per
    // the numeric model). A caller with a `Project.cdz` overflow global uses `compile_with_opt_and_overflow`
    // directly; this keeps every existing caller unchanged.
    compile_with_opt_and_overflow(
        inputs,
        targets,
        opt_level,
        crate::db::OverflowSpec::default(),
    )
}

/// The compile entry parameterized by BOTH the optimization level AND the GLOBAL overflow policy — the
/// sink for a `Project.cdz` `def overflow-signed`/`overflow-unsigned` manifest global (`numeric-model.md`
/// §Overflow precedence: module `(pragma overflow …)` > this global manifest default > the built-in
/// `Trap`). The `overflow` [`crate::db::OverflowSpec`] seeds `db.global_overflow`, which
/// `infer::overflow_mode_of` reads (via `Db::global_overflow_default`) for an arith node with NO
/// governing module spec. `compile_with_opt(inputs, targets, level)` is exactly this with
/// `OverflowSpec::default()` (None/None), so the module-pragma and no-policy paths are byte-identical.
pub fn compile_with_opt_and_overflow(
    inputs: &[Artifact],
    targets: &[Target],
    opt_level: crate::opt::OptLevel,
    overflow: crate::db::OverflowSpec,
) -> CompileOutput {
    // Establish the compile-stack precondition at the SHARED SINK — every entry point (the bin, the tests
    // calling `compile`/`compile_with_opt` directly, `compile_component`) reaches compilation through here,
    // so wrapping here is the one place that guarantees the guard-sized worker stack for ALL of them. (The
    // old wrap lived only in `compile_component`, so a caller that used `compile` directly — e.g. the CSE
    // perf tests building a 400-deep arith chain — recursed on the ≈2 MB `cargo test` worker thread and
    // SIGABRT'd before the semantic depth guard could fire, regardless of the per-level budget.)
    // `run_with_compiler_stack` is idempotent (runs inline if already on the worker), so the bin's/embedders'
    // (`cli.rs`, `cdz-kernel`, `cdz-smith`) existing outer wrap does NOT double-spawn. (`compile_component`
    // no longer wraps — it reaches its guard-sized stack through this sink like every other caller.) The
    // borrowed inputs/targets and the `CompileOutput` result are all `Send`, so the scoped worker is sound.
    crate::host::run_with_compiler_stack(|| {
        compile_with_opt_inner(inputs, targets, opt_level, overflow)
    })
}

fn compile_with_opt_inner(
    inputs: &[Artifact],
    targets: &[Target],
    opt_level: crate::opt::OptLevel,
    overflow: crate::db::OverflowSpec,
) -> CompileOutput {
    trace!(target: "rcdzc::compile", inputs = inputs.len(), targets = targets.len(), level = %opt_level, "compile requested");
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
        .and_then(|a| cadenza_compile_abi::decode_name(&a.bytes));
    let (mut arenas, linkage, source_snapshots) =
        match link_inputs(&ast_arts, entry_name.as_deref()) {
            Ok(a) => a,
            Err(r) => return fail(vec![r]),
        };
    // NO-REDECLARE for an EXTERNAL artifact world: a reducer delivered with a `KIND_WIT_WORLD` artifact (vs
    // an in-source `(world …)`) gets its world-import effects synthesized too. Inject them into the arena
    // BEFORE `Db::load` so the synthesized `(effect …)` decls are scanned + resolved like hand-written ones
    // — the same generate-before-resolve ordering `wit_world::inject_world_import_effects` uses for the
    // in-source case (which runs INSIDE `Db::load_linked`). The skip-already-declared guard makes the two
    // paths composable: an interface a guest hand-declares (or one the in-source pre-pass would also inject)
    // is never synthesized twice.
    if let Some(world_art) = inputs.iter().find(|a| a.kind == link::KIND_WIT_WORLD) {
        crate::wit_world::inject_world_import_effects_from_bytes(&mut arenas, &world_art.bytes);
        // NO-ANNOTATION EXPORT BOUNDARY for an EXTERNAL artifact world: derive each guest-export def's
        // boundary param types from the artifact's guest-export members (the artifact analogue of the
        // in-source `derive_world_export_param_annotations`, which runs in `Db::load_linked`). The flagship +
        // identity + provenance reducers target an artifact world, so this is the real reducer path — without
        // it those reducers still need the entry-point param annotation the operator flagged. Runs here,
        // BEFORE `Db::load`, so the injected `(: <param> <type>)` is scanned + resolved like a hand-written
        // one; the `(: …)`-skip keeps it composable with the in-source pass (an author annotation wins).
        crate::wit_world::derive_world_export_param_annotations_from_bytes(
            &mut arenas,
            &world_art.bytes,
        );
    }
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
    // Seed the GLOBAL overflow policy (from a `Project.cdz` overflow manifest global, `OverflowSpec`
    // default = None/None otherwise) BEFORE any infer/lower pass runs: `infer::overflow_mode_of` reads
    // `db.global_overflow` lazily (during `type_of` on an arith node with no governing module pragma), so
    // setting it right after `Db::load_linked` — before the first compile pass touches an arith node —
    // makes the manifest global take effect at the correct precedence (module pragma > this > `Trap`).
    db.global_overflow = overflow;
    // Hand the compiler the per-file pre-resolve SOURCE snapshots captured by `link_inputs` (before
    // `Db::load` mutated the arena). The self-reflection fill (`Ast.module` → `Prim::ReflectModule`) reflects
    // the enclosing module from these at lowering, keyed by `file_of`. Set here (post-load), like
    // `component_name`, so `Db::load`'s many callers need no new argument.
    db.source_snapshots = source_snapshots;
    // MACRO EXPANSION (DESIGN-macro-system.md §4) — run POST-RESOLVE, PRE-INFER: rewrite every quote-param
    // macro call to its expansion so infer/lower type the EXPANSION, not the macro's declared `Ast` return
    // (spec: macro expansion precedes type checking). A no-op for a program with no `quote` parameter.
    crate::lower::expand_macros(&mut db);
    // A PROVIDER compile names the interface it publishes its exports under (X4b) — the
    // `component-name` request artifact. A peer consumer binds to this name with an `(effect …)`
    // `(bind "cadenza:pkg/iface")` (the effects-unified surface, U2). Absent (the common case) → exports
    // cross as top-level funcs (byte-identical).
    db.component_name = inputs
        .iter()
        .find(|a| a.kind == link::KIND_COMPONENT_NAME)
        .and_then(|a| cadenza_compile_abi::decode_name(&a.bytes));
    // The PREPARSED TARGET WIT WORLD (binary-AST), if the program targets one — the §3b full-A ingest and
    // the SOLE bytes-boundary signal. Raw `cadenza-ast` bytes; the world-structure reader decodes + walks it
    // for emit-to-match (a member the world declares `list<u8>`-in/out over a value-encodable guest compound
    // crosses as canonical bytes, not a runtime handle). rcdzc never parses WIT text (a producer / v-syntax
    // inline-decl lowering emits this artifact). Absent → no world-targeted emit (handles, byte-identical).
    db.wit_world = inputs
        .iter()
        .find(|a| a.kind == link::KIND_WIT_WORLD)
        .map(|a| a.bytes.clone());
    // An in-source top-level `(world …)` DECLARATION targets the world when no external artifact was
    // supplied. The external KIND_WIT_WORLD artifact (a deliberate build input retargeting the program's
    // world without editing source) OVERRIDES the in-source decl — mirroring the effect-bind
    // request-overrides-source precedent — so this only fires when the artifact left `wit_world` unset. The
    // parsed `world` subtree is byte-identical to the artifact form (v-syntax's inline lowering routes
    // through the SAME shared `world_schema_tree`/`wit_type_*` builders the artifact and rcdzc's emit
    // target, per the landed cross-source identity gate), so we codec-encode the subtree VERBATIM — never
    // re-derive the world node a second way, which is where cross-source drift would re-enter.
    if db.wit_world.is_none() {
        let world_forms = db.top_world_forms();
        // A module targets AT MOST ONE world. Two top-level `(world …)` decls is a structural error —
        // decline (naming the count) rather than silently encoding the first and dropping the rest
        // (decline-don't-miscompile). No single AST node anchors "the module has two worlds", so a coded
        // decline, mirroring the `--component-name` validation just below.
        if world_forms.len() > 1 {
            return fail(vec![Reject::coded(
                crate::diag::Code::Malformed,
                format!(
                    "a module declares {} top-level `(world ...)` targets; a reducer targets at most one world — remove the extra declaration(s)",
                    world_forms.len()
                ),
            )]);
        }
        if let Some(&world_item) = world_forms.first() {
            db.wit_world = Some(crate::codec::encode(&crate::sidecar::extract_subtree(
                &db.ast, world_item,
            )));
        }
    }
    // COMPONENT NAME FROM THE IN-SOURCE WORLD (reducer-guest DX): the typed interface-instance emit
    // (`record_interface_export`) needs `db.component_name` — the FQ name it exports the `guest` instance
    // under. When no `--component-name` was given (the `cdz compile <reducer>.sexp --target wasm` path) but
    // the in-source `(world …)` declares an EXPORT interface whose name is already a FULLY-QUALIFIED WIT
    // interface name (`ns:pkg/iface`), derive the component name from it — so a self-describing reducer whose
    // world says `(export cadenza:platform/guest …)` compiles WITHOUT a redundant `--component-name` naming
    // the same interface. A bare (non-FQ) export name (e.g. a hand-written `guest`) does NOT qualify — the FQ
    // name is genuine extra information the CLI still needs via `--component-name`, so this stays a no-op
    // there (declines later at the interface-name guard rather than misusing a bare name as an extern name).
    if db.component_name.is_none()
        && let Some(world_bytes) = &db.wit_world
        && let Some(arenas) = crate::codec::decode(world_bytes)
        && let Some(world) = crate::wit_world::parse_target_world(&arenas, arenas.root)
        && let Some(export_iface) = world.exports.first()
        && crate::backend::common::export_name::is_valid_interface_name(&export_iface.name)
    {
        db.component_name = Some(export_iface.name.clone());
    }
    // The `--component-name` a PROVIDER publishes its interface under is a component-boundary name,
    // emitted verbatim as the exported interface-instance's extern name. A non-conforming value would
    // produce a component `wasmtime` rejects at LOAD with no diagnostic (the provider twin of the
    // `(bind …)` interface-name miscompile). Validate the compile-request value here — no AST node to
    // anchor, so a coded decline naming the offending string.
    if let Some(name) = &db.component_name
        && !crate::backend::common::export_name::is_valid_interface_name(name)
    {
        return fail(vec![Reject::coded(
            crate::diag::Code::Malformed,
            format!(
                "the `--component-name` `{name}` is not a valid component interface name — {}",
                crate::diag::MALFORMED_INTERFACE_NAME_MESSAGE
            ),
        )]);
    }
    // A compile-request `effect-bind` artifact OVERRIDES the program's in-source `(bind …)` defaults (U3):
    // it carries an effect→interface map (canonical binary AST, operator P0 seq-284) — a non-empty interface
    // REBINDS an effect to a peer; an EMPTY interface UNBINDS it (so it escapes to the host, or a test
    // handles it in-program). Request wins over the source default; an in-program `(handle …)` still wins
    // over both (it discharges the effect before it escapes). A malformed artifact decodes to `None` and is
    // ignored (an empty map — the input degrades to "no override", never a crash).
    if let Some(a) = inputs.iter().find(|a| a.kind == link::KIND_EFFECT_BIND) {
        for (effect, iface) in
            cadenza_compile_abi::effect_bind_wire::decode(&a.bytes).unwrap_or_default()
        {
            let effect = effect.trim();
            let iface = iface.trim();
            if iface.is_empty() {
                db.effect_bindings.remove(effect); // UNBIND (empty interface)
                continue;
            }
            // A compile-request REBIND target is the same component-boundary interface name as a source
            // `(bind …)`; validate it so a bad `--bind` value is a clear reject, not a silent
            // invalid-component miscompile.
            if !crate::backend::common::export_name::is_valid_interface_name(iface) {
                return fail(vec![Reject::coded(
                    crate::diag::Code::Malformed,
                    format!(
                        "the compile-request rebind of `{effect}` to `{iface}` is not a valid \
                         component interface name — {}",
                        crate::diag::MALFORMED_INTERFACE_NAME_MESSAGE
                    ),
                )]);
            }
            db.effect_bindings
                .insert(effect.to_string(), iface.to_string()); // REBIND
        }
    }
    trace!(target: "rcdzc::compile", defs = db.defs.len(), exports = db.exports.len(), "loaded program");

    // Run the BACKEND-INDEPENDENT optimization passes the requested level enables, over the shared Core
    // column — ABOVE the layout/backend split, so every backend inherits them (`DESIGN-tiered-
    // optimization-levels-rcdzc.md`). This slice registers NO passes yet (the pipeline is empty, so this
    // is a verified no-op and every level emits a byte-identical artifact); it establishes the seam the
    // migration fills — each pass added here declares its `min_level`, and the `PassManager` runs only
    // those the requested level reaches. The correctness bar is that every level is observably identical.
    // The BACKEND-INDEPENDENT Core-opt PassManager runs POST-LAYOUT — see the call after
    // `layout::compute` below. It MUST run after layout because layout is what establishes each
    // node's EMIT-TIME lowering context (lambda-lift -> db.captured_ref, db.lifted, handler-lift
    // scoping, layout.order): a pass-time core_of on a context-dependent node (an Apply/call, a
    // closure, a lifted-param ref, a match-binder) run PRE-layout memoizes the wrong context-free
    // form into db.core and poisons emit. Running the passes here (pre-layout) forced
    // GlobalCsePass's scalar-only/reject-Apply guard; the post-layout seam lets a pass return the
    // emit-identical node, the timing foundation for Core-DCE / CSE-on-calls over calls/closures.

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
    // `EmitTests` is a THIRD kind: it emits a wasm component like `Emit(Wasm)` but from the `@test` defs
    // (`layout::compute_tests`) — it joins `emit_targets` as `Wasm` (so the fault-gate + backend run the
    // same way) with a flag that swaps the LAYOUT source.
    let mut queries = Vec::new();
    let mut emit_targets: Vec<Target> = targets.to_vec();
    let mut emit_tests = false;
    let mut emit_tests_per_file = false;
    let mut emit_tests_composed = false;
    let mut emit_tests_consumer_only = false;
    let mut emit_tests_shred = false;
    let mut emit_tests_shred_standalone = false;
    let mut emit_tests_shred_two_stage = false;
    for req in &requests {
        match req {
            sidecar::Request::Query(q) => queries.push(q.clone()),
            sidecar::Request::Emit(t) => emit_targets.push(*t),
            sidecar::Request::EmitTests => {
                emit_tests = true;
                emit_targets.push(Target::Wasm);
            }
            // `EmitTestsPerFile` lowers the linked closure ONCE and emits one `@test` component PER FILE
            // (the shared-arena lower-once path). It is handled on its OWN branch below (it produces N
            // artifacts from N layout-views, not the single-layout emit loop), so it does NOT push a
            // `Target` here — it only sets the flag. Like `EmitTests` it is a Wasm test build.
            sidecar::Request::EmitTestsPerFile => {
                emit_tests_per_file = true;
            }
            // `EmitTestsComposed` (Option C) hoists the shared closure into its OWN provider component +
            // emits N consumer components importing it — the emit-reuse path. Like `EmitTestsPerFile` it
            // produces multiple artifacts from a shared lowering on its OWN branch below (a provider + N
            // consumers + a `component-name` sidecar), so it only sets the flag, pushing no `Target`.
            sidecar::Request::EmitTestsComposed => {
                emit_tests_composed = true;
            }
            // `EmitTestsConsumerOnly` (the provider-cache path) reuses the composed driver's bucket + guard +
            // union-edge computation but SKIPS the provider emit — the caller supplies the cached provider at
            // run time. Shares the `emit_tests_composed` branch below (both need the `@test` layout); a flag
            // distinguishes whether to emit the provider.
            sidecar::Request::EmitTestsConsumerOnly => {
                emit_tests_consumer_only = true;
            }
            // `EmitTestsShred` (compiler-driven shred, §S6b): emit ONE whole-library MAIN provider + one thin
            // CONSUMER per `@test`. Its OWN branch below (multiple artifacts from one shared lowering), like the
            // composed request — sets the flag, pushes no `Target`.
            sidecar::Request::EmitTestsShred => {
                emit_tests_shred = true;
            }
            // `EmitTestsShredStandalone`: the shred branch with NO main — each `@test` self-contained.
            sidecar::Request::EmitTestsShredStandalone => {
                emit_tests_shred = true;
                emit_tests_shred_standalone = true;
            }
            // `EmitTestsShredTwoStage`: emit cadenza-ast FRAGMENTS (not wasm) — one shared-closure
            // `(do (def..)..)` no-export fragment + one per-`@test` fragment, spliced+compiled LATER by the
            // fan-out (`rcdzc closure.cdzb test.cdzb --export <name>`). Its OWN top-level branch below (does
            // NOT set `emit_tests_shred`, so the wasm shred block is skipped).
            sidecar::Request::EmitTestsShredTwoStage => {
                emit_tests_shred_two_stage = true;
            }
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
    // Each query result's artifact NAME is its REQUEST INDEX (`0`, `1`, …), not the query's semantic name
    // (a node-id / symbol). Results ride back in request order, so a positional (index) name lets a batch
    // caller that sent N same-kind queries (`--where`'s N `TypeAt`, an LSP multi-query sidecar) locate the
    // i-th result WITHOUT replicating the per-query naming — the delegated-compile reader reads `<i>` by
    // position. rcdzc's own tests read query results BY KIND, and a single-query caller captures the lone
    // result off stdout, so neither depends on the name. (The semantic name was only ever a hidden contract
    // for the file reader; the index makes the contract positional and reader-agnostic.)
    query_artifacts.extend(queries.iter().enumerate().map(|(i, q)| {
        let r = sidecar::run_query(&mut db, q);
        Artifact::new(r.kind, i.to_string(), r.bytes)
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
            // No emit ran on this query-only path, so no CSE partition compares happened.
            cse_partition_core_eq_calls: 0,
            value_range_uncached_calls: 0,
            param_apply_extra_handled_calls: 0,
            is_cse_shareable_uncached_calls: 0,
        };
    }

    // Compute the boundary layout once (target-neutral). A program with no export declines. Layout also
    // gates emission on properties `collect_faults` does not model (e.g. a boundary shape it cannot lay
    // out), so it must run — a well-formed program can still fail to lay out. (Its coarser declines that
    // DUPLICATE a `collect_faults` coded fault — the ambiguous-param case — are handled by reporting the
    // coded fault too, below, so the sidecar `check` surfaces it; the emit path keeps layout's decline.)
    // A test build lays out the boundary from the `@test` NULLARY defs (`compute_tests`) IN PLACE OF the
    // program's `(export …)` clauses; an ordinary build uses `compute`. Everything downstream (faults,
    // reachability, emit) is identical — only the export SOURCE differs.
    // `emit_tests_per_file` gates + faults over the WHOLE `@test` set (like `emit_tests`) — this `layout`
    // validates the closure lays out and drives fault-gating; the per-file branch below re-lays each file's
    // bucket as a cheap layout-view over the SAME lowered Core (no re-lower).
    let layout = match if emit_tests
        || emit_tests_per_file
        || emit_tests_composed
        || emit_tests_consumer_only
        || emit_tests_shred
        || emit_tests_shred_two_stage
    {
        layout::compute_tests(&mut db)
    } else {
        layout::compute(&mut db)
    } {
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

    // B2 SHARING-AWARE-EMIT (post-layout Core-IR seam) — runs AFTER layout (so the Core column is lowered
    // WITH its lift/handler context) and BEFORE emit, binding a shared heap-handle node once into a
    // `Core::Let` slot so the emit-analysis walks stop re-descending it (the durable cmb1/pom5 fix).
    // Layout is provably stable across this intra-body rewrite (v-rb, layout owner: zero per-node state).
    // STEP 1: detection-only, a verified byte-neutral no-op (opt-sweep 0-divergence).
    // POST-LAYOUT backend-independent Core-opt passes (moved from pre-layout). Layout has lowered
    // every reachable body top-down (finish_layout reachability worklist -> core_of), so
    // db.captured_ref/db.lifted/handler-lift/layout.order are established and a pass-time core_of
    // returns the emit-identical node (no context-free poison). Runs at the same post-layout point as
    // run_sharing_aware_emit below and BEFORE it (general Core opt precedes the wasm-emit-prep sharing
    // rewrite; the two no-op on each other's residue). A post-layout pass MUST be LAYOUT-PRESERVING
    // (intra-body; does not change the reachable-def/lifted set) — the same invariant
    // run_sharing_aware_emit holds; a future reachability-changing pass needs a layout recompute here.
    crate::opt::PassManager::for_level(opt_level).run(&mut db);

    // `scrutinee_shares_only = false`: the default (wasm) path installs the FULL B2 plan (the O2 body's pass
    // pipeline makes the general shared-heap bindings reclaim-safe). v-cadenza-backend flips this to a
    // cadenza-at-O1 gate (`emit_targets` includes Cadenza && level < O2) to install only the match-scrutinee
    // subset — the co-designed mechanism lives in `run_sharing_aware_emit`/`b2_bind_plan_scrutinee_only`.
    crate::opt::run_sharing_aware_emit(&mut db, &layout, opt_level, false);

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
    // Potentially-reachable-trap warnings (a const-folded trap DEMOTED to a runtime trap in a runtime `if`
    // branch / `match` arm — CDZ0309): the program builds + runs (RULING A), but the fold-synthesized trap
    // could fire along a reachable path, so flag it (never for an explicit user `trap`).
    diagnostics.extend(collect_reachable_const_trap_warnings(&mut db));

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

    let mut artifacts = query_artifacts;

    // `EmitTestsPerFile`: the shared-arena lower-once `cdz test <dir>` path. The closure was linked +
    // lowered ONCE (this `db`); now emit one wasm test component PER FILE — bucket `db.test_defs()` by
    // `file_of(sig_occ)`, and for each non-empty bucket lay out a `compute_tests_for` VIEW rooted at that
    // file's tests (a cheap re-layout over the SAME Core — no re-lower, no relocation) and emit it as a
    // `component` artifact NAMED by the file's `link` path. `cdz test` calls this ONCE and demuxes the N
    // components by name — replacing N per-file compiles that each re-lowered the whole closure.
    // Behavior-identical: each file's view is the same layout its own `EmitTests` compile produced, and a
    // per-file emit decline is reported node-anchored (the caller demuxes to `file:line:col`). Runs on its
    // OWN branch (it produces N artifacts from N views) — the flag pushed NO `Target`, so the normal emit
    // loop below stays empty for a pure `EmitTestsPerFile` request.
    if emit_tests_per_file {
        use std::collections::BTreeMap;
        // Bucket test defs by file index (BTreeMap → deterministic ascending file order). A test whose
        // `sig_occ` maps to no file (single-file compile, or a synthesized test) buckets under `None`.
        let mut by_file: BTreeMap<Option<usize>, Vec<usize>> = BTreeMap::new();
        for def in db.test_defs() {
            let sig = db.defs[def].sig_occ;
            by_file.entry(db.file_of(sig)).or_default().push(def);
        }
        for (fi, defs) in &by_file {
            // The artifact name = the file's `link` path (so `cdz test` demuxes by name); a `None` bucket
            // (single-file / unfiled) falls back to the program name, matching a plain `EmitTests`.
            let name = fi
                .and_then(|i| db.file_path(i))
                .map(str::to_string)
                .unwrap_or_else(|| program_name(&db));
            match layout::compute_tests_for(&mut db, defs) {
                Ok(view) => match backend::emit(
                    Target::Wasm,
                    &mut db,
                    &view,
                    span_data.as_ref(),
                    external_debug_info.as_deref(),
                ) {
                    Ok(bytes) => {
                        artifacts.push(Artifact::new(Target::Wasm.artifact_kind(), &name, bytes))
                    }
                    Err(mut r) => {
                        trace!(target: "rcdzc::compile", file = %name, reason = %r.message, "per-file test emit declined");
                        sanitize_origin(&db, &mut r);
                        diagnostics.push(crate::abi_bridge::diagnostic_from_reject(&r));
                    }
                },
                Err(mut r) => {
                    trace!(target: "rcdzc::compile", file = %name, reason = %r.message, "per-file test layout declined");
                    sanitize_origin(&db, &mut r);
                    diagnostics.push(crate::abi_bridge::diagnostic_from_reject(&r));
                }
            }
        }
    }

    // `EmitTestsComposed` (Option C): hoist the shared import-closure into ONE provider component + emit N
    // per-file CONSUMER components that import it — the emit-reuse path that collapses the O(tests ×
    // closure-size) embed cost `EmitTestsPerFile` still pays. Produces: one `component-provider` artifact (the
    // shared closure, `db.component_name` set to the closure interface), one `component-name` sidecar (that
    // interface string, so a downstream runner builds `Peer{interface}` without introspecting the component),
    // and N `component` artifacts (the consumers, NAMED by `db.file_path` — the SAME name-demux as
    // `EmitTestsPerFile`). Runs on its OWN branch (multiple artifacts from the shared lowering), so the normal
    // emit loop below stays empty for a pure `EmitTestsComposed` request.
    // `EmitTestsConsumerOnly` (the provider-cache follow-on) shares this driver: same bucket + stem guard +
    // union-edge set, but emits ONLY the consumers (skips the expensive provider emit — the caller supplies a
    // CACHED provider at run time). `emit_provider` = false for that request, true for `EmitTestsComposed`.
    if emit_tests_composed || emit_tests_consumer_only {
        use std::collections::BTreeMap;
        let emit_provider = emit_tests_composed;
        // The closure's published interface — the fixed provider↔consumer contract name (both the provider's
        // `component_name` and every consumer's import interface; the index-agreement witness validates the
        // export/import order under it). A runner reads the `component-name` sidecar to bind `Peer{interface}`.
        const CLOSURE_IFACE: &str = "cadenza:closure/api";
        // Bucket the `@test` defs by file (BTreeMap → deterministic ascending file order), exactly as
        // `EmitTestsPerFile`. A test with no file (`None`) — a single-file / synthesized test — has no shared
        // closure to import, so a `None`-only build has no cross-edge and DECLINES below (falls back).
        let mut by_file: BTreeMap<Option<usize>, Vec<usize>> = BTreeMap::new();
        for def in db.test_defs() {
            let sig = db.defs[def].sig_occ;
            by_file.entry(db.file_of(sig)).or_default().push(def);
        }
        // Resolve each file bucket to its artifact NAME (`db.file_path` = the import-stem) up front. SINGLE-DIR
        // / no-stem-collision GUARD: `db.file_path` is the dir-blind import stem (load-bearing for link
        // resolution — it CANNOT be dir-qualified), so a multi-dir tree with two same-stem files would collide
        // the consumer name-demux (pr881). If any two buckets share a name — or a bucket has no file path — the
        // composed emit is unsound; DECLINE so the caller falls back to the per-file `EmitTests` path
        // (behavior-identical, just without the closure-sharing win). The named files, in bucket order:
        let named_files: Vec<(usize, String, &Vec<usize>)> = by_file
            .iter()
            .filter_map(|(fi, defs)| {
                fi.and_then(|i| db.file_path(i).map(|p| (i, p.to_string(), defs)))
            })
            .collect();
        // SINGLE-DIR / STEM-COLLISION GUARD (pr881, pr888). `db.file_path` is the file's LINK path — a bare
        // import STEM in the flat case (`sread-eval`), or a directory-qualified path in a tree. A downstream
        // runner demuxes the N consumer components by the file's STEM (its basename — the import name a `cdz
        // test` run keys on), which is DIR-BLIND and load-bearing for import resolution (it cannot be
        // dir-qualified). So the real collision is two files whose STEMS match (`a/t.cdz` + `b/t.cdz` → both
        // `t`), NOT two equal full paths (always unique per file — the prior full-path dedup ~never fired, the
        // dead-guard pr888 flagged). Key the collision on the STEM (the final path component): decline if any
        // two files share a stem, so the composed emit never mis-demuxes; the caller falls back to the per-file
        // build. A `None`-file bucket (a synthesized/unfiled test) has no path either → also declines.
        let stem_of = |p: &str| p.rsplit(['/', '\\']).next().unwrap_or(p).to_string();
        let mut stems_seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let stem_collision = named_files
            .iter()
            .any(|(_, n, _)| !stems_seen.insert(stem_of(n)));
        let all_filed = named_files.len() == by_file.len();
        if !all_filed || stem_collision {
            diagnostics.push(crate::abi_bridge::diagnostic_from_reject(&Reject::decline(
                "composed test emit needs every `@test` in a file with a DISTINCT import stem: a file with \
                 no link path, or two files sharing a stem (e.g. `a/t.cdz` + `b/t.cdz` across directories), \
                 would collide the per-file component demux — falling back to the per-file test build",
            )));
        } else {
            // The UNION cross-edge set across ALL files = the shared closure the ONE provider exports. Computed
            // off the whole-`@test` `layout` (validated above), in canonical `layout.order` order so the
            // provider's export order matches every consumer's import order (the index-agreement invariant).
            let file_indices: Vec<usize> = named_files.iter().map(|(i, _, _)| *i).collect();
            let union_edges = layout::cross_component_edges_union(&mut db, &layout, &file_indices);
            // Build the PROVIDER (interface = `CLOSURE_IFACE`, exports the union edges) UNLESS this is a
            // consumer-only (cache-hit) request. A build with no cross-edge declines here → fall back. For the
            // consumer-only path we still VALIDATE the union edge set exists (an empty union → nothing to
            // import → decline to per-file), but skip the expensive `compute_provider_for_edges` + emit (the
            // caller supplies the cached provider at run time). `component_name` is set only for the provider
            // emit and restored after, so the consumer emits stay non-provider.
            let provider_result: Result<Option<Vec<u8>>, Reject> = if emit_provider {
                let saved_component_name = db.component_name.take();
                db.component_name = Some(CLOSURE_IFACE.to_string());
                let r = layout::compute_provider_for_edges(&mut db, &union_edges)
                    .and_then(|pl| {
                        backend::emit(Target::Wasm, &mut db, &pl, span_data.as_ref(), None)
                    })
                    .map(Some);
                db.component_name = saved_component_name;
                r
            } else if union_edges.is_empty() {
                // Consumer-only with no shared closure to import — nothing to hoist/cache → fall back.
                Err(Reject::decline(
                    "no shared closure: the @tests call no imported (cross-file) definition",
                ))
            } else {
                Ok(None) // consumer-only: no provider bytes (the caller supplies the cached provider)
            };
            match provider_result {
                Ok(provider_bytes) => {
                    // The provider component — emitted only on the MISS (provider-emitting) path.
                    if let Some(bytes) = provider_bytes {
                        artifacts.push(Artifact::new("component-provider", CLOSURE_IFACE, bytes));
                    }
                    // The closure CONTENT-HASH (a `u64` fold over the union edge set, carried as canonical
                    // binary AST via `encode_closure_hash`) — emitted on BOTH the
                    // provider (MISS) AND the consumer-only path. On a MISS a runner persists the provider
                    // keyed by this exact hash (recompute-free) + validates its own HIT-decision hash against
                    // this canonical one (a fold of the same `def_content_hash` = FuncLayout col-2 — a
                    // drift-guard). On the CONSUMER-ONLY path it lets `precompile_group` do the cache-HIT
                    // decision from ONE `EmitTestsConsumerOnly` drive (read this hash, confirm the HIT) WITHOUT
                    // the expensive provider mono+codegen the composed path pays — the codegen-skip-on-HIT
                    // win: on a HIT the composed provider bytes would only be DISCARDED, and emitting them is
                    // the dominant warm-once cost. Cheap to add: `union_edges` is already computed above and
                    // `closure_content_hash` just folds `def_content_hash` over it. Additive for existing
                    // consumer-only callers (an extra sidecar they ignore); no behavior change on the MISS
                    // path (same hash, same value, emitted as before — just hoisted out of the provider-bytes
                    // guard). See `consumer_only_emits_the_closure_hash_sidecar`.
                    let hash = sidecar::closure_content_hash(&db, &union_edges);
                    artifacts.push(Artifact::new(
                        sidecar::KIND_CLOSURE_HASH,
                        CLOSURE_IFACE,
                        cadenza_compile_abi::encode_closure_hash(hash),
                    ));
                    artifacts.push(Artifact::new(
                        link::KIND_COMPONENT_NAME,
                        CLOSURE_IFACE,
                        cadenza_compile_abi::encode_name(CLOSURE_IFACE),
                    ));
                    // Each file's CONSUMER: excludes the union cross-edges from its emission set + imports them
                    // from the provider (named by `db.file_path`, the same demux `EmitTestsPerFile` uses).
                    for (_, name, defs) in &named_files {
                        match layout::compute_tests_consumer(
                            &mut db,
                            defs,
                            &union_edges,
                            CLOSURE_IFACE,
                        )
                        .and_then(|cl| {
                            backend::emit(Target::Wasm, &mut db, &cl, span_data.as_ref(), None)
                                .map(|b| (cl, b))
                        }) {
                            Ok((_, bytes)) => artifacts.push(Artifact::new(
                                Target::Wasm.artifact_kind(),
                                name,
                                bytes,
                            )),
                            Err(mut r) => {
                                trace!(target: "rcdzc::compile", file = %name, reason = %r.message, "composed consumer emit declined");
                                sanitize_origin(&db, &mut r);
                                diagnostics.push(crate::abi_bridge::diagnostic_from_reject(&r));
                            }
                        }
                    }
                }
                Err(mut r) => {
                    // No shared closure to hoist (no cross-edge), or the provider does not lay out/emit —
                    // decline so the caller falls back to the per-file path.
                    trace!(target: "rcdzc::compile", reason = %r.message, "composed provider emit declined — fall back to per-file");
                    sanitize_origin(&db, &mut r);
                    diagnostics.push(crate::abi_bridge::diagnostic_from_reject(&r));
                }
            }
        }
    }

    // `EmitTestsShred` (compiler-driven shred, `design/DESIGN-cdz-plugin-dispatch.md` §S6b): emit ONE
    // whole-library MAIN provider component (every reachable NON-`@test` def, exported under `CLOSURE_IFACE`)
    // + one thin CONSUMER component PER `@test` (each exports just its own test and imports the whole library
    // from main). A runner links each test target against main (`cdz-run test-<name>.wasm --peer
    // <iface>=main.wasm`) and grades by exit code — a per-TEST ca-derivation matrix. The KEY difference from
    // `EmitTestsComposed`: the provider boundary is the WHOLE LIBRARY (`layout.order` minus the `@test`
    // entries), NOT just the CROSS-FILE closure (`cross_component_edges`) — so a SAME-FILE suite (no cross-file
    // imports, e.g. `iterators`) still gets a NON-EMPTY main + uniform per-test linking (no empty-main special
    // case for the runner). Reuses `compute_provider_for_edges` (main) + `compute_tests_consumer` (each test, a
    // single-def slice over that whole-library boundary — a single-element bucket is the degenerate consumer
    // case; it imports the whole provider interface at the right positions, unused imports harmless). Runs on
    // its OWN branch (multiple artifacts from one shared lowering), so the emit loop below stays empty.
    if emit_tests_shred {
        const CLOSURE_IFACE: &str = "cadenza:closure/api";
        let test_defs = db.test_defs();
        // The whole-library boundary: every reachable def in emission (`layout.order`) order that is NOT a
        // `@test` entry AND has a body (a body-having VALUE def). main EXPORTS these; each consumer EXCLUDES +
        // imports them. The `body.is_some()` filter is REQUIRED (v-inference review): `compute_provider_for_edges`
        // DECLINES on a bodyless edge ("export has no body") — a bodyless entry in `layout.order` (a malformed /
        // decl-only def) must not enter the provider export set. Empty result (a suite whose tests call no
        // body-having user def — only prims/literals) → `compute_provider_for_edges` declines below (no library
        // to hoist); the real gate suites all have library defs.
        //
        // NOTE (deferred #4031, v-inference caveats 2/3): a library fn with a NON-SCALAR/compound param
        // (List/tuple/record/Char/arrow) is not boundary-representable, so `compute_provider_for_edges` →
        // `export_params` DECLINES it as a provider export (and the consumer's cross-boundary call to it hits the
        // same limit). The cross-FILE-edge subset the composed path uses happened to be boundary-exportable; the
        // WHOLE library pulls in more, some non-exportable. So whole-library shred is GREEN for a suite whose
        // reachable library fns are all scalar-boundary, and DECLINES (handled gracefully below) on a
        // compound-param library fn until the deferred #4031 compound-entry-param emit lands (v-rust-backend).
        let test_set: crate::fxhash::FxHashSet<usize> = test_defs.iter().copied().collect();
        let library_edges: Vec<usize> = layout
            .order
            .iter()
            .copied()
            .filter(|d| !test_set.contains(d) && db.defs[*d].body.is_some())
            .collect();
        // TWO shapes, chosen by whether this program HAS an emitted shared library:
        //  • HAS-MAIN (`library_edges` non-empty — a package/group whose @tests call emitted library defs):
        //    emit ONE MAIN provider (the library, under `CLOSURE_IFACE`) + one CONSUMER per @test importing it.
        //    `main-file` = "main.wasm"; the runner `--peer <iface>=main.wasm`.
        //  • STANDALONE (`library_edges` empty — an independent file whose @tests call no emitted user def, all
        //    inlined/prims): emit NO main; each @test is a SELF-CONTAINED component (`compute_tests_for`), run
        //    with NO `--peer`. `main-file` = "" (v-test-shred's exec conditionally adds `--peer` on a non-empty
        //    main-file, keeping enumeration uniform without betting on `--peer`-ing an empty provider).
        // The cdz-side `cdz test --emit-shred` groups a multi-file project by shared closure + drives this per
        // group, renaming each `main` → `main-<group>.wasm` + merging manifests (§S6b / v-test-shred layout).
        // STANDALONE mode forces NO main — every `@test` is self-contained (`compute_tests_for`), even when a
        // shared library exists, so there is no peer boundary (compound-param tests shred cleanly, no #4031).
        let has_main = !emit_tests_shred_standalone && !library_edges.is_empty();
        let mut main_file = String::new();
        let mut main_ok = true;
        if has_main {
            // MAIN artifact NAMED "main" (not the iface) so `cdz-compile -o D` writes `main.wasm` (via the
            // `component-provider`→`wasm` ext). Its INTERFACE identity stays `CLOSURE_IFACE` (set as
            // `db.component_name` for this emit + carried per-entry as `main-iface`); the artifact NAME is the
            // file key. `component_name` restored after so the per-test consumer emits stay non-provider.
            let saved_component_name = db.component_name.take();
            db.component_name = Some(CLOSURE_IFACE.to_string());
            let main_result = layout::compute_provider_for_edges(&mut db, &library_edges)
                .and_then(|pl| backend::emit(Target::Wasm, &mut db, &pl, span_data.as_ref(), None));
            db.component_name = saved_component_name;
            match main_result {
                Ok(main_bytes) => {
                    artifacts.push(Artifact::new("component-provider", "main", main_bytes));
                    main_file = "main.wasm".to_string();
                }
                Err(mut r) => {
                    // A group WITH a shared library whose provider won't lay out/emit (e.g. a compound-param
                    // library fn, deferred #4031) — decline the whole shred for this program (no partial main;
                    // the consumers would hit the same boundary limit). The caller falls back.
                    trace!(target: "rcdzc::compile", reason = %r.message, "shred main (library provider) emit declined");
                    sanitize_origin(&db, &mut r);
                    diagnostics.push(crate::abi_bridge::diagnostic_from_reject(&r));
                    main_ok = false;
                }
            }
        }
        if main_ok {
            // One CONSUMER per `@test` — artifact NAMED `test-<def-name>` so `-o D` writes `test-<name>.wasm`
            // (matching the manifest's `target`). HAS-MAIN → a thin consumer importing the library from main;
            // STANDALONE → a self-contained component. Track SUCCESSFULLY-emitted tests so the manifest lists
            // only runnable targets (a compound-param test that declines pre-#4031 has no target → omitted).
            let mut emitted: Vec<usize> = Vec::new();
            for &def in &test_defs {
                let name = db.defs[def].name.clone();
                let target_name = format!("test-{name}");
                let layout_result = if has_main {
                    layout::compute_tests_consumer(&mut db, &[def], &library_edges, CLOSURE_IFACE)
                } else {
                    layout::compute_tests_for(&mut db, &[def])
                };
                match layout_result.and_then(|cl| {
                    backend::emit(Target::Wasm, &mut db, &cl, span_data.as_ref(), None)
                }) {
                    Ok(bytes) => {
                        artifacts.push(Artifact::new(
                            Target::Wasm.artifact_kind(),
                            &target_name,
                            bytes,
                        ));
                        emitted.push(def);
                    }
                    Err(mut r) => {
                        trace!(target: "rcdzc::compile", test = %name, reason = %r.message, "shred consumer emit declined");
                        sanitize_origin(&db, &mut r);
                        diagnostics.push(crate::abi_bridge::diagnostic_from_reject(&r));
                    }
                }
            }
            // The MANIFEST — a cadenza-ast VALUE (mirroring `Query::TestList`'s shape + the exec fields),
            // `codec::encode`d so `cdz test --emit-shred` forwards it + v-test-shred's `mkTestExec` decodes it
            // with the ONE shared codec. One `(entry <name> <is-property> <file> <export> <target> <main-iface>
            // <main-file>)` per emitted test — POSITIONAL, 7 fields. `main-file` is "" for a STANDALONE program
            // (run with no `--peer`) or "main.wasm" here; the cdz side rewrites it to `main-<group>.wasm` when it
            // merges per-group manifests.
            let mut b = crate::ast::Builder::new();
            let atom_str =
                |b: &mut crate::ast::Builder, s: &str| b.atom_leaf(crate::ast::Leaf::Str(s.into()));
            let mut entries: Vec<crate::ast::StructId> = Vec::with_capacity(emitted.len());
            for &def in &emitted {
                let d = &db.defs[def];
                let is_property = !d.params.is_empty() || d.name.ends_with("-gen");
                let file = db
                    .file_of(d.sig_occ)
                    .and_then(|fi| db.file_path(fi))
                    .unwrap_or("")
                    .to_string();
                // `export` = the wasm symbol the runner `--call`s — the `@test`'s raw def name (a plain nullary
                // def carries no transform suffix, so both `compute_tests_consumer` and `compute_tests_for`
                // export it under that name); explicit so a consumer never re-derives it.
                let export = d.name.clone();
                let target = format!("test-{}.wasm", d.name);
                let head = b.name("entry");
                let name_n = atom_str(&mut b, &d.name);
                let isprop_n = b.atom_leaf(crate::ast::Leaf::Bool(is_property));
                let file_n = atom_str(&mut b, &file);
                let export_n = atom_str(&mut b, &export);
                let target_n = atom_str(&mut b, &target);
                let iface_n = atom_str(&mut b, CLOSURE_IFACE);
                let mainfile_n = atom_str(&mut b, &main_file);
                entries.push(b.list(vec![
                    head, name_n, isprop_n, file_n, export_n, target_n, iface_n, mainfile_n,
                ]));
            }
            let manifest_head = b.name("shred-manifest");
            let mut children = Vec::with_capacity(entries.len() + 1);
            children.push(manifest_head);
            children.extend(entries);
            let root = b.list(children);
            artifacts.push(Artifact::new(
                sidecar::KIND_SHRED_MANIFEST,
                "manifest",
                crate::codec::encode(&b.finish(root)),
            ));
        }
    }

    // TWO-STAGE shred (§S6b, standalone-everywhere heavy suites): emit cadenza-ast FRAGMENTS, not wasm. ONE
    // shared-closure no-export fragment (`closure.cdzb`) + one per-`@test` fragment (`test-<name>.cdzb`), both
    // via `backend::cadenza::emit_fragment` over the SAME full-suite `layout` (its `layout.order` filter is
    // deterministic → byte-stable fragment = v-nix's CA key). The per-test WASM is built LATER by the fan-out:
    // `rcdzc closure.cdzb test-<name>.cdzb --export <name>` (the `--export` splice, #5405) — closure lowered
    // ONCE + CA-cached, each test cheap codegen. Manifest reuses the 7-field shred-manifest shape with
    // `main-file` = "closure.cdzb" (the shared CA fragment) + `target` = "test-<name>.cdzb" (the per-test
    // fragment); the fan-out reads those two + `export` for the splice.
    if emit_tests_shred_two_stage {
        let test_defs = db.test_defs();
        let test_set: crate::fxhash::FxHashSet<usize> = test_defs.iter().copied().collect();
        // The shared closure = every reachable non-`@test` BODY-having def, by source name (the fragment mode
        // filters `layout.order` to this set; `body.is_some()` mirrors the shred main's provider edge filter).
        let closure_names: std::collections::HashSet<String> = layout
            .order
            .iter()
            .copied()
            .filter(|d| !test_set.contains(d) && db.defs[*d].body.is_some())
            .map(|d| db.defs[d].name.clone())
            .collect();
        // The shared-closure fragment — lowered ONCE, `include_type_decls=true` so each `(type …)` appears
        // exactly once in the spliced program (per-test fragments pass false → no duplicate decl on recompile).
        let closure_ok = match backend::cadenza::emit_fragment(
            &mut db,
            &layout,
            &closure_names,
            true,
        ) {
            Ok(bytes) => {
                artifacts.push(Artifact::new(Artifact::KIND_AST, "closure", bytes));
                true
            }
            Err(mut r) => {
                trace!(target: "rcdzc::compile", reason = %r.message, "two-stage closure fragment declined");
                sanitize_origin(&db, &mut r);
                diagnostics.push(crate::abi_bridge::diagnostic_from_reject(&r));
                // No closure ⇒ the per-test fragments (which splice against it) are void — skip them.
                false
            }
        };
        if closure_ok {
            let mut b = crate::ast::Builder::new();
            let atom_str =
                |b: &mut crate::ast::Builder, s: &str| b.atom_leaf(crate::ast::Leaf::Str(s.into()));
            let mut entries: Vec<crate::ast::StructId> = Vec::with_capacity(test_defs.len());
            for &def in &test_defs {
                let name = db.defs[def].name.clone();
                let subset: std::collections::HashSet<String> =
                    std::iter::once(name.clone()).collect();
                let per_test = match backend::cadenza::emit_fragment(
                    &mut db, &layout, &subset, false,
                ) {
                    Ok(bytes) => bytes,
                    Err(mut r) => {
                        trace!(target: "rcdzc::compile", test = %name, reason = %r.message, "two-stage per-test fragment declined");
                        sanitize_origin(&db, &mut r);
                        diagnostics.push(crate::abi_bridge::diagnostic_from_reject(&r));
                        continue;
                    }
                };
                artifacts.push(Artifact::new(
                    Artifact::KIND_AST,
                    format!("test-{name}"),
                    per_test,
                ));
                // Manifest entry — the 7-field shred-manifest shape. `target`/`main-file` are the two
                // fragments the fan-out splices; `export` is the boundary symbol (`--export <export>`).
                let d = &db.defs[def];
                let is_property = !d.params.is_empty() || d.name.ends_with("-gen");
                let file = db
                    .file_of(d.sig_occ)
                    .and_then(|fi| db.file_path(fi))
                    .unwrap_or("")
                    .to_string();
                let head = b.name("entry");
                let name_n = atom_str(&mut b, &name);
                let isprop_n = b.atom_leaf(crate::ast::Leaf::Bool(is_property));
                let file_n = atom_str(&mut b, &file);
                let export_n = atom_str(&mut b, &name);
                let target_n = atom_str(&mut b, &format!("test-{name}.cdzb"));
                let iface_n = atom_str(&mut b, "");
                let mainfile_n = atom_str(&mut b, "closure.cdzb");
                entries.push(b.list(vec![
                    head, name_n, isprop_n, file_n, export_n, target_n, iface_n, mainfile_n,
                ]));
            }
            let manifest_head = b.name("shred-manifest");
            let mut children = Vec::with_capacity(entries.len() + 1);
            children.push(manifest_head);
            children.extend(entries);
            let root = b.list(children);
            artifacts.push(Artifact::new(
                sidecar::KIND_SHRED_MANIFEST,
                "manifest",
                crate::codec::encode(&b.finish(root)),
            ));
        }
    }

    // Clean: ask each requested target's backend to fill its artifact. The query artifacts (facts
    // read above) lead, then each emitted backend artifact — all one kinded-artifact list, selected by
    // kind (`build-tool-interface.md`).
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
                diagnostics.push(crate::abi_bridge::diagnostic_from_reject(&r));
            }
        }
    }

    // GUEST export RESULT-TYPE map (bytes-second run-wiring, `KIND_RESULT_TYPES`): surface each boundary
    // export's COMPILED result type as a `<name>\t<Ty::render_name>` line so `cdz-run` can disambiguate a
    // WIT-erased leaf at render (`render_val_typed` — a `list<u8>` as `Bytes` `b"…"` vs `List UInt8`
    // `#list(…)`, a `string` as a `Symbol` `#"…"`). Computed HERE (db + layout still live; the db is dropped
    // below) — `type_of` needs `&mut db`, so collect the Tys first, then render (`name_ctx` borrows `&db`).
    if !layout.exports.is_empty() {
        let export_tys: Vec<(String, crate::ty::Ty)> = layout
            .exports
            .iter()
            .map(|e| (e.name.clone(), crate::infer::type_of(&mut db, e.body)))
            .collect();
        // seq-284/307 (operator ruling B: "full type AST, no render-name strings across boundaries"): the
        // guest RESULT-TYPE map carries each export's FULL structured `Ty` as a cadenza-ast payload
        // (`encode_ty_payload`), NOT a `<name>\t<render_name>` line — so `cdz-run`/v-rust-backend render
        // from the DECODED `Ty` (`render_val_typed`) rather than parsing a string. Build a standalone
        // `Arenas` per export (extract the `encode_ty_payload` subtree out of `db.ast`, exactly as the
        // `KIND_EXPORT_TYPES` producer does) + frame them via the shared codec `encode_result_types`
        // (mirrors `export_types_wire`; total-decode). One `Ty→AST` encoding, one canonical wire.
        let entries: Vec<(String, crate::ast::Arenas)> = export_tys
            .iter()
            .map(|(name, ty)| {
                let root = crate::eval::encode_ty_payload(&mut db, ty);
                (name.clone(), crate::sidecar::extract_subtree(&db.ast, root))
            })
            .collect();
        let map_bytes = cadenza_compile_abi::encode_result_types(&entries);
        // Surface the map as a standalone artifact (an IN-PROCESS consumer reads it via `out.artifact`).
        artifacts.push(Artifact::new(
            Artifact::KIND_RESULT_TYPES,
            program_name(&db),
            map_bytes.clone(),
        ));
        // ALSO EMBED it as a COMPONENT-TOP-LEVEL custom section `cdz-result-type` (the run-wiring): the corpus
        // gate is a multi-process pipe — it spawns the `cdz-run` BINARY over the component bytes, so no
        // in-process artifact reaches it; the runner byte-scans this section from the piped component to
        // disambiguate the WIT-erased leaves. A COMPONENT-level custom section (NOT inside a nested core
        // module — those have their own id-0 customs a top-level scan must not false-match) that wasmtime
        // ignores, appended to the finished component bytes (valid at the end of the top-level section
        // sequence). See `cdz-run`'s `scan_result_type_section`.
        //
        // ONLY for a PLAIN single-component build (the corpus gate / `cdz run` — every request is a Query or
        // a plain Emit): `layout.exports` describes THIS one component. The `EmitTests*` builds emit
        // per-`@test`/per-file test components from per-file layout views — the whole-layout map would
        // mis-describe them + appending it breaks the per-file byte-identity invariant
        // (`emit_tests_per_file_..._byte_identical`); those run IN-PROCESS via `cdz test` anyway, and the
        // typed-render cases are CORPUS cases (plain single-component builds).
        let plain_build = requests
            .iter()
            .all(|r| matches!(r, sidecar::Request::Query(_) | sidecar::Request::Emit(_)));
        let one_component = artifacts
            .iter()
            .filter(|a| a.kind == Target::Wasm.artifact_kind())
            .count()
            == 1;
        if plain_build
            && one_component
            && let Some(comp) = artifacts
                .iter_mut()
                .find(|a| a.kind == Target::Wasm.artifact_kind())
        {
            let section =
                crate::backend::wasm::dwarf::custom_section("cdz-result-type", &map_bytes);
            comp.bytes.extend_from_slice(&section);
        }
    }

    CompileOutput {
        artifacts,
        diagnostics,
        // Surface the emit path's per-`Db` CSE-partition compare count (the `Db` is dropped here) for the
        // regression-guard test to read a single-compile value — see `Db::cse_partition_core_eq_calls`.
        // The `CompileOutput` field is always-present (it moved to the shared crate, where a
        // `#[cfg(test)]` field can't be set from `rcdzc`'s tests), but the `Db` COUNTER stays
        // `#[cfg(test)]` (it's CSE-lane test instrumentation) — so read it under test, else a harmless 0.
        cse_partition_core_eq_calls: {
            #[cfg(test)]
            {
                db.cse_partition_core_eq_calls
            }
            #[cfg(not(test))]
            {
                0
            }
        },
        value_range_uncached_calls: {
            #[cfg(test)]
            {
                db.value_range_uncached_calls
            }
            #[cfg(not(test))]
            {
                0
            }
        },
        param_apply_extra_handled_calls: {
            #[cfg(test)]
            {
                db.param_apply_extra_handled_calls
            }
            #[cfg(not(test))]
            {
                0
            }
        },
        is_cse_shareable_uncached_calls: {
            #[cfg(test)]
            {
                db.is_cse_shareable_uncached_calls
            }
            #[cfg(not(test))]
            {
                0
            }
        },
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
        // A synthesized (β-copy) anchor has no span. Before dropping it, try to RELOCATE it to the
        // source occurrence the copy was made from (`synth_name_origin`, recorded in `copy_structural`):
        // a name that re-resolved UNBOUND in an inlined body — the whole-program CDZ0101 that the
        // per-file `cdz check` reports at its user occurrence but the reached-poison walk produced on the
        // spliced copy — then anchors at the real source reference instead of being un-anchored (a bare
        // "unbound name `x`" with nothing to point at, the hard-to-debug symptom). If there is no
        // recorded provenance to a user node, fall back to the old behavior: null the unmappable anchor.
        reject.at = db.source_of_synth(id);
    }
}

/// A convenience over [`compile`]: a lone canonical-AST byte string → the WebAssembly component bytes,
/// or the first error diagnostic. What the tests and simple callers use.
///
/// The compile-stack precondition (`crate::host::run_with_compiler_stack`) is now established at the
/// shared sink `compile_with_opt`, so this — like every other `compile` caller (the bin, the tests) —
/// gets the guard-sized worker stack without needing its own wrap. See `crate::host` for why the stack
/// is sized from `DESCENT_DEPTH_LIMIT`.
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
                // A genuine REJECTION (coded, and NOT a decline) is the stronger, more actionable "no" —
                // prefer it over a coded DECLINE (`CDZ0900`, still an `is_decline` construct now that
                // seq-286 gives declines a code). The old `find(code.is_some())` assumed coded ⟹ rejection
                // (true when every decline was codeless); once a decline can carry `CDZ0900`, a coded
                // decline raised EARLIER in the pipeline (e.g. the partial-builtin wrong-arity decline in
                // `infer`) would otherwise pre-empt a coded rejection raised LATER (e.g. the CDZ0201
                // partial-application-escapes-the-boundary reject), inverting the safety ordering
                // (`reference-compiler.md` §Outcomes Are Ordered By Safety). So: rejection first, then any
                // coded diagnostic (the coded decline), then any error at all.
                .find(|d| d.code.is_some() && d.code.as_deref() != Some("CDZ0900"))
                .or_else(|| errors().find(|d| d.code.is_some()))
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
    let mut out: Vec<Diagnostic> = faults
        .iter()
        .map(crate::abi_bridge::diagnostic_from_reject)
        .collect();
    // WARNINGS ride alongside the faults so "diagnostics as you type" (`Query::Diagnostics` / `cdz
    // check`) surfaces them too — an unused binding and a dead-trap are exactly the kind of thing an
    // editor's inline lint should show. They are non-error severity, so they never deny an artifact.
    out.extend(collect_dead_trap_warnings(db));
    out.extend(collect_unused_binding_warnings(db));
    out.extend(collect_redundant_arm_warnings(db));
    out.extend(collect_discarded_value_warnings(db));
    out.extend(collect_reachable_const_trap_warnings(db));
    out
}

/// The FIXED registry of module-directive keys the specification defines (`modules-and-namespaces.md` §A
/// Module Directive Is Drawn From A Fixed Set). The single source of truth for BOTH the `(pragma …)`
/// validation (a key not here is CDZ0601) and the "did you mean?" suggestion an unknown key gets — so the
/// suggestion can never drift from the accepted set. Small and closed today (`default-integer`,
/// `default-fraction`, `default-float`); a new spec directive adds its key here.
const PRAGMA_REGISTRY: &[&str] = &[
    "default-integer",
    "default-fraction",
    "default-float",
    "param",
    "overflow",
];

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
    // No mechanical fix is offered here even though the domain error is clear: which integer type the
    // author meant for the default is a guess (Int64? a narrower width? BigInt?), so suggesting one would
    // be a heuristic edit the author must review, not a mechanical repair. The prose already says "must
    // name an integer type", which is the actionable guidance. (A well-formed `(pragma default-integer
    // <T>)` now COMPILES and applies to bare literals — the effect is modeled; only the NON-integer domain
    // error reaches here.)
    Some(
        Reject::coded(
            Code::NonIntegerDefault,
            format!(
                "`default-integer` must name an integer type, but `{}` is not an integer type \
                 (the default fixes the type otherwise-unconstrained integer literals take)",
                ty.render_name(&db.name_ctx())
            ),
        )
        .at(form),
    )
}

/// The numeric-domain check for a well-formed `(pragma default-fraction <T>)`: `<T>` MUST be an exact
/// rational type (`numeric-model.md` §A Module May Declare Its Default Fraction Literal Type). The
/// fraction analogue of [`non_integer_default_fault`] — same conservatism (an unbound name surfaces its
/// CDZ0101; a type that does not reduce to a concrete `Ty` returns `None`, no false reject), the domain
/// predicate is `Ty::Rational`. A non-rational type-value (`Int64`, `Float64`, a record, …) is CDZ0303 —
/// the machine-readable diagnostic for the unsatisfied "the named type must be an exact rational" constraint.
//= spec/capabilities/numeric-model.md#a-module-may-declare-its-default-fraction-literal-type
//# The type named by a default-fraction-literal directive MUST be an exact rational type the numeric model admits, and a directive naming a type that is not an exact rational type MUST be rejected with the machine-readable diagnostic for that unsatisfied constraint.
fn non_rational_default_fault(db: &mut Db, form: StructId, ty_expr: StructId) -> Option<Reject> {
    // An UNBOUND type name is the same CDZ0101 an annotation gives — surface it (see the integer twin's
    // note for the bound-unmodeled vs unbound distinction this turns on).
    if let crate::resolved::Resolved::Poison(reject) = crate::resolve::resolved_of(db, ty_expr)
        && reject.code == Some(Code::Unbound)
    {
        return Some(reject);
    }
    let ty = crate::eval::typeval_of(db, ty_expr)?;
    // The exact-fraction domain is `Ty::Rational` — the one exact rational type the numeric model admits.
    if matches!(ty, crate::ty::Ty::Rational) {
        return None;
    }
    // Unlike `default-integer` (which declines a fix — many integer types are valid, so which one the
    // author meant is a guess), the exact-fraction domain has EXACTLY ONE admitted type: `Rational`. So
    // the repair is not a guess but the sole valid target — a VERIFIED replace of the named type with
    // `Rational` clears the diagnostic by construction (`diagnostics.md` §A Confirmed Fix Is Marked
    // Verified). Anchored at the type-name node `ty_expr` (what the author wrote), not the whole `form`.
    Some(
        Reject::coded(
            Code::NonIntegerDefault,
            format!(
                "`default-fraction` must name an exact rational type (Rational), but `{}` is not \
                 (the default grounds otherwise-unconstrained numeric literals to an exact fraction)",
                ty.render_name(&db.name_ctx())
            ),
        )
        .at(form)
        .with_fix(crate::diag::Fix::replace_verified(
            ty_expr,
            "Rational",
            "replace with `Rational` (the only exact rational type)",
        )),
    )
}

/// The numeric-domain check for a well-formed `(pragma default-float <T>)`: `<T>` MUST be a floating-point
/// type (`numeric-model.md` §A Module May Declare Its Default Float Literal Type). The floating-point twin
/// of [`non_integer_default_fault`] — same conservatism (an unbound name surfaces its CDZ0101; a type that
/// does not reduce to a concrete `Ty` returns `None`, no false reject), the domain predicate is
/// `Ty::Float` (`Float32`/`Float64` — every admitted IEEE width, the ONE representation every fixed-width
/// and deferred float shares). A non-float type-value (`Int64`, `Rational`, a record, …) is CDZ0303 — the
/// machine-readable diagnostic for the unsatisfied "the named type must be a floating-point type" constraint.
//= spec/capabilities/numeric-model.md#a-module-may-declare-its-default-float-literal-type
//# The type named by a default-float-literal directive MUST be a floating-point type the numeric model admits, and a directive naming a type that is not a floating-point type MUST be rejected with the machine-readable diagnostic for that unsatisfied constraint.
fn non_float_default_fault(db: &mut Db, form: StructId, ty_expr: StructId) -> Option<Reject> {
    // An UNBOUND type name is the same CDZ0101 an annotation gives — surface it (see the integer twin's
    // note for the bound-unmodeled vs unbound distinction this turns on).
    if let crate::resolved::Resolved::Poison(reject) = crate::resolve::resolved_of(db, ty_expr)
        && reject.code == Some(Code::Unbound)
    {
        return Some(reject);
    }
    let ty = crate::eval::typeval_of(db, ty_expr)?;
    // The floating-point domain is `Ty::Float` — every admitted IEEE width (`Float32`/`Float64`), fixed or
    // deferred.
    if matches!(ty, crate::ty::Ty::Float(_)) {
        return None;
    }
    Some(
        Reject::coded(
            Code::NonIntegerDefault,
            format!(
                "`default-float` must name a floating-point type (Float32 or Float64), but `{}` is not \
                 (the default fixes the type otherwise-unconstrained decimal literals take)",
                ty.render_name(&db.name_ctx())
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
pub(crate) fn push_payload_type_positions(
    db: &Db,
    occ: StructId,
    params: &[String],
    out: &mut Vec<(StructId, Vec<String>)>,
) {
    if db.ast.head_name(occ) == Some("Record")
        && let crate::ast::Struct::List(children) = db.ast.get(occ)
    {
        // Descend into each field's TYPE, skipping the name label. A field is a 2-element `(name Type)`
        // pair (s-expr) OR a 3-element `(: name Type)` annotation triple (the ML `{name: Type}` lowering) —
        // handle both so an ML-surfaced record field's type is validated (the `(: name type)` companion of
        // the same field-spelling fix in `db::collect_type_params` + the RecordCtor reducers).
        for &pair in children.iter().skip(1) {
            if let crate::ast::Struct::List(items) = db.ast.get(pair) {
                match items.as_slice() {
                    [_name, ty] => push_payload_type_positions(db, *ty, params, out),
                    [colon, _name, ty] if db.ast.as_name(*colon) == Some(":") => {
                        push_payload_type_positions(db, *ty, params, out);
                    }
                    _ => {}
                }
            }
        }
        return;
    }
    // A `(Qty T u)` quantity type: validate only the inner-type argument (`T`); the second argument is a
    // compile-time UNIT expression whose leaves are unit bases, not types (mirrors `db::collect_type_params`
    // + `resolve::decode_ty`). Validating the whole `(Qty …)` form would push the unit expression as a
    // type position, faulting on its `Unit.base`/`#"meter"` internals — which are not types.
    if db.ast.head_name(occ) == Some("Qty")
        && let crate::ast::Struct::List(children) = db.ast.get(occ)
        && children.len() == 3
    {
        push_payload_type_positions(db, children[1], params, out);
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
pub(crate) fn is_record_bearing(db: &Db, id: StructId) -> bool {
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
pub(crate) fn validate_type_position(
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
    // An ILL-FORMED integer WIDTH in this type position — an over-ceiling `(UInt 65)`, a zero `(UInt 0)`,
    // or a MALFORMED (negative / non-natural) width `(Int -8)`. This must be checked BEFORE the
    // `typeval_of` early-return below: `reduce_ctor` CLAMPS such a width to the sentinel `Int0`, so
    // `typeval_of` succeeds with a valid-looking `Ty` and the position would be waved through as "a real
    // type" — yet the written width is ill-formed. The value-/parameter-annotation checks catch a TOP-LEVEL
    // ill-formed width (`int_width_fault`), but a width NESTED in a compound annotation (`(List (Int -8))`,
    // `(Option (UInt 65))`) or a TYPE-DECLARATION variant payload (`(type T (Mk (Int -8)))`) reaches the
    // front end only through THIS position walk — so without this check it slipped past `cdz check` (silent
    // exit 0) and crossed into a compiled artifact. Well-formedness is TOTAL (numeric-model.md: a bit width
    // outside the admitted range MUST be rejected at compile time), so reject it here, with the SAME coded
    // CDZ0302 + message the annotation sites give, so every position agrees.
    if let Some(fault) = crate::eval::int_width_fault(db, pos) {
        let mut reject = Reject::coded(
            Code::IntOutOfRange,
            crate::infer::ill_formed_int_width_message(&fault),
        )
        .at(pos);
        if let Some(fix) = crate::infer::ill_formed_int_width_fix(&fault, pos) {
            reject = reject.with_fix(fix);
        }
        out.push(reject);
        return;
    }
    // The `(Float W)` companion — an ill-formed float width (outside the admitted IEEE set {32,64}) in a
    // type-declaration payload (`(type T (Mk (Float 8)))`) reaches the front end only through this walk,
    // same as the integer case above. `reduce_ctor` clamps a bad float width to the sentinel `Float0`, so
    // `typeval_of` would wave it through; reject it here with the same coded CDZ0302 the annotation sites
    // give.
    if crate::eval::is_ill_formed_float_width(db, pos) {
        let mut reject =
            Reject::coded(Code::IntOutOfRange, crate::infer::FLOAT_WIDTH_MESSAGE).at(pos);
        if let Some(fix) = crate::infer::ill_formed_float_width_fix(db, pos) {
            reject = reject.with_fix(fix);
        }
        out.push(reject);
        return;
    }
    // A RUNTIME WIDTH `(Int n)`/`(Float n)` (n a parameter/ref) anywhere in this type position — a runtime
    // value in a type-determining position, forbidden (numeric-model.md §A … Type Is Indexed By A
    // Compile-Time Width). `reduce_ctor` clamps it to a sentinel so `typeval_of` would wave it through.
    // DESCEND the type expression (`nested_runtime_width_type`), anchored at the inner `(Int n)`/`(Float
    // n)`, rather than the old top-level-only `is_runtime_width_type(db, pos)` which missed a width NESTED
    // in a compound (`(List (Int n))`) — the caller (`push_payload_type_positions`) only descends
    // record-bearing containers, so a `List`/`Option`/`Tuple` of a width type reaches here WHOLE (PR #439 /
    // Copilot r3592610268). This is a variant-payload / effect-op-type position (the two callers), neither
    // of which has a runtime binding in scope TODAY — so the descent is a soundness-hardening + comment-
    // accuracy fix that closes the code-flagged latent hole and pre-empts any future path (a const-param-
    // threaded width) where a nested type-position width does resolve to runtime data. Same CDZ0302 the
    // annotation sites give.
    if let Some(node) = crate::infer::nested_runtime_width_type(db, pos) {
        let msg = if crate::eval::is_float_ctor_type(db, node) {
            "a floating-point width must be a compile-time admitted width (32 or 64), not runtime data"
        } else {
            "an integer width must be a compile-time natural, not runtime data"
        };
        out.push(Reject::coded(Code::IntOutOfRange, msg).at(node));
        return;
    }
    // A TYPE CONSTRUCTOR applied at the WRONG ARITY in this payload/op-type position — a user generic sum
    // `(Box Int64 Bool)` (Box takes 1) or a prelude `(Option Int64 Bool)`. Like the annotation path
    // (`infer.rs` — `type_ctor_arity_message`), this MUST be checked BEFORE the `typeval_of` early-return
    // below: a user generic REDUCES to a `Ty::Sum` silently ignoring the extra arg (so `typeval_of`
    // succeeds and the position is waved through), yet the extra/missing arg is ill-formed. Without this,
    // a mis-arity payload slipped `cdz check` at the DECLARATION and surfaced only LATER as a confusing
    // CDZ0201 at a construction site ("payload has declared type Box, but a value of type Box was applied"
    // — the two render identically because the extra arg was dropped). Emit the SAME CDZ0203 the annotation
    // path gives, anchored at the offending type expression, so a payload position reads like an annotation.
    if let Some(msg) = crate::infer::type_ctor_arity_message(db, pos) {
        out.push(Reject::coded(Code::TypeMismatch, msg).at(pos));
        return;
    }
    // A BARE type-CONSTRUCTOR name used with NO argument in this position — `(type W (Wrap Box))`, `(op emit
    // (-> Option Int64))`. The APPLIED wrong-arity is caught above, but a bare generic ctor is not an
    // application, so `type_ctor_arity_message` returns `None` AND `typeval_of` SUCCEEDS (a user generic's
    // bare name reduces to a `Ty::Sum` with a fresh var; a prelude ctor fails to reduce but the "not a type"
    // branch below would give a vaguer message) — so it slipped through. The ANNOTATION path rejects this
    // via `bare_type_ctor_needs_argument` (infer.rs); mirror it here so a declaration position agrees:
    // CDZ0203 naming the missing argument + the canonical spelling, checked BEFORE the `typeval_of` return.
    // Returns `None` for a monomorphic/nullary type or a genuine value, so those are unaffected.
    if let Some((ctor, placeholder)) = crate::infer::bare_type_ctor_needs_argument(db, pos) {
        out.push(
            Reject::coded(
                Code::TypeMismatch,
                format!(
                    "`{ctor}` is a type constructor — it needs a type argument here, e.g. `({ctor} {placeholder})`"
                ),
            )
            .at(pos),
        );
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
        // A genuinely-unknown UPPERCASE type name here (a variant payload / nested payload type — `(type
        // Box (Mk Widget))`) gets the BARE "unbound name `Widget`" from `type_errors`, which does not
        // convey that a TYPE is missing. Rewrite it to the same "unknown type `Widget` — declare it with
        // `(type Widget …)`" the annotation sites give (`infer::unknown_type_reject`), so a payload type
        // position reads like an annotation one. Only a BARE unbound name (no ` — did you mean …` suffix)
        // is rewritten: a typo of a real type already carries the more useful did-you-mean, which we keep.
        .map(
            |f| match (f.code, f.at, unbacktick(&f.message).map(str::to_string)) {
                (Some(Code::Unbound), Some(at), Some(name))
                    if name.starts_with(|c: char| c.is_ascii_uppercase())
                        && !f.message.contains("did you mean") =>
                {
                    crate::infer::unknown_type_reject(&name, at, what)
                }
                _ => f,
            },
        )
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

/// The parameter NAMES of definition `def_ix` (a param is a bare name atom or a `(: name T)` binder whose
/// first child is the name), for the b4c predicate name-scope check.
fn def_param_names(db: &Db, def_ix: usize) -> Vec<String> {
    let Some(def) = db.defs.get(def_ix) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for &p in &def.params {
        if let Some(n) = db.ast.as_name(p) {
            names.push(n.to_string());
        } else if let Some(tail) = db.ast.as_form(p, ":")
            && let Some(&first) = tail.first()
            && let Some(n) = db.ast.as_name(first)
        {
            names.push(n.to_string());
        }
    }
    names
}

/// The `@requires`/`@ensures` predicate occurrences of definition `def_ix`, each with whether it is an
/// `@ensures` (so `it` is an additional bound name). Requires first, then ensures.
fn verify_predicates_of(db: &Db, def_ix: usize) -> Vec<(StructId, bool)> {
    let mut v: Vec<(StructId, bool)> = db.requires_of(def_ix).iter().map(|&p| (p, false)).collect();
    v.extend(db.ensures_of(def_ix).iter().map(|&p| (p, true)));
    v
}

/// The first NAME the predicate at `pred` references that is neither a def parameter, nor the SUBJECT binder
/// (`subject_binder` — `ret` for an `@ensures` result, `it` for an `@invariant` value; `None` for
/// `@requires`, which binds no subject), nor a name bound by a `match`-arm pattern / `let` INSIDE the
/// predicate, nor a name that resolves standalone (a prelude/global) — i.e. an UNBOUND name — as a `CDZ0101`
/// `Reject` anchored at that occurrence, or `None` if all names are in scope. A member-access key
/// `(. operand key)` is a label, not a name lookup, so its `key` child is NOT checked (skipped like the
/// resolver does).
///
/// PREDICATE-LOCAL BINDERS: the predicate may DESTRUCTURE via `match` (`(match it ((T.V v) … v …))`) or bind
/// via `let` — the binder names (`v`) are in scope in the arm/body and MUST NOT be flagged unbound. This is
/// ESSENTIAL for `@invariant`, whose canonical form is a destructure (`it` = whole value, predicate reaches
/// the payload via a match arm — nominal-boundary rule). A recursive walk threads a `bound` set that grows
/// with each binder scope (match-arm pattern vars, let bindings), so a name introduced in the predicate is
/// treated as in scope for its subtree.
fn first_unbound_predicate_name(
    db: &mut Db,
    pred: StructId,
    param_names: &[String],
    subject_binder: Option<&str>,
) -> Option<Reject> {
    let mut bound: Vec<String> = param_names.to_vec();
    if let Some(subject) = subject_binder {
        bound.push(subject.to_string());
    }
    unbound_in(db, pred, &mut bound)
}

/// Collect the NAMES a match/let PATTERN binds (a bare `name`, or a constructor pattern's payload names
/// `(T.V a b)` / `(a .. rest)`). Delegates to [`crate::resolve::arm_pattern_binders`] — the well-scoped
/// binder-leaf collector `lower`/`resolve` already use — so the predicate name-scope check binds EXACTLY
/// the names the language binds. That collector skips a compound pattern's HEAD (a ctor / `list`/`tuple`/
/// `map` alias / `guard`), a `.`-member whole-pattern (`C.R`, a nullary-ctor reference — binds nothing),
/// AND the separators `_` (wildcard) / `..` (rest marker). The previous local walk here pushed EVERY bare
/// `as_name` (skipping only a leading `(. …)` head), so it over-collected `_`/`..`/a bare-name pattern
/// head into `bound` — which MASKED a genuine unbound-name error in a predicate body whose reference
/// happened to equal one of those tokens (a false negative slipping the `@requires`/`@ensures`/
/// `@invariant` gate). Reusing the canonical collector closes that gap and keeps this in lockstep with
/// how the arms actually bind. (PR#562 Copilot finding; the collector names the fix.)
fn pattern_binder_names(db: &Db, pat: StructId, out: &mut Vec<String>) {
    out.extend(
        crate::resolve::arm_pattern_binders(db, pat)
            .into_iter()
            .map(|(name, _occ)| name),
    );
}

/// Recursive predicate walk respecting binder scopes. Returns the first unbound-name reject, or `None`.
//
// TERMINATION (no depth guard needed — PR#562 Copilot flagged the recursion as a re-introduced overflow
// risk; dismissed on the same invariant as the PR#556 lower.rs walks): the recursion descends only into
// CHILD occurrences of `cur` (a `.`-member operand, a match scrutinee/arm body, a let binding value/body,
// or a form's children), which are strictly-smaller arena StructIds — the arena is append-only
// (`Arenas::push` assigns `len()` then never reassigns a slot; quote/metaprogramming uses the same
// append-only builder), so a child id is always < its parent and the predicate AST is acyclic by
// construction. A predicate is a finite arena subtree, so the walk bottoms out; a depth cap would be
// cosmetic, not required.
fn unbound_in(db: &mut Db, cur: StructId, bound: &mut Vec<String>) -> Option<Reject> {
    // A `(. operand key)` member access: check the operand, NOT the key (a label).
    if let Some(operand) = db.ast.as_form(cur, ".").and_then(|t| t.first().copied()) {
        return unbound_in(db, operand, bound);
    }
    // `(match SCRUT (PAT ARM) …)` — SCRUT is checked in the outer scope; each ARM is checked with the arm's
    // PATTERN binders added to scope (they are the destructure payload names, in scope only in that arm).
    if let Some(mtail) = db.ast.as_form(cur, "match").map(<[_]>::to_vec)
        && let Some((&scrut, arms)) = mtail.split_first()
    {
        if let Some(r) = unbound_in(db, scrut, bound) {
            return Some(r);
        }
        for &arm in arms {
            // An arm is `(PAT BODY…)` — collect PAT's binders, check BODY under them.
            if let crate::ast::Struct::List(items) = db.ast.get(arm).clone()
                && let Some((&pat, body)) = items.split_first()
            {
                let base = bound.len();
                pattern_binder_names(db, pat, bound);
                for &b in body {
                    if let Some(r) = unbound_in(db, b, bound) {
                        return Some(r);
                    }
                }
                bound.truncate(base); // pop the arm-local binders
            }
        }
        return None;
    }
    // `(let ((n v) …) BODY…)` — each binding value is checked in the outer scope, then BODY under the new
    // names. (A simple non-recursive `let`; the predicate fragment does not use letrec.)
    if let Some(ltail) = db.ast.as_form(cur, "let").map(<[_]>::to_vec)
        && let Some((&binds, body)) = ltail.split_first()
    {
        let base = bound.len();
        if let crate::ast::Struct::List(pairs) = db.ast.get(binds).clone() {
            for &pair in &pairs {
                if let crate::ast::Struct::List(nv) = db.ast.get(pair).clone()
                    && let (Some(&n), Some(&v)) = (nv.first(), nv.get(1))
                {
                    if let Some(r) = unbound_in(db, v, bound) {
                        return Some(r);
                    }
                    if let Some(nm) = db.ast.as_name(n) {
                        bound.push(nm.to_string());
                    }
                }
            }
        }
        for &b in body {
            if let Some(r) = unbound_in(db, b, bound) {
                return Some(r);
            }
        }
        bound.truncate(base);
        return None;
    }
    // A bare NAME: in scope if a param / `it` / a predicate-local binder; else must resolve standalone.
    if let Some(name) = db.ast.as_name(cur).map(str::to_string) {
        if bound.contains(&name) {
            return None;
        }
        if let crate::resolved::Resolved::Poison(reject) = crate::resolve::resolved_of(db, cur)
            && reject.code == Some(Code::Unbound)
        {
            return Some(reject);
        }
        return None;
    }
    // Any other form: recurse into every child.
    if let crate::ast::Struct::List(children) = db.ast.get(cur).clone() {
        for c in children {
            if let Some(r) = unbound_in(db, c, bound) {
                return Some(r);
            }
        }
    }
    None
}

fn collect_faults(db: &mut Db) -> Vec<Reject> {
    let mut faults = Vec::new();
    // A top-level `(world …)` whose descriptor does NOT parse is silently dropped by the world reader
    // (`parse_target_world` → None): the no-redeclare sidecar then synthesizes NO import effects, so every
    // world import cascades to a misleading `unbound name <iface>` CDZ0101 with no hint at the real cause.
    // Surface the ROOT cause — a malformed world descriptor (most often a wrong type-descriptor HEAD: a
    // PRIMITIVE is NAME-head `(bool)`/`(u8)`/…, a COMPOUND is STRING-head `("list" …)`/`("record" …)`).
    // Uses the SAME encode→decode→parse the sidecar/emit path uses, so it fires exactly when a world drops.
    for wf in db.top_world_forms() {
        let bytes = crate::codec::encode(&crate::sidecar::extract_subtree(&db.ast, wf));
        let parses = crate::codec::decode(&bytes)
            .and_then(|a| crate::wit_world::parse_target_world(&a, a.root).map(|_| ()))
            .is_some();
        if !parses {
            faults.push(
                Reject::coded(
                    crate::diag::Code::Malformed,
                    "this `(world …)` declaration did not parse as a valid world descriptor — check each \
                     type descriptor's head: a PRIMITIVE is a NAME-head `(bool)`/`(u8)`/`(string)`/… and a \
                     COMPOUND is a STRING-head `(\"list\" …)`/`(\"record\" …)`/`(\"option\" …)`/`(\"unit\")` \
                     (a malformed descriptor drops the whole world, leaving its imports unbound)",
                )
                .at(wf),
            );
        }
    }
    // A BAKEABLE type-valued export (a nullary export whose type-value reduces to a concrete type) crosses
    // via the constant value-form escape — NOT a fault. But its body's lowering still declines the bare
    // type-value as a runtime value (`TYPE_VALUE_NO_RUNTIME_DECLINE` + friends); this flag lets
    // `dedup_faults` drop that cascade (the escape, not a reject, is the answer). Set in the export loop.
    let mut has_bakeable_type_export = false;
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
    // INLINE-POLICY CONFLICTS (Addendum 4). `inline-always` on a RECURSIVE def is a contradiction — a
    // recursive def CANNOT inline (it would inline without end; it is always emitted once), so the marker
    // is meaningless and almost certainly an author error. Reject it (CDZ0201) rather than silently ignore.
    // (`inline-never` on a recursive def is a harmless no-op — recursion already emits once — so it is NOT
    // a conflict.) The compile-time-DEMANDED `inline-never` conflict is caught at the call site in `lower`
    // (a `const`-arg / type-position demand), not here, since it is a property of the USE, not the decl.
    if !db.inline_always.is_empty() {
        for di in 0..db.defs.len() {
            let Some(body) = db.defs[di].body else {
                continue;
            };
            if db.inline_always.contains(&body) && crate::eval::is_recursive(db, body) {
                let name = db.defs[di].name.clone();
                faults.push(
                    Reject::coded(
                        crate::diag::Code::Malformed,
                        format!(
                            "`{name}` is recursive, so it cannot be `inline-always` — a recursive \
                             definition is always emitted as one function (drop the `inline-always`, or \
                             use `inline-never` which is its default)"
                        ),
                    )
                    .at(db.defs[di].sig_occ),
                );
            }
        }
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
        // `(import …)` top-level form) that the MODULE LINKER resolves, not this single-module compile
        // path — a STRUCTURAL boundary, distinct from a typo (concierge seq-286 ruling: cross-module
        // imports ARE realized, via the linker/module-system path, so this is not a "not yet built" gap).
        // The generic path below would suggest "did you mean `export`?" (import→export is only 2 edits),
        // an actively MISLEADING fix: an author who wrote `import` never meant its opposite. Name the real
        // situation — a module form handled by the linker, not compiled here — with NO swap fix.
        if head == "import" {
            faults.push(
                Reject::decline(
                    "`import` is resolved by the module linker, not by this single-module compile \
                     path — a top-level `import` is not compiled here",
                )
                .at(occ),
            );
            continue;
        }
        // `doc` is a real form, but it documents a definition from INSIDE it — `(def (f …) (doc "…") …)`
        // (the `///` surface renders exactly there) — NOT as a top-level WRAPPER around the def. A user
        // who writes `(doc "…" (def (f …) …))` at the top level (a natural guess, since a `//` COMMENT
        // *does* wrap transparently — `strip_comments` peels it) gets the generic "unbound name `doc`"
        // PLUS a misleading "export names no definition" cascade (the wrapped `def` is hidden from the
        // top-level scan). Name the real placement so the author moves the `(doc …)` inside the def,
        // rather than hunting a `def`/`export` typo the generic keyword-suggestion would propose. A DECLINE
        // (the wrapped form is invisible, so the program cannot compile either way — outcome unchanged).
        if head == "doc" {
            // Coded CDZ0201 (a MALFORMED program — a `(doc …)` in an invalid position), NOT an uncoded
            // decline: the doc form is a KNOWN construct merely MISPLACED (unlike `import`, an unmodeled
            // feature), so this is a well-formedness rejection. Coding it also makes it the PREFERRED
            // primary over the consequent "export names no definition" CDZ0101 (`compile_component` prefers
            // a coded error; both are coded, and this one — anchored earlier, at the wrapper — sorts first
            // and names the ROOT, while the export fault is the downstream symptom of the hidden def).
            faults.push(
                Reject::coded(
                    Code::Malformed,
                    "a `(doc …)` documents a definition from INSIDE it — write `(def (name …) (doc \
                     \"…\") body)`, the shape a `///` doc-comment produces — not as a top-level wrapper \
                     around the definition (a wrapper hides the definition from the module, so nothing \
                     is defined or exported)",
                )
                .at(occ),
            );
            continue;
        }
        // `pragma` is a recognized MODULE DIRECTIVE, but its `default-integer` effect (fixing bare
        // literals' type) is collected only from the members of a NESTED `(module NAME …)` declaration
        // (one written inside a `(do …)`), not from the program's outermost/root module. A `(pragma …)`
        // that reaches `unknown_top_forms` is one at the program's top level — the root module's own
        // member, or a bare do-item — where its effect is NOT applied. The generic path would suggest
        // "did you mean `def`?" (a misleading typo fix); instead name the real situation so the author
        // wraps the module in a `(do …)` (the tested, effective placement) rather than chasing a phantom
        // typo. Coded CDZ0601 (the pragma-directive code): a mis-scoped directive is a directive fault, not
        // an unbound name. (A pragma inside a nested module is collected by the pragma pass and its module's
        // members never reach `unknown_top_forms`, so this fires only for a top-level/root-scope pragma.)
        if head == "pragma" {
            // A well-formed TOP-LEVEL default pragma (`(pragma default-integer|default-fraction|default-
            // float <T>)` at the root / file-top) now TAKES EFFECT — `Db::load` harvests it over the root
            // scope's `(def …)` literals, exactly as a nested `(module …)` pragma is harvested for its
            // members (numeric-model.md §"A Module May Declare Its Default … Literal Type" — no do-nesting
            // requirement; a file IS a module). So it is NO LONGER mis-scoped: emit NO placement fault for
            // it. A MALFORMED pragma (unknown key / wrong arity / domain-bad type) is still rejected by the
            // pragma-REGISTRY pass below (CDZ0601 key / CDZ0602 arity / CDZ0303 domain), which owns those;
            // this branch only ever emitted the placement message, now obsolete for the well-formed default.
            // `continue` (no placement fault) — the registry pass is the single authority for a malformed
            // one, and a well-formed one is now honored.
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
    // A bare NAME atom top-level item resolving to NOTHING — `(module m nonesuch …)` — is the paren-less
    // twin of the `(nonesuch)` APPLICATION rejected in the loop above, and the two must behave identically.
    // `head_name` is `None` for an atom, so `unknown_top_forms` never sees it and the item was SILENTLY
    // ACCEPTED. A bare name naming no binding is broken under ANY reading of the grammar (whether or not a
    // bare EXPRESSION is a legal top-level item — a pending design call routed to the operator — a bare
    // expression referencing a binding that does not exist cannot be intentional). Reject it CDZ0101 with
    // the SAME "unbound name at the top level" message + defined-name did-you-mean the application gets. A
    // LITERAL or a BOUND bare name is left to the bare-expression-legality ruling (not returned here).
    for (name, occ) in db.unbound_bare_name_items() {
        let hint = crate::diag::suggest::did_you_mean(&name, &defined_names, 3);
        faults.push(
            Reject::decline(format!(
                "unbound name `{name}` at the top level{hint} (if `{name}` is meant as a declaration, \
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
            // `default-fraction <T>` — exactly one argument; a well-formed one whose type is not an exact
            // rational type → the numeric-domain CDZ0303 (the fraction twin of `default-integer`).
            Some("default-fraction") => {
                if ptail.len() != 2 {
                    faults.push(
                        Reject::coded(
                            Code::MalformedDirective,
                            "`default-fraction` takes exactly one type argument (e.g. `(pragma default-fraction Rational)`)",
                        )
                        .at(form),
                    );
                } else if let Some(reject) = non_rational_default_fault(db, form, ptail[1]) {
                    faults.push(reject);
                }
            }
            // `default-float <T>` — exactly one argument; a well-formed one whose type is not a
            // floating-point type → the numeric-domain CDZ0303 (the float twin of `default-integer`).
            Some("default-float") => {
                if ptail.len() != 2 {
                    faults.push(
                        Reject::coded(
                            Code::MalformedDirective,
                            "`default-float` takes exactly one type argument (e.g. `(pragma default-float Float32)`)",
                        )
                        .at(form),
                    );
                } else if let Some(reject) = non_float_default_fault(db, form, ptail[1]) {
                    faults.push(reject);
                }
            }
            // `@!param` — the module-level runtime-parameter directive (operator ruling 2026-07-18). Shape:
            // `(pragma param (param <kv>…) (: name Type))` — the config-group `(param …)` app + the typed
            // binder `(: name Type)`. The param SEMANTICS (scan → generate the `Param` effect + the widget
            // manifest) live in `param_sidecar`; this arm is the structural GATE that a well-formed `@!param`
            // passes and a malformed one (missing binder / no config group / untyped) is rejected here rather
            // than reaching the sidecar (the B-invariant: a `@!param` MUST carry an explicit type). A valid
            // one is accepted (Ok — no fault); the sidecar consumes it in `Db::load`.
            Some("param") => {
                // PLACEMENT first: `@!param` is a MODULE directive (operator ruling 2026-07-18 — it
                // parameterizes the whole module, like `@!default-fraction`), so it is well-placed ONLY as a
                // direct top-level member of the program root. A `(pragma param …)` nested in a def body / a
                // `(do …)` value position is misplaced — there it is not a module directive, and the sidecar
                // scans skip it (no accessor / no manifest row). Report the placement as a coded CDZ0602
                // rather than letting its unbound config names (`widget`, `slider`, …) raise a confusing
                // CDZ0101 cascade at value positions. (The default-* pragmas have no analogous guard because
                // a nested one already CDZ0101s as an unbound `(pragma …)` call; `@!param` deserves the
                // actionable placement message since it is the directive a user is most likely to misplace.)
                if !crate::param_sidecar::is_top_level_member(&db.ast, form) {
                    faults.push(
                        Reject::coded(
                            Code::MalformedDirective,
                            "an `@!param` module parameter must be a top-level module directive, not nested \
                             inside a definition or a value position — move it to the module's top level",
                        )
                        .at(form),
                    );
                }
                // tail = [param-key, config-group, binder]; the binder must be a `(: name Type)` colon node
                // and the config-group a `(param …)` app. `param_sidecar::is_param_site` checks exactly this.
                else if !crate::param_sidecar::is_param_site(&db.ast, form) {
                    faults.push(
                        Reject::coded(
                            Code::MalformedDirective,
                            "`@!param` must be `@!param(config…) name : Type` — a module parameter with an \
                             explicit type (e.g. `@!param(widget: slider) width : Int64`)",
                        )
                        .at(form),
                    );
                }
            }
            // `overflow (signed <mode>) (unsigned <mode>)` — the module overflow policy (`numeric-model.md`
            // §A Module May Declare Its Overflow Policy). At least one of the two SIGNEDNESS sub-forms must
            // be present; each present one must be `(signed <mode>)` / `(unsigned <mode>)` naming a `trap` or
            // `wrap` mode. A stray non-`signed`/`unsigned` sub-form, a missing/extra mode argument, or a mode
            // name outside {trap, wrap} → malformed. (The EFFECT — selecting each unqualified `+`/`-`/`*`
            // node's mode — is realized by `db.overflow_specs` + the infer-time signedness selection.)
            Some("overflow") => {
                let subs = &ptail[1..];
                let well_formed_sub = |sub: StructId| -> bool {
                    let Some(t) = db
                        .ast
                        .as_form(sub, "signed")
                        .or_else(|| db.ast.as_form(sub, "unsigned"))
                    else {
                        return false;
                    };
                    // Exactly one argument, naming `trap` or `wrap`.
                    matches!(t, [m] if matches!(db.ast.as_name(*m), Some("trap" | "wrap")))
                };
                if subs.is_empty() || !subs.iter().all(|&s| well_formed_sub(s)) {
                    faults.push(
                        Reject::coded(
                            Code::MalformedDirective,
                            "`overflow` takes one or both of `(signed <mode>)` and `(unsigned <mode>)`, \
                             each naming `trap` or `wrap` (e.g. `(pragma overflow (signed trap) (unsigned wrap))`)",
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
    //
    // PER-MODULE (per-file), not global — the same reasoning as the duplicate-TYPE check below: a value-
    // name set is fixed within ONE module, but two SEPARATE modules of a linked package may each define a
    // private helper of the same name (`(def node-count …)` in a lib AND in the importing entry — each
    // module has its own value namespace; a sibling's un-imported def is invisible, so re-using its name
    // is not a redeclaration). Key the seen-set on `(file, name)` via the same `file_of` identity the
    // resolver scopes name visibility by (`None` for a single-file program collapses to one bucket — the
    // flat case is byte-identical to the old global scan). Without the file key, a global scan flagged a
    // cross-module same-named def as a spurious duplicate — blocking the idiomatic multi-module layout.
    let mut seen: std::collections::HashSet<(Option<usize>, &str)> =
        std::collections::HashSet::new();
    let dups: Vec<(String, StructId)> = db
        .defs
        .iter()
        .filter(|d| {
            !d.internal
                && !d.name.is_empty()
                && !seen.insert((db.file_of(d.sig_occ), d.name.as_str()))
        })
        .map(|d| (d.name.clone(), d.sig_occ))
        .collect();
    for (name, sig_occ) in dups {
        // Each definition after the first with a given name is a REDUNDANT declaration — DELETE it (the
        // first already binds the name; a module's names are a fixed set). The delete target is the whole
        // `(def <sig> <body>)` FORM (the parent of the signature occurrence), so the fix removes the entire
        // redundant definition, not just its signature — the def analogue of the duplicate-variant / -type
        // / -export / -operation delete fixes. A `sig_occ` with no parent (a malformed shape) anchors + is
        // deleted at the signature itself. Heuristic: deleting the LATER def is the direct resolution, but
        // the author may have meant to keep the second and rename/remove the first — so an agent confirms.
        let delete_at = db.parent_of(sig_occ).unwrap_or(sig_occ);
        faults.push(
            Reject::coded(
                Code::Malformed,
                format!("`{name}` is defined more than once (a module has a fixed set of names)"),
            )
            .at(sig_occ)
            .with_fix(crate::diag::Fix::delete_heuristic(
                delete_at,
                format!("remove the duplicate definition of `{name}`"),
            )),
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
    // DUPLICATE MODULE DECLARATION. A `(module a …)` declaration BINDS its name `a` in the enclosing
    // scope to a record of its exports (`modules-and-namespaces.md` / `11-modules.sexp`: "a module binds
    // its name in the enclosing scope") — reached by member access `(. a field)`, exactly like a `def`/
    // `type` name. So declaring `(module a …)` TWICE in ONE scope is the same fixed-name-set ill-formedness
    // a duplicate `type`/`def`/`export` is. It was SILENTLY ACCEPTED: both `ModuleDecl`s register, and
    // member access resolved INCONSISTENTLY (`(. a g)` and `(. a h)` each found a DIFFERENT one of the two
    // `a`s) — a genuine ambiguity, not the first-wins DISTINCT that same-named EFFECTS deliberately have
    // (an effect is reached through a handler naming it, not by name-in-scope; a module IS a name binding).
    // Reject each declaration after the first CDZ0201, with a delete fix. Keyed on `(parent, name)` — the
    // enclosing form is the scope, so two same-named modules under the SAME parent collide, while the same
    // name in DIFFERENT parents (a nested `inner` in two separate `outer`s, or two files) stays distinct.
    let mut seen_modules: std::collections::HashSet<(Option<StructId>, &str)> =
        std::collections::HashSet::new();
    let dup_modules: Vec<(String, StructId)> = db
        .modules
        .iter()
        .filter(|m| db.is_user_node(m.occ))
        .filter(|m| {
            !m.name.is_empty() && !seen_modules.insert((db.parent_of(m.occ), m.name.as_str()))
        })
        .map(|m| (m.name.clone(), m.occ))
        .collect();
    for (name, occ) in dup_modules {
        faults.push(
            Reject::coded(
                Code::Malformed,
                format!(
                    "module `{name}` is declared more than once in this scope (a module binds its name, \
                     and a scope has a fixed set of names)"
                ),
            )
            .at(occ)
            .with_fix(crate::diag::Fix::delete_heuristic(
                occ,
                format!("remove the duplicate declaration of module `{name}`"),
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
    // Validate every top-level DEFINITION's BODY COUNT. A def is `(def <sig> <body>)` — exactly ONE body.
    // `scan_top_level` reads `body = tail.get(1)` and IGNORES the rest, so two shapes slipped through:
    //   • NO body — `(def (main))` — leaving `body: None`. This surfaced ONLY at emit (`layout` declined
    //     "definition has no body", uncoded, so `cdz check` reported nothing — a check≡compile gap).
    //   • TOO MANY bodies — `(def (main) 1 2)` — silently ACCEPTED, the trailing `2` dropped (a silent
    //     miscompile, the `def` analogue of the M108 let/fn surplus check + a likely `do`-sequencing slip).
    // Reject both here CDZ0201 so BOTH surfaces report it: the no-body case anchored at the def form; the
    // too-many case with a delete-the-surplus fix. Walked over the raw `(def …)` AST tail (the def FORM is
    // the sig occurrence's parent), for USER definitions — gated on `is_user_node(sig_occ)` so a
    // SYNTHESIZED def (a module-member alias, a β-reduced copy — well-formed by construction) is skipped
    // and a QUOTED `(def …)` (inert data, not a declaration) is never flagged. This covers BOTH a
    // top-level def AND a do-local FUNCTION def (registered INTERNAL by `register_do_local_callables` but
    // still a user node), so `(do (def (helper x) x 99) …)` is caught too. A `sig_occ` shared by two
    // registered defs (a do-local recursive fn registered under both its original and a β-copy) is
    // de-duplicated so its form is checked once. A VALUE def `(def NAME VALUE)` and a FUNCTION def
    // `(def (NAME p…) BODY)` share the shape — both `<head> <sig> <body>` — so the tail after `def` is
    // `[sig, body…]`.
    let mut seen_def_forms: std::collections::HashSet<StructId> = std::collections::HashSet::new();
    let user_def_forms: Vec<StructId> = db
        .defs
        .iter()
        .filter(|d| db.is_user_node(d.sig_occ))
        .filter_map(|d| db.parent_of(d.sig_occ))
        .filter(|&form| seen_def_forms.insert(form))
        .collect();
    for form in user_def_forms {
        let Some(tail) = db.ast.as_form(form, "def").map(|t| t.to_vec()) else {
            continue;
        };
        // tail[0] is the signature; tail[1] the body; tail[2..] surplus. A `(doc …)`-wrapped def is
        // normalized away before scan (db.rs), so the tail here is the bare `(def sig body…)`.
        match tail.len() {
            0 | 1 => {
                faults.push(
                    Reject::coded(
                        Code::Malformed,
                        "this definition has no body — a definition is `(def <name> <value>)` or \
                         `(def (<name> <param>…) <body>)`",
                    )
                    .at(form),
                );
            }
            2 => {
                // Well-formed ARITY, but the SIGNATURE must name the definition. `(def () <body>)` (an
                // empty signature list) and `(def (5 x) …)` (a non-name head) register a def with an EMPTY
                // name (db.rs `scan_top_level` falls to the `String::new()` arm) — which was SILENTLY
                // ACCEPTED: the def is unreachable (nothing can name it) and unexportable, a dead
                // declaration. Reject it CDZ0201 — a def is `(def <name> …)` / `(def (<name> <param>…) …)`.
                // (A bare-NAME value-def sig `(def answer 42)` and a proper function sig `(def (f x) …)`
                // both yield a name and pass; only a nameless sig fails.)
                let sig = tail[0];
                let has_name = match db.ast.get(sig) {
                    crate::ast::Struct::Atom(_) => db.ast.as_name(sig).is_some(),
                    crate::ast::Struct::List(children) => children
                        .first()
                        .is_some_and(|&h| db.ast.as_name(h).is_some()),
                };
                if !has_name {
                    faults.push(
                        Reject::coded(
                            Code::Malformed,
                            "this definition has no name — a definition is `(def <name> <value>)` or \
                             `(def (<name> <param>…) <body>)`; the signature must name it (an empty `()` \
                             or a non-name head binds nothing)",
                        )
                        .at(form),
                    );
                }
            }
            _ => {
                // TOO MANY bodies — delete the first surplus (tail[2]); `fixed_arity_reject`'s
                // surplus-delete, the same as let/fn/resume/host. The message names `(do …)` as the way to
                // sequence multiple statements, matching the let/fn wording.
                faults.push(crate::resolve::fixed_arity_reject(
                    form,
                    &tail,
                    2,
                    "this definition has more than one body — a definition takes a single body \
                     (wrap multiple statements in a `(do …)`)",
                ));
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
    // Only USER type declarations are validated — a prelude/synthesized decl's payloads were well-formed
    // when built, and re-checking them against the CURRENT namespace produces a false positive when the
    // user SHADOWS a payload-type name: `(def (Int64) 1)` rebinds `Int64` to a nullary function, so a
    // prelude sum whose payload is typed `Int64` (e.g. an internal `Result`/pair) then fails
    // `validate_type_position` ("a variant payload requires a type") — reported at the PRELUDE payload
    // node, which has no source span, so the fault printed with no `line:col` and a message about a
    // "variant payload" the user never wrote. Gate on `is_user_node(d.occ)`, exactly as the effect-decl
    // payload loop below does, so shadowing a prelude type name is a plain rebind, not a phantom fault.
    let mut type_positions: Vec<(StructId, Vec<String>)> = Vec::new();
    for d in &db.type_decls {
        if !db.is_user_node(d.occ) {
            continue;
        }
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
    // DUPLICATE FIELD in a RECORD TYPE — `(Record (x Int64) (x Bool))`. A record's field names are a fixed
    // SET (the same rule the record VALUE `(record (a 1) (a 2))` is rejected for, `resolve::read_record_fields`),
    // but a record TYPE built the field map by last-wins insert (`eval::RecordCtor`), silently accepting the
    // duplicate (`x` became `Bool`) — so a `(Record (x Int64) (x Bool))` annotation / payload compiled as
    // `(Record (x Bool))` with no diagnostic. Scan every user `(Record …)` TYPE form (the capital-R head is
    // unambiguously a type form, distinct from the lowercase `record` value alias) for a repeated field name
    // and reject it CDZ0201, anchored at the redundant `(name Type)` entry with a delete fix — the type-form
    // twin of the value-form duplicate-field reject.
    for i in 0..db.ast.structure.len() as u32 {
        let id = StructId(i);
        if !db.is_user_node(id) || db.ast.head_name(id) != Some("Record") {
            continue;
        }
        let crate::ast::Struct::List(children) = db.ast.get(id) else {
            continue;
        };
        let field_entries = children[1..].to_vec();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for entry in field_entries {
            // Each field entry is the canonical `(: name Type)` ascription (RT3) or the legacy
            // `(name Type)` head-app pair; read the field name from whichever form. (Without the
            // ascription arm the duplicate-field check silently passed once the encoder/corpus moved to
            // ascription — the field name went unread, so a repeated `x` slipped through.)
            let Some(name) = (match db.ast.as_form(entry, ":") {
                Some([name_occ, _ty]) => db.ast.as_name(*name_occ),
                _ => match db.ast.get(entry) {
                    crate::ast::Struct::List(kv) if kv.len() == 2 => db.ast.as_name(kv[0]),
                    _ => None,
                },
            })
            .map(str::to_string) else {
                continue;
            };
            if !seen.insert(name.clone()) {
                faults.push(
                    Reject::coded(
                        Code::Malformed,
                        format!("record type names field `{name}` more than once"),
                    )
                    .at(entry)
                    .with_fix(crate::diag::Fix::delete_heuristic(
                        entry,
                        format!("remove the duplicate `{name}` field"),
                    )),
                );
            }
        }
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
    // A MALFORMED EFFECT CLAUSE — a clause that is NOT an `(op …)` operation. An effect's members are its
    // operations, `(op <name> (-> …))` (`capabilities-and-effects.md` §An Effect Declaration Names The
    // Effect And Types Its Operations). `scan_effect_decl` SILENTLY DROPS any clause whose head is not
    // `op` (a bare literal `(effect E 5)`, a non-`op`-headed list `(effect E (foo …))`) AND an `(op)` with
    // no name at all (its `op_tail.first()` is `None`) — so a typo'd/garbled operation vanishes and the
    // effect looks like it has fewer ops than written (a match/handle over it then wrongly type-checks as
    // exhaustive — a correctness hazard, the effect analogue of the malformed-variant scan-drop). Reject
    // each such clause CDZ0201 at the clause. A leading `(doc "…")` clause is TOLERATED (the doc affordance
    // effects share with defs — silently ignored, not a fault). Walked over the raw `(effect …)` AST tail
    // (the scanned `ops` already dropped the bad ones), for USER effect declarations only.
    let effect_decl_occs: Vec<StructId> = db
        .effect_decls
        .iter()
        .filter(|e| db.is_user_node(e.occ))
        .map(|e| e.occ)
        .collect();
    for occ in effect_decl_occs {
        let Some(tail) = db.ast.as_form(occ, "effect").map(<[_]>::to_vec) else {
            continue;
        };
        // tail[0] is the effect NAME; tail[1..] are its clauses — each must be an `(op …)` (or a tolerated
        // `(doc …)`).
        for &clause in tail.iter().skip(1) {
            let head = db.ast.head_name(clause);
            // A well-shaped `(op …)` clause is validated by the nameless-ops / op-type checks; a `(doc …)`
            // clause is tolerated. Anything else (a bare atom, a non-op/doc-headed list, an `(op)` with no
            // name) never registered an operation — reject it here.
            let is_op =
                head == Some("op") && db.ast.as_form(clause, "op").is_some_and(|t| !t.is_empty());
            let is_doc = head == Some("doc");
            if !is_op && !is_doc {
                faults.push(
                    Reject::coded(
                        Code::Malformed,
                        "an effect clause must be an operation `(op <name> (-> Arg… Result))` — this is \
                         not one, so it declares no operation",
                    )
                    .at(clause),
                );
            }
        }
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
    // An operation declared with NO type at all — `(op get)` — has `op.ty == None`. It was SILENTLY
    // ACCEPTED (this loop `continue`d past it), and performing it later leaked the internal op-record type
    // `(Record (apply Any) (effect-op Any) (t Type))` at the user (the leaky-representation anti-pattern) —
    // the missing-type companion of the non-arrow `(op get Int64)` case below. Collect each such op's NAME
    // occurrence to reject after the borrow (the same defer-then-report shape as `non_arrow_op_types`).
    let mut missing_op_types: Vec<StructId> = Vec::new();
    for e in &db.effect_decls {
        for op in &e.ops {
            let Some(ty) = op.ty else {
                missing_op_types.push(op.name_occ);
                continue;
            };
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
    // An operation declared with NO type — `(op get)`. Reject it CDZ0201 at the op NAME (the companion of
    // the non-arrow reject above). NO mechanical fix: the non-arrow case WRAPS an existing type it can see,
    // but here there is no type at all — the operation's actual signature is a semantic choice only the
    // author knows (a `(-> Result)` guess would compile but is almost certainly wrong), so this is
    // honestly message-only (like a "retype the value" fault). The message spells the exact shape a
    // well-formed op takes, so the author knows precisely what to write.
    for &name_occ in &missing_op_types {
        faults.push(
            Reject::coded(
                Code::Malformed,
                "this operation has no type — an operation is declared `(op <name> (-> Arg… Result))` \
                 and performed like a function (a nullary operation is `(op <name> (-> Result))`)",
            )
            .at(name_occ),
        );
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
    // A `(fn (<param>…) <body>)` LAMBDA parameter is the SAME binder position as a def parameter, so the
    // same rule holds: a bare LITERAL parameter binds nothing. But the def-param scan above reads only
    // `db.defs` (top-level definitions), and a lambda's parameters are never registered there — so
    // `(fn (5) …)` / `(fn (true) …)` was SILENTLY ACCEPTED while the def twin `(def (f 5) …)` is rejected,
    // an asymmetry between the two binder forms. `resolve_lambda` validates the param LIST's shape (it must
    // be a list, the body must be present) but not that each element is a binder. Scan every `(fn …)` form
    // in the arena and apply the identical predicate (a bare atom that is not a name), so the two binder
    // positions reject a literal parameter identically. A COMPOUND param is a destructuring pattern (left to
    // the binding-pattern path); `_` and a `(: name T)` binder `as_name`/list out exactly as for a def.
    let malformed_params: Vec<StructId> = db
        .defs
        .iter()
        .flat_map(|d| d.params.clone())
        .chain(
            (0..db.ast.structure.len() as u32)
                .map(StructId)
                // A GENUINE user lambda is `(fn (<param>…) <body>)` — its tail is `[params, body]`, length
                // ≥ 2. Requiring a BODY (`tail.len() >= 2`) is what distinguishes a real lambda from a
                // SYNTHESIZED `(fn …)` node: `modules::synthesize` / the type-record synthesis wrap a
                // declaration in a `(fn …)`-headed record whose "params" position holds member/variant
                // occurrences, NOT lambda binders — and a type-record synth is `(fn <record>)` (tail length
                // 1, no body). Scanning those would false-flag a variant NAMED `fn` (it collides with the
                // lambda keyword and reifies to a non-name atom). A bodyless user `(fn (5))` is caught by
                // `resolve_lambda`'s own "no body" reject, so requiring a body loses no real diagnostic.
                .filter(|&id| db.ast.as_form(id, "fn").is_some_and(|t| t.len() >= 2))
                .filter_map(|id| db.ast.as_form(id, "fn").and_then(|t| t.first().copied()))
                .filter_map(|params_occ| match db.ast.get(params_occ) {
                    crate::ast::Struct::List(ps) => Some(ps.clone()),
                    _ => None, // a non-list fn param position is `resolve_lambda`'s shape reject
                })
                .flatten(),
        )
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
        crate::infer::param_list_linearity_faults(db, params, &mut faults);
    }
    // Check EVERY definition's body — reachable or not. (The demand is still lazy per node; this just
    // demands each definition once, which is what well-formedness requires.)
    // A def is an ENTRYPOINT if it is exported — the only context where a body is lowered STANDALONE as
    // the emitted artifact (a nullary export IS its value; a parameterized export is the boundary
    // function, with no internal call site). A non-exported def is always inlined at its call sites (or
    // dead), so its standalone lowering is not what ships; its reached-poison walk would fault on a
    // decline that the inline at the call site resolves (e.g. a library def that performs an effect whose
    // home is its caller's handler). So run the reached-poison walk on EXPORTED bodies — a nullary export
    // surfaces every reached poison; a parameterized export surfaces the CODED ones (the per-body loop
    // below).
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
    let bodies: Vec<(StructId, bool, bool)> = db
        .defs
        .iter()
        .filter(|d| d.body.is_none_or(|b| !cyclic_bodies.contains(&b)))
        .filter_map(|d| {
            d.body
                .map(|b| (b, d.params.is_empty(), exported_bodies.contains(&b)))
        })
        .collect();
    for (body, is_nullary, is_exported) in bodies {
        // PER-DEF-BODY reset of the structural-reduction work counter (see `STRUCTURAL_REDUCTION_BUDGET`):
        // this body's `type_errors`/reached-poison walk gets its OWN budget, so a divergent body (the
        // self-app structural-explosion HANG) trips fast while the whole-compile cumulative total stays
        // unbounded (a real multi-module compile legitimately does ~800k structural reductions across all
        // bodies — a program-level counter would false-decline it, the Option.None carve-out regression).
        db.structural_reductions = 0;
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
        // lowers the body; it runs only on an EXPORTED body (a non-exported def is inlined / reached at a
        // call site, where its traps surface — walking it standalone would fault on a decline the inline
        // resolves, e.g. a library def whose performed effect's home is its caller's handler).
        faults.extend(type_errors(db, body));
        if is_exported {
            if is_nullary {
                // A nullary EXPORTED value is lowered standalone as the emitted artifact — every reached
                // poison (coded OR codeless) is a real build failure.
                collect_reached_poisons(db, body, &mut faults);
            } else {
                // A PARAMETERIZED EXPORTED body is ALSO lowered standalone — it is the boundary export
                // function, with NO internal call site (the boundary IS the entry). So its unconditionally-
                // reached poisons are real (emit hits them). Surface the CODED ones, satisfying the
                // documented `cdz check` contract that check reports every CODED fault (a codeless not-yet
                // decline stays out of scope). This closes the #7143/#7210 parity gap where a coded
                // compound-ordering / compare / to-list CDZ0203 in a parameterized export was SILENT in
                // `cdz check` yet REJECTED by `cdz compile` — a coded-fault-invisible-to-check contract
                // violation (breaker). The reached-poison walk descends only UNCONDITIONALLY-reached
                // positions (never a guarded `if`/`match` arm), so it is a strict SUBSET of what emit
                // lowers — check ⊆ compile, so this can never make check reject a program compile accepts.
                let mut param_poisons = Vec::new();
                collect_reached_poisons(db, body, &mut param_poisons);
                faults.extend(param_poisons.into_iter().filter(|r| r.code.is_some()));
            }
        }
    }
    // MODULE-MEMBER VALUE/NULLARY BODIES. `modules::register_fn_def` registers only a ≥1-PARAM member in
    // `db.defs` (for recursive-call lowering), so a bare-name VALUE `(def v V)` or a NULLARY `(def (v) V)`
    // module member is NOT in `db.defs` and its body was NOT type-checked by the loop above — an ill-typed
    // one (`(module m (def (bad) (+ 1 2.0)))`) slipped through to a DECLINE or an INVALID COMPONENT
    // (`type-system.md §A program that is not well-typed MUST be rejected`). Type-check each such member
    // body here so it rejects with its code (CDZ0301/…) rather than miscompiling. A ≥1-param member's body
    // IS in `db.defs` (already checked above), so gather it into `checked_bodies` to avoid a redundant
    // second walk. A member reached only through its module's synthesized record still runs this check —
    // well-formedness is unconditional over every definition (`§454`).
    let checked_bodies: std::collections::HashSet<StructId> =
        db.defs.iter().filter_map(|d| d.body).collect();
    let member_value_bodies: Vec<StructId> = db
        .modules
        .iter()
        .flat_map(|m| module_member_value_bodies(db, m.occ))
        .filter(|b| !checked_bodies.contains(b))
        .collect();
    for body in member_value_bodies {
        // Keep only the TYPE faults, DROPPING an `Unbound` (CDZ0101): a member body references its
        // SIBLINGS by bare name (a sibling effect `log`, a sibling def), which resolve through the module's
        // in-scope context — but this STANDALONE `type_errors` walk re-resolves from the body node without
        // that context and spuriously reports the sibling `Unbound`. A GENUINELY unbound name still faults
        // where the member is actually reached (the reached-poison walk over the export that inlines it, or
        // the projection site), so dropping the standalone `Unbound` loses no real error while it removes
        // the false positive. The MISCOMPILE this pass exists to catch is a TYPE fault (CDZ0301/0302/0303 —
        // a numeric mix that would emit an invalid component), which is scope-independent and kept.
        for fault in type_errors(db, body) {
            if fault.code != Some(Code::Unbound) {
                faults.push(fault);
            }
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
        // Only a USER (source-written) `resume` can be a stray one — a SYNTHESIZED reduction copy (a
        // `beta_reduce`/handle-fold node) is not source the author can move. A MALFORMED resume
        // (`(resume v)` / `(resume v s extra)`) reports its own CDZ0201 at its source node (the resolve
        // poison), and the handle fold then produces a spanless COPY of the arm body that lands outside a
        // recognizable handle-arm structure — which `is_stray_resume` would wrongly flag "stray", a
        // MISLEADING second error (the resume IS in an arm, just malformed). Gating on `is_user_node`
        // drops that synthesized copy so a malformed resume reports ONE primary fault (its shape), while a
        // genuine top-level `(resume …)` — a user node with no enclosing arm — is still flagged.
        if db.ast.head_name(id) == Some("resume")
            && db.is_user_node(id)
            && crate::resolve::is_stray_resume(db, id)
        {
            faults.push(Reject::coded(Code::Malformed, crate::diag::STRAY_RESUME_MESSAGE).at(id));
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
                // A declared TYPE named bare in `(export T)` is the OPAQUE-TYPES handle export — a valid
                // form whose consumer is an IMPORTING peer (`(import "lib" (T …))`), which exists only in a
                // linked PACKAGE. In a single-module compile there is no importer, so the handle export has
                // no effect here; but it is NOT the "only definitions are exported" category error the old
                // message claimed (opaque types made a type handle first-class exportable). Say THAT — the
                // export is meaningful only across a package boundary — so an author writing an abstract
                // type is not misled into thinking a type can never be exported. An EFFECT is still a true
                // category error (an effect is not an exportable entity), and an unknown name stays "names
                // no definition".
                let message = if db.type_decl_by_name(&name).is_some() {
                    format!(
                        "export `{name}` names a TYPE — a bare type export is the abstract-type HANDLE \
                         export (opaque types), meaningful only when a peer module imports it; in a \
                         single module it has no importer, so it exports nothing here. Export a value \
                         `(def …)`, or `(export (. {name} *))` to publish its constructors too."
                    )
                } else if db.effect_decl_by_name(&name).is_some() {
                    format!(
                        "export `{name}` names an effect, not a value definition — only definitions are \
                         exported (a module's exports are the values its definitions bind)"
                    )
                } else {
                    format!("export `{name}` names no definition")
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
        // A bad element whose head is `.` is a MALFORMED CONSTRUCTOR-EXPORT — the author wrote `(. …)`
        // intending the opaque-types surface `(export (. T A))` / `(export (. T *))` but got the shape
        // wrong (a `(. T)` with no ctor, a `(. T A B)` with too many segments). Name THAT form, not the
        // generic "an export names a definition" (which reads as "only bare names allowed" — false, a
        // ctor-export is valid) — and offer NO `.`-head replace fix (which would be nonsense). A non-`.`
        // bad element keeps the generic message + its head-name replace fix.
        let is_ctor_export_attempt = bad_arg.is_some_and(|a| db.ast.head_name(a) == Some("."));
        let message = if is_ctor_export_attempt {
            "a constructor-export is `(export (. T A))` (the handle + constructor `A`) or `(export \
             (. T *))` (the handle + all constructors) — this `(. …)` form is neither"
                .to_string()
        } else {
            "an export names a definition — write `(export <name>)`, e.g. `(export main)` \
             (an export clause is one or more bare definition names)"
                .to_string()
        };
        let mut reject = Reject::coded(Code::Malformed, message).at(anchor);
        // When the bad element is a compound whose HEAD is a name — `(export (g x))` — the author most
        // likely meant to export `g`; offer replacing that element with the bare `<head>`. A non-name
        // atom (`(export 5)`, the `5` in `(export a 5)`) or an empty `(export)` has no name to recover →
        // message only. A `.`-headed ctor-export attempt gets NO fix (the head `.` is not a name to
        // recover, and which valid ctor-export shape was meant is a guess).
        if !is_ctor_export_attempt
            && let Some(arg) = bad_arg
            && let Some(head) = db.ast.head_name(arg)
        {
            reject = reject.with_fix(crate::diag::Fix::replace_heuristic(arg, head.to_string()));
        }
        faults.push(reject);
    }
    // A BARE zero-operand DECLARATION form — `(def)` / `(type)` / `(effect)` — declares nothing (no name,
    // no body/variants/ops) yet was SILENTLY ACCEPTED: it registers no `Def`/`TypeDecl`/`EffectDecl`, so
    // the per-declaration validation walks never see it, and `unknown_top_forms` skips it (its head IS a
    // known keyword). The `(export)` empty case is caught above; this is its `def`/`type`/`effect` sibling.
    // Reject each CDZ0201, naming the form's canonical shape.
    for (kw, occ) in db.empty_declaration_forms() {
        let shape = match kw {
            "def" => "a definition is `(def <name> <value>)` or `(def (<name> <param>…) <body>)`",
            "type" => "a type declaration is `(type <Name> (<Variant> <payload>…)…)`",
            _ => "an effect declaration is `(effect <Name> (op <name> <type>)…)`",
        };
        faults.push(
            Reject::coded(
                Code::Malformed,
                format!("this `({kw})` declares nothing — {shape}"),
            )
            .at(occ),
        );
    }
    // A MALFORMED top-level `@`-annotation form — a bare `(@)`, a name-only `(@ test)`, or a non-form
    // target `(@ test 5)` — that SURVIVED `strip_annotations` because it wraps no definition. `@` is the
    // GENERAL-PURPOSE annotation head `(@ <name> (def …))`; `strip_annotations` unwraps every def-wrapping
    // annotation in place (even an unknown name — a transparent forward-compat marker), so a SURVIVING
    // top-level `(@ …)` has no wrappable target. Left alone it resolves as a misleading "unbound name `@`
    // at the top level" (`@` is a recognized head, not a name) plus a phantom unbound-name for any def it
    // hid. Reject each CDZ0201 naming the annotation shape + how an annotation attaches to a definition.
    for occ in db.malformed_annotation_forms() {
        faults.push(
            Reject::coded(
                Code::Malformed,
                "this `(@ …)` annotation wraps no definition — an annotation is `(@ <name> (def …))`, \
                 e.g. `(@ test (def (t) 1))` (the annotation name, then the definition it marks)"
                    .to_string(),
            )
            .at(occ),
        );
    }
    // A MALFORMED `@tag` — `(@ (tag …) def)` whose argument is not exactly ONE STRING (`@tag(5)`,
    // `@tag(foo)`, `@tag()`, `@tag("a" "b")`). A `@tag` takes exactly one string literal (the tag text
    // `cdz test --tag` matches). Without this reject the malformed tag is SILENTLY DROPPED — the def is
    // untagged and the author's intent (to tag it) is lost with no signal — so a mis-typed `--tag` filter
    // then matches nothing. Reject each, naming the required shape.
    for &occ in db.malformed_tag_forms() {
        faults.push(
            Reject::coded(
                Code::Malformed,
                "this `@tag` annotation takes exactly one STRING argument — the tag text, e.g. \
                 `@tag(\"slow\")` (`(@ (tag \"slow\") (def …))`); a non-string, missing, or multiple \
                 argument is not a tag and would be silently ignored"
                    .to_string(),
            )
            .at(occ),
        );
    }
    // A `@requires`/`@ensures` with not-exactly-one PREDICATE argument (`@requires()`, `@requires(a b)`) is
    // a shape error the verification layer cannot model — silently recording no predicate would mask the
    // author's mistake (exactly the `@tag` masking bug). Reject each, naming the required one-predicate
    // shape. (Whether the predicate's NAMES resolve + it is boolean is checked later, at denotation, where
    // the def param scope + the `it` binder are in scope — reported there at the annotation span.)
    for &occ in db.malformed_verify_forms() {
        faults.push(
            Reject::coded(
                Code::Malformed,
                "a `@requires`/`@ensures`/`@invariant` annotation takes exactly one PREDICATE argument — a \
                 boolean expression over the def's parameters (and, for `@ensures`/`@invariant`, the result/\
                 value binder `it`), e.g. `@requires(> x 0)` (`(@ (requires (> x 0)) (def …))`) or \
                 `@invariant(> (len it) 0)` (`(@ (invariant …) (type …))`); a missing or multiple argument \
                 is not a well-formed condition and would be silently ignored"
                    .to_string(),
            )
            .at(occ),
        );
    }
    // b4c NAME-RESOLUTION: a `@requires`/`@ensures` predicate references only names in scope — the def's
    // PARAMETERS, the result binder `it` (for `@ensures`), and prelude/global names. A name that is NONE of
    // those is UNBOUND — reported CDZ0101 AT the annotation (good locality), not far away at denotation.
    // The recorded predicate was stripped from the def body so it has no scope of its own; rather than
    // re-parent it (broad blast radius), we check each NAME OCCURRENCE it references: skip the def's param
    // names and (for `@ensures`) `it`, and for every OTHER name test whether it resolves standalone (a
    // prelude op like `>`/`+` does; a stray name resolves to `Poison(CDZ0101)`). This is exact for the flat
    // arithmetic fragment (no predicate-local binders yet); a future nested-binder predicate would extend
    // the skip set with the predicate's own bindings.
    for di in 0..db.defs.len() {
        let param_names = def_param_names(db, di);
        for &(pred, is_ens) in &verify_predicates_of(db, di) {
            // `@ensures` binds the result subject `ret`; `@requires` binds no subject (only params + prelude).
            let subject = if is_ens {
                Some(crate::verify_enforce::RESULT_BINDER)
            } else {
                None
            };
            if let Some(reject) = first_unbound_predicate_name(db, pred, &param_names, subject) {
                faults.push(reject);
            }
        }
    }
    // The SAME name-resolution for an `@invariant(pred)` on a TYPE: the predicate references only the value
    // binder `self` (the value of the type) and prelude/global names — no def params (a type has none). A stray
    // name is UNBOUND → CDZ0101 at the annotation. `@invariant`'s subject binder is `self` (the value being
    // checked), a DISTINCT name from `@ensures`'s `ret` (the return value) — the operator ruled each family
    // member gets the name that fits its meaning (*"ret for ensures and self for invariants"*, 2026-07-18).
    let invariant_preds: Vec<StructId> = db.invariant_preds();
    for pred in invariant_preds {
        if let Some(reject) = first_unbound_predicate_name(
            db,
            pred,
            &[],
            Some(crate::invariant_establish::VALUE_BINDER),
        ) {
            faults.push(reject);
        }
    }
    // `@ensures`-CAPTURE-GUARD REJECT (breaker 2026-07-17). `@ensures(Q)` enforcement binds the def's RESULT
    // to `ret` (`(let ((ret BODY)) (if Q ret (trap)))`, `verify_enforce`). If a PARAMETER is
    // literally named `ret`, that binder would SHADOW the param — so `verify_enforce` SKIPS the `@ensures`
    // enforcement for such a def. Skipping SILENTLY is a footgun: the author wrote a postcondition that is
    // quietly NOT enforced (a violating result returns with no trap, no diagnostic). REJECT it instead — a
    // stated contract is enforced OR the author is told precisely why not, never silently dropped (the (D)
    // philosophy). The fix is trivial (rename the param), so name it. Anchored at the first `@ensures` predicate
    // occ (good locality). (The result binder was renamed `it` → `ret` per the operator's collision-safety
    // directive; a user naming a param `ret` is now vanishingly unlikely, but the guard stays for soundness.)
    for di in 0..db.defs.len() {
        let ensures = db.ensures_of(di);
        let Some(&first_ensures) = ensures.first() else {
            continue; // no @ensures on this def — the guard only applies to @ensures
        };
        if def_param_names(db, di)
            .iter()
            .any(|n| n == crate::verify_enforce::RESULT_BINDER)
        {
            faults.push(
                Reject::coded(
                    Code::Malformed,
                    "a `def` with a parameter named `ret` cannot carry `@ensures`: the `@ensures` result \
                     binder `ret` would shadow the parameter, so the postcondition would be silently \
                     unenforced — rename the parameter (e.g. `ret` → `x`) so the postcondition can bind \
                     the result"
                        .to_string(),
                )
                .at(first_ensures),
            );
        }
    }
    // SEMANTIC validation of a CONSTRUCTOR-EXPORT `(export (. T A))` / `(export (. T *))` — the opaque-types
    // surface. `malformed_exports` (above) accepts its SHAPE, and the linker's `as_ctor_export` records the
    // (type, ctor) names WITHOUT checking they exist — so `(export (. T Nonesuch))` (a ctor `T` lacks),
    // `(export (. foo A))` (`foo` is a value/effect, not a sum), and `(export (. Undeclared A))` were all
    // SILENTLY ACCEPTED. Reject each here: the head must name a declared SUM TYPE, and (unless the `*`
    // wildcard) the ctor must be one of its variants — with a did-you-mean over the type's variant names.
    let ctor_exports = db.ctor_export_elements();
    for elem in ctor_exports {
        let crate::db::CtorExportElem {
            elem: elem_occ,
            ty_name,
            ctor_key: ctor_name,
            ty_anchor,
            ctor_anchor,
            is_atom,
        } = elem;
        // The head must name a declared SUM TYPE. A value def / effect / undeclared name is not one — a
        // ctor-export of a non-type has no constructors to publish. `type_decl_by_name` returns the sum's
        // SYNTHESIZED RECORD occurrence; `type_decl_by_synth` recovers the `TypeDecl` (with its variants).
        let Some(decl) = db
            .type_decl_by_name(&ty_name)
            .and_then(|synth| db.type_decl_by_synth(synth))
        else {
            let category = if db.def_by_name(&ty_name).is_some() {
                "a value definition"
            } else if db.effect_decl_by_name(&ty_name).is_some() {
                "an effect"
            } else {
                "not a declared type"
            };
            // When the head is an UNDECLARED name that is a plausible typo of a declared SUM TYPE — `(. Colr
            // *)` for a `(type Color …)` — name it + carry a rename fix on the TYPE-name occurrence, the
            // type-name twin of the ctor-name did-you-mean below. Only for the "not a declared type" case (a
            // value def / effect with that exact name is a different mistake, kept as its own category); the
            // candidate set is the declared type names (a closed set).
            let type_names: Vec<String> = db.type_decls.iter().map(|t| t.name.clone()).collect();
            let type_hint = if db.def_by_name(&ty_name).is_none()
                && db.effect_decl_by_name(&ty_name).is_none()
            {
                crate::diag::suggest::nearest(&ty_name, type_names.iter().map(String::as_str))
            } else {
                None
            };
            let mut reject = Reject::coded(
                Code::Malformed,
                format!(
                    "a constructor-export `(export (. {ty_name} …))` needs `{ty_name}` to be a sum \
                     type, but it is {category} — only a sum type has constructors to export{}",
                    type_hint
                        .as_ref()
                        .map(|n| format!(" — did you mean `{n}`?"))
                        .unwrap_or_default()
                ),
            )
            .at(elem_occ);
            if let Some(near) = type_hint {
                // The rename fix targets the TYPE-name occurrence. For the LIST form `(. Colr *)` that is a
                // distinct node (`ty_anchor`), replaced with the bare corrected name. For the dotted ATOM
                // `Colr.*` there is only the one atom node, so the fix replaces the WHOLE atom and its
                // replacement must carry the `.*` tail (`Colr.*` -> `Color.*`) — replacing with just
                // `Color` would drop the wildcard and yield `(export Color)`.
                let (fix_at, replacement) = if is_atom {
                    (elem_occ, format!("{near}.{ctor_name}"))
                } else {
                    (ty_anchor, near)
                };
                reject = reject.with_fix(crate::diag::Fix::replace_heuristic(fix_at, replacement));
            }
            faults.push(reject);
            continue;
        };
        // The wildcard `(. T *)` publishes every ctor — no per-ctor check. A NAMED ctor `(. T A)` must be
        // an actual variant of `T`; name a near-miss over the type's variant names (a closed set).
        if ctor_name == "*" {
            continue;
        }
        let variant_names: Vec<String> = decl.variants.iter().map(|v| v.name.clone()).collect();
        if !variant_names.contains(&ctor_name) {
            let hint = match crate::diag::suggest::nearest(
                &ctor_name,
                variant_names.iter().map(String::as_str),
            ) {
                Some(near) => format!(" — did you mean `{near}`?"),
                None => String::new(),
            };
            let mut reject = Reject::coded(
                Code::Malformed,
                format!(
                    "`{ctor_name}` is not a constructor of the sum type `{ty_name}` — a \
                     constructor-export names one of its variants (or `*` for all){hint}"
                ),
            )
            .at(ctor_anchor);
            if let Some(near) =
                crate::diag::suggest::nearest(&ctor_name, variant_names.iter().map(String::as_str))
            {
                reject = reject.with_fix(crate::diag::Fix::replace_heuristic(ctor_anchor, near));
            }
            faults.push(reject);
        }
    }
    // A MALFORMED `(bind …)` peer-binding directive. `scan_effect_bindings` SILENTLY DROPS a `(bind …)`
    // that is not `(bind <name> <string>)` (wrong arity, or a non-string interface) — so a typo'd binding
    // does nothing, with no diagnostic, and the effect quietly escapes to the host (or is unrouted). And a
    // `(bind foo …)` naming a VALUE definition rather than an effect binds nothing. Reject both here (the
    // `bind` analogue of the malformed-extern scan-and-drop check + the host-delegates-a-value reject): a
    // shape that is not `(bind Effect "iface")` is CDZ0201, and a `bind` whose name is an unambiguous VALUE
    // DEF (not an effect) is flagged. (A name that is neither a top-level effect nor a value def — a
    // module-scoped effect, or a genuinely unbound name — is NOT flagged here: a module effect is legitimate
    // and an unbound name surfaces its own CDZ0101 at the reference; only an unambiguous non-effect def is a
    // certain mis-bind. Uses `def_by_name` — a top-level value def — exactly as the host-delegates-a-value
    // check does, since `effect_decl_by_name` is a top-level-only registry.)
    // Scan only the TOP-LEVEL `(bind …)` directives — the same scope `scan_effect_bindings` uses. An
    // arena-wide scan (`0..structure.len()`) also matches a `(bind …)` list NESTED in a handler arm — an
    // effect declaring an operation named `bind`, whose arm is `(bind (params) s body)` — and misreads it
    // as a malformed peer-binding (arity ≠ 2) → a spurious CDZ0201 on a legal operation name. `bind` is an
    // ordinary identifier; only a top-level `(bind …)` is a peer-binding directive.
    let mut bound_effects: std::collections::HashSet<String> = std::collections::HashSet::new();
    for form in db.top_bind_forms() {
        let Some(btail) = db.ast.as_form(form, "bind").map(<[_]>::to_vec) else {
            continue;
        };
        // Shape: exactly `(bind <name> <string>)`.
        let well_shaped = btail.len() == 2
            && btail.first().is_some_and(|&n| db.ast.as_name(n).is_some())
            && btail.get(1).is_some_and(|&i| db.ast.as_str(i).is_some());
        if !well_shaped {
            let anchor = btail.first().copied().unwrap_or(form);
            faults.push(
                Reject::coded(Code::Malformed, crate::diag::MALFORMED_BIND_MESSAGE).at(anchor),
            );
            continue;
        }
        // Well-shaped: the INTERFACE STRING is a component-boundary name — it is emitted verbatim as the
        // extern name a peer instance import binds under. A non-conforming string (`"Math/API"`) would
        // `kebab_extern_name`-mangle to an invalid extern name and produce a component `wasmtime` rejects
        // at LOAD with no compiler diagnostic (an invalid-component miscompile). Validate it here so a bad
        // interface name is a clear compile-time reject naming the offending string, not a silent failure.
        let iface_occ = btail[1];
        let iface = db.ast.as_str(iface_occ).unwrap().to_string();
        if !crate::backend::common::export_name::is_valid_interface_name(&iface) {
            faults.push(
                Reject::coded(
                    Code::Malformed,
                    crate::diag::MALFORMED_INTERFACE_NAME_MESSAGE,
                )
                .at(iface_occ),
            );
            continue;
        }
        // Well-shaped: the bound name must be a declared EFFECT. A name that is NOT an effect binds
        // nothing and is silently dropped by `scan_effect_bindings`, so flag it here. Two non-effect
        // cases, both certain mis-binds at the TOP LEVEL (a top-level `(bind …)` can only route a
        // top-level effect):
        //   • the name is a VALUE DEF (`(bind foo …)` for a `(def (foo) …)`) — it names something real,
        //     just not an effect;
        //   • the name is UNKNOWN — neither an effect nor a def (`(bind Ghost …)`, or a typo `Loger` of
        //     `Logger`). This used to be SILENTLY ACCEPTED: the comment here assumed an unbound bind-name
        //     surfaces its own CDZ0101 "at the reference", but a `(bind …)` directive OPERAND is not
        //     resolved as a value reference, so nothing flagged it and the bind quietly vanished. Reject it
        //     WITH a did-you-mean over the declared effect names (a `bind` targets an effect, a small
        //     closed set — listing/suggesting is signal).
        let name_occ = btail[0];
        let name = db.ast.as_name(name_occ).unwrap().to_string();
        if db.effect_decl_by_name(&name).is_none() {
            let effect_names: Vec<&str> = db.effect_decls.iter().map(|e| e.name.as_str()).collect();
            let hint = crate::diag::suggest::did_you_mean(&name, effect_names.iter().copied(), 3);
            let mut reject = Reject::coded(
                Code::Malformed,
                format!("{}{hint}", crate::diag::BIND_NOT_AN_EFFECT_MESSAGE),
            )
            .at(name_occ);
            // A confident single typo of an effect name → carry the rename fix on the bind-name occ.
            if let Some(near) = crate::diag::suggest::nearest(&name, effect_names.iter().copied()) {
                reject = reject.with_fix(crate::diag::Fix::replace_heuristic(name_occ, near));
            }
            faults.push(reject);
            continue;
        }
        // A CLOSURE in a peer-bound operation's signature. Peers exchange VALUE-HEAP HANDLES, and a closure
        // is not a value-heap value — it has no peer-boundary form (a closure crosses the HOST boundary as a
        // resource, not a peer). Without this, `(op mk (-> Int64 (-> Int64 Int64)))` bound to a peer
        // type-checks, then APPLYING the peer-returned closure declines at lower time with the opaque "value
        // is not applyable". Detect it SYNTACTICALLY at the binding: each op's declared type is a `(-> A B
        // …)` arrow, and a boundary position that is ITSELF a `(-> …)` list is a closure. Reported at the
        // bind name with the real reason. (Diagnostic-only — the emit path already declines; this just names
        // it. A closure crossing the HOST boundary via `(host …)` is unaffected — this fires only for a
        // peer-BOUND effect.)
        {
            // The declared operation types for this effect. (`effect_decl_by_name` yields the SYNTH-record
            // occ, not the declaration occ `effect_decl_by_occ` keys on — so match the decl by NAME field.)
            let op_tys: Vec<StructId> = db
                .effect_decls
                .iter()
                .find(|e| e.name == name)
                .map(|e| e.ops.iter().filter_map(|o| o.ty).collect())
                .unwrap_or_default();
            let mut closure_op: Option<StructId> = None;
            for ty in op_tys {
                // The op type is `(-> Arg… Result)`; each element after the `->` head is a boundary
                // position. A position that is a list headed by `->` is a closure (function-typed).
                if let Some(arrow) = db.ast.as_form(ty, "->") {
                    for &pos in arrow {
                        if db.ast.as_form(pos, "->").is_some() {
                            closure_op = Some(pos);
                            break;
                        }
                    }
                }
                if closure_op.is_some() {
                    break;
                }
            }
            if closure_op.is_some() {
                // Anchor at the `(bind E …)` name — the ACTIONABLE locus the author edits (change the
                // route, or give the op a value type), NOT the nested `(-> …)` arrow fragment mid-signature
                // that merely DETECTED the closure. (Copilot PR #418: the reject was `.at(pos)`, the inner
                // arrow, while the comment says "reported at the bind name" — align them on the bind name.)
                faults.push(
                    Reject::coded(Code::Malformed, crate::diag::CLOSURE_ACROSS_PEER_MESSAGE)
                        .at(name_occ),
                );
                continue;
            }
            // (A `String`/`Bytes` in an ARGUMENT position of a peer-bound op USED to be declined here — it
            // lowered as a component `string` needing a `mem` option the peer envelope never supplied,
            // yielding an invalid consumer. That is now EMITTED: a peer String/Bytes arg crosses as a runtime
            // rope HANDLE like any compound — `collect_used_ops` + `collect_host_arg_strings` are peer-aware
            // (a peer String arg builds a rope, unlike a HOST String arg marshaled as (ptr,len)), and the
            // peer emit hands over the handle. Pinned e2e by `a_string_argument_crosses_to_a_peer_*`.)
        }
        // A DUPLICATE `(bind E …)` — the same effect bound to a peer TWICE in source. `scan_effect_bindings`
        // silently keeps the FIRST (`.or_insert_with`), so a second directive is a dead, ambiguous line: the
        // author wrote two different routes for one effect and only one takes. This is the same fixed-set
        // ill-formedness a duplicate `(host (A A) …)` delegation is rejected for (CDZ0201) — report each
        // occurrence after the first, anchored at the redundant one. (A compile-request `--bind` REBIND is a
        // SEPARATE layer — an input artifact merged AFTER load, not a second source `(bind …)` — so it is
        // unaffected: this flags only two source directives for one effect.)
        if !bound_effects.insert(name.clone()) {
            faults.push(
                Reject::coded(
                    Code::Malformed,
                    format!(
                        "the effect `{name}` is bound to a peer more than once — a `(bind …)` route is a \
                         SET (one peer per effect); remove the redundant binding"
                    ),
                )
                .at(name_occ),
            );
        }
    }
    // AN EXPORT WHOSE RESULT IS A NON-REPRESENTABLE CLOSURE — e.g. an entrypoint returning a PARTIAL
    // APPLICATION `(f 1)` for a two-parameter `f`, whose residual parameter type inference never fixed
    // (`Any`) — cannot cross the component boundary. The backend rejects it deep in closure-resource
    // emit (an uncoded decline `cdz check`'s Diagnostics query never runs), so `check` used to accept it
    // while `compile` failed. Detect it here from the export's SOLVED result type (a `Ty::Fn` whose
    // parameter or result has no `abi_val_type`) so BOTH surfaces report it, coded CDZ0201 (ill-formed:
    // the public surface is not boundary-representable), anchored at the export clause. A REPRESENTABLE
    // closure export (`(-> Int64 Int64)` — the C-HOST feature) is fine and NOT flagged.
    // An UNCONSTRAINED parameter/result in an exported closure's type — inference never fixed it, the
    // partial-application / unannotated-closure case. This is the genuinely-unrepresentable signal,
    // NARROWER than "not host-ABI-representable": the closure boundary supports every aliased scalar width
    // (Float32, Int8/16/…), which `abi_val_type` (the host-CALL table) does NOT model, so keying on
    // `abi_val_type` would over-reject a REPRESENTABLE closure export (the C-HOST feature). A
    // concrete-but-unrepresentable component is the backend's own decline, not this well-formedness fault.
    // "Unconstrained" is BOTH `Ty::Any` AND an unresolved unification variable `Ty::Var(_)`: a residual
    // param of a partial application in a FUNCTION body — `(def (g (: n Int64)) (Map.insert (Map.empty) n))`,
    // whose result is `(-> ?7 (Map …))` — surfaces as a `Var(_)`, not `Any`. Matching only `Any` let that
    // case slip through `check` while the backend declined it (leaking the internal `?7` in its message);
    // matching both makes `check` report the same CDZ0201 and closes the check-vs-emit gap. The backend's
    // own `closure_boundary_reject` guards on the same `Any | Var(_)` pair (they must agree).
    fn arrow_has_unconstrained(ty: &crate::ty::Ty) -> bool {
        match ty {
            crate::ty::Ty::Fn(p, r) => {
                matches!(p.as_ref(), crate::ty::Ty::Any | crate::ty::Ty::Var(_))
                    || arrow_has_unconstrained(r)
            }
            _ => false,
        }
    }
    // Collect (body, name, occ, nullary) FIRST (immutable borrow), then read each body's type with `&mut
    // db`. `nullary` (the def takes no parameters) gates whether a type-valued export can be baked — a
    // parameterized one has no constant form.
    let export_results: Vec<(StructId, String, StructId, bool)> = db
        .exports
        .iter()
        .filter_map(|e| {
            let d = e.def?;
            let body = db.defs[d].body?;
            Some((body, e.name.clone(), e.occ, db.defs[d].params.is_empty()))
        })
        .collect();
    for (body, name, occ, nullary) in export_results {
        let ty = crate::infer::type_of(db, body);
        // A CONSTANT QUANTITY export whose DISPLAY-scaled magnitude overflows its inner Int width is a
        // provable trap (CDZ0304), NOT a value to render. A quantity displays at its dimension's REFERENCE
        // unit with the magnitude scaled by the unit's ratio (`5 km` → `5000 m`); when that scaled value
        // exceeds the inner Int (`9223372036854776 km` × 1000 > i64 MAX), the OLD render emitted an
        // out-of-range value form — a wrong-VALUE miscompile. Per the operator overflow ruling (decline when
        // statically detectable, else trap): a CONSTANT (statically-known) scaled magnitude that overflows
        // DECLINES here with CDZ0304, unifying the constant path with the runtime path's trap-on-overflow.
        // Uses the SAME scale (`unit.scale()` of the solved type) the render (`const_value_ast_scaled`) uses,
        // so check and render agree. Only a fixed-width Int inner with a const magnitude is checkable; a
        // BigInt inner (unbounded) or a non-constant magnitude never overflows-at-compile-time here.
        if let crate::ty::Ty::Qty { inner, unit } = &ty
            && let crate::ty::Ty::Int(it) = inner.as_ref()
            && let Core::ConstInt(v) = core_of(db, body)
            && let Some(raw) = v.to_i128()
        {
            // `ground_width`/`ground_signed` resolve a still-DEFERRED inner (an un-annotated `Qty.of 5 km`
            // has a deferred inner width that grounds to Int64) — the SAME grounding the render and the
            // machine-boundary use, so check and render agree on the width the scaled magnitude must fit.
            let w = it.ground_width();
            let (num, den) = unit.scale();
            if let Some(scaled) = raw.checked_mul(num).map(|p| p / den) {
                let scaled_iv = crate::ast::IntValue::from_i128(scaled);
                if !scaled_iv.fits_width(it.ground_signed(), w) {
                    faults.push(
                        Reject::coded(
                            Code::ConstTrap,
                            format!(
                                "quantity `{name}` overflows its inner type when scaled to its reference \
                                 unit — the displayed magnitude does not fit (a compile-time overflow)"
                            ),
                        )
                        .at(occ),
                    );
                    continue;
                }
            } else {
                // The scale multiply itself overflowed i128 (an astronomically large magnitude × scale) —
                // still a provable overflow.
                faults.push(
                    Reject::coded(
                        Code::ConstTrap,
                        format!("quantity `{name}` overflows when scaled to its reference unit"),
                    )
                    .at(occ),
                );
                continue;
            }
        }
        // A TYPE-VALUED export — `(def (main) Int64)` exports a bare type value. A Type is a FIRST-CLASS
        // value that can be returned and inspected at run time (core-semantics.md §Types Are First-Class
        // Values), so a NULLARY export whose type-value reduces to a concrete `Ty` CROSSES the boundary via
        // the constant value-form escape (`constant_value_form` bakes `(: <TypeName> Type)` — the type is
        // fully compile-time-known, its runtime footprint nil). Only a type-value that CANNOT be baked is
        // rejected: a PARAMETERIZED export (its result would depend on a runtime argument, but a type-value
        // never flows from runtime data — §226), or a type that does not reduce to a concrete type (a free
        // variable). Report that ONCE here, coded CDZ0201 (the emit path would otherwise cascade four
        // no-runtime-form declines); `dedup_faults` drops the downstream declines.
        if matches!(ty, crate::ty::Ty::Type) {
            let bakeable =
                nullary && crate::eval::typeval_of(db, body).is_some_and(|t| !t.has_free_var());
            if bakeable {
                has_bakeable_type_export = true;
            }
            if !bakeable {
                // The message embeds `TYPE_EXPORT_MARKER` ("is a TYPE, not a runtime value") so
                // `dedup_faults` drops the downstream no-runtime-form decline family (a built-in-as-value /
                // nullary-lambda / type-value-no-runtime-form cascade the emit path leaks for this same
                // body) — exactly as the nested-type-value branch below does. Without the marker phrasing,
                // a NON-bakeable type-value export (`(: Int64 Type)`, a parameterized/undetermined type)
                // reported the coded reject PLUS three unanchored declines (the very cascade the comment
                // above promises this reject replaces).
                faults.push(
                    Reject::coded(
                        Code::Malformed,
                        format!(
                            "export `{name}` is a TYPE, not a runtime value that can cross the component \
                             boundary — a type-value crosses only from a NULLARY export and only when it \
                             reduces to a concrete type (a type-value never flows from runtime data, so a \
                             parameterized or not-fully-determined type has no boundary form)"
                        ),
                    )
                    .at(occ),
                );
            }
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
                        ty.render_name(&db.name_ctx()),
                        ty.render_name(&db.name_ctx())
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
                        ty.render_name(&db.name_ctx())
                    ),
                )
                .at(occ),
            );
            continue;
        }
        // A NULLARY export whose SOLVED result type is UNDETERMINED — a bare `(None)` (`Option ?`), an
        // `Ok` whose `Err` arm is never built (a free `Var`), OR an empty `(list)`/`(Set.of (list))` whose
        // element GROUNDED to `Ty::Any` rather than staying a `Var` (`(List Any)`). The value escapes to the
        // host with a type no use constrains, so its serialized type header is undetermined — a compile-time
        // reject (type-system.md §Inference Is Principal … a value that escapes with an unconstrained
        // variable MUST be rejected, the fix is an annotation). The EMIT path (`backend::wasm`
        // resource-escape) rejects it, but `cdz check` runs no backend, so it used to ACCEPT the program
        // while `compile` failed — a check≡compile gap. Detect it HERE from the solved result type so BOTH
        // surfaces agree. Gated exactly as the emit path: `nullary` (a parameterized export's free var is a
        // shape issue, not this) AND a single export (`!multi_export`).
        // `has_undetermined_escape_component` catches BOTH the free-`Var` grounding AND the `Ty::Any`
        // grounding (an empty list grounds its element to `Any`, not a `Var`, so bare `has_free_var` missed
        // it — the `(def (main) (list))` gap: `check` accepted, emit gave an uncoded "value-form walker
        // loops to runtime depth" decline). GATED on `crosses_as_resource_escape` — the SAME predicate the
        // emit path uses — which admits only compound/heap shapes, so a bare `Never`/`Any`/`Var` result (a
        // DIVERGING export whose body always traps) is NOT flagged (it does not cross as a heap resource);
        // and every `Any` this sees is a nested element/payload the boundary walker cannot render. The
        // `Ty::Type`/unrepresentable-arrow categories are handled by the `continue`d branches above, so only
        // a genuine undetermined VALUE reaches here.
        //= spec/capabilities/type-system.md#inference-is-principal-type-inference-by-unification
        //# A value that escapes to the host whose type contains a type variable no use constrains MUST be rejected at compile time with the type-determination fault code, rather than crossing the boundary with an invented type, so that a serialized value's type header is always fully determined.
        let multi_export = db.exports.len() > 1;
        if nullary
            && !multi_export
            && ty.has_undetermined_escape_component()
            && crate::backend::wasm::crosses_as_resource_escape(&ty)
        {
            faults.push(
                Reject::coded(
                    Code::TypeMismatch,
                    format!(
                        "the result type `{}` is not fully determined — annotate it \
                         (e.g. `(: <expr> (Option Int64))`) so its value has a defined form",
                        ty.render_name(&db.name_ctx())
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
            // Skip a SYNTHESIZED parameter (not a user-written node) — e.g. the `p$0` placeholder that
            // `strip_const_params` leaves when a `const` parameter is MALFORMED (`(const n Int64)`, two
            // operands, not the well-formed `(const (: n T))`). Its type never resolved BECAUSE the const
            // is malformed; the real, actionable defect is the const-shape CDZ0201 ("a `const` parameter
            // wraps exactly ONE annotated binder", M180) already reported at the user's source. Flagging the
            // synthesized `p$0` "parameter type is ambiguous — annotate it" is a CONSEQUENT second error the
            // author can't act on (there is no user node to annotate, so it renders spanless), and its
            // `(: … Int64)` wrap fix targets a synthesized node. Defer to the const-shape reject. A GENUINE
            // unannotated user param (`(def (mk x) …)`) is a user node → still flagged with its fix.
            if !db.is_user_node(p) {
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
    // A MALFORMED `(Unit.define …)` — wrong arity / non-symbol name / non-integer scale — is silently
    // DROPPED by `scan_unit_defines`, so it registers no family unit and a later use surfaces only as
    // "unknown unit `…`". Reject the malformed FORM here so the real defect is named (the scan-and-drop
    // companion of the malformed-extern / -effect checks); a well-formed one flows to the conflict check.
    crate::infer::check_malformed_unit_defines(db, &mut faults);
    crate::infer::check_unit_defines(db, &mut faults);
    // UNKNOWN UNITS. A quantity literal / `(Unit.of #"name")` naming a unit that is neither a built-in
    // family nor a user `Unit.define` (`5zorks`, `5gram`) fails to reduce and otherwise surfaces only as a
    // generic "no machine representation" decline — name the unknown unit (CDZ0201) with a did-you-mean.
    crate::infer::check_unknown_units(db, &mut faults);
    // MISSPELLED FORM-KEYWORD CASCADE. A misspelled control/binding keyword in head position — `(mtch n
    // (0 1) …)` for `match`, `(le ((x 5)) x)` for `let` — is an unbound-name CDZ0101 whose suggestion is a
    // GRAMMAR keyword. But the whole form is then (mis)read as an APPLICATION, so its arms/bindings fault
    // too: `(mtch …)`'s arm `(0 1)` → "cannot apply Int64", its `_` wildcard → "unbound `_`"; `(le …)`'s
    // body reference `x` → "unbound `x`" (the bindings never took effect). Those are CONSEQUENT on the head
    // typo, not INDEPENDENT problems (`diagnostics.md` §Maximal Independent Set). Once the head is fixed to
    // the keyword, they vanish — so keep the head's did-you-mean CDZ0101 (with its fix) as the ONE primary
    // and drop every OTHER fault anchored strictly INSIDE that form. Keyed on the suggestion being a grammar
    // keyword, so an ordinary misspelled FUNCTION head `(helpr a b)` — whose arguments are genuine
    // sub-expressions with independent faults — is untouched.
    let keyword_typo_forms: Vec<StructId> = faults
        .iter()
        .filter(|r| r.code == Some(Code::Unbound))
        .filter_map(|r| r.at)
        .filter_map(|head| crate::resolve::unbound_head_suggests_grammar_keyword(db, head))
        .collect();
    if !keyword_typo_forms.is_empty() {
        // The HEAD occurrences that carry the primary keyword-typo reject — never suppressed themselves.
        let typo_heads: std::collections::HashSet<u32> = keyword_typo_forms
            .iter()
            .filter_map(|&form| match db.ast.get(form) {
                crate::ast::Struct::List(kids) => kids.first().map(|k| k.0),
                _ => None,
            })
            .collect();
        faults.retain(|r| {
            let Some(at) = r.at else {
                return true; // an unanchored fault is not attributable to a form — keep
            };
            if typo_heads.contains(&at.0) {
                return true; // the primary head reject stays
            }
            // Drop the fault iff its node lies within any keyword-typo form's subtree (walk parents).
            !keyword_typo_forms.iter().any(|&form| {
                let mut cur = at;
                loop {
                    if cur == form {
                        break true;
                    }
                    match db.parent_of(cur) {
                        Some(p) => cur = p,
                        None => break false,
                    }
                }
            })
        });
    }
    // CENTRAL RESUME-POISON FILTER (v-effects design ruling; the tail-resumptive-fold decline is their
    // lane). The `Resolved::Resume` core-lowering poison (`RESUME_NOT_REDUCIBLE_DECLINE`, lower/compute.rs)
    // is emitted whenever ANY fault-collection walk lowers a resume-bearing body STANDALONE via `core_of`
    // (the reached-poison walk, the per-body `type_errors` walks, …), but the REAL emit splices the resume
    // INSIDE the enclosing handle fold and succeeds — so this standalone poison is speculative. Per-walk
    // skips (#6390/#6399 patched only `collect_reached_poisons_at`) are whack-a-mole; drop it ONCE here for
    // ALL walks. SOUND by v-effects's 3-case analysis, losing no real diagnostic: (a) the handle FOLDS →
    // emit succeeds → this poison is spurious; (b) the handle cannot fold → `HANDLER_NOT_REDUCIBLE_DECLINE`
    // is reported AT THE HANDLE (a DIFFERENT message → not filtered → still surfaces); (c) a truly STRAY
    // resume → `STRAY_RESUME` CDZ0201 upstream (different message → not filtered). Keyed on the exact const
    // so it never catches the sibling handler-level decline. Today the dedup self-suppression (a coded
    // decline drops itself at a coded node) ALSO happens to drop this poison, so this filter is
    // behavior-NEUTRAL now; it makes the drop EXPLICIT + robust (independent of that self-suppression) and
    // completes #6390/#6399's per-walk skips, which miss a poison produced deep by `core_of` of a
    // containing node (the nested `…_folds` resume at node 187 leaks through the `type_errors` walks).
    faults.retain(|r| r.message != crate::diag::RESUME_NOT_REDUCIBLE_DECLINE);
    dedup_faults(db, faults, has_bakeable_type_export)
}

/// The BODY occurrence of each VALUE / NULLARY `(def …)` member of the module at `mod_form` — a bare-name
/// `(def v V)` (body `V`) or a nullary `(def (v) V)` (body `V`). A ≥1-param member is EXCLUDED (its body
/// is registered in `db.defs` by `modules::register_fn_def` and checked with the top-level defs); this
/// gathers only the members that registration skips, so `collect_faults` can type-check them too. Reads
/// the raw `(module …)` form (its members are the tail after the name). Does NOT descend into a nested
/// `(module inner …)` member — that inner module is its own `ModuleDecl` and is walked when this function
/// is called for `inner.occ`, so each member is gathered exactly once.
fn module_member_value_bodies(db: &Db, mod_form: StructId) -> Vec<StructId> {
    let Some(tail) = db.ast.as_form(mod_form, "module") else {
        return Vec::new();
    };
    let mut bodies = Vec::new();
    for &member in tail.get(1..).unwrap_or(&[]) {
        let Some(def_tail) = db.ast.as_form(member, "def") else {
            continue;
        };
        let (Some(&sig), Some(&body)) = (def_tail.first(), def_tail.get(1)) else {
            continue;
        };
        // A bare-name value `(def v V)` — the sig is an atom (a name). A list signature `(NAME p…)` is a
        // function; it is a VALUE-body case (checked here) only when NULLARY (`(NAME)`, no params).
        let is_value_or_nullary = match db.ast.get(sig) {
            crate::ast::Struct::Atom(_) => db.ast.as_name(sig).is_some(),
            crate::ast::Struct::List(children) => children.len() == 1, // `(NAME)` — nullary, no params
        };
        if is_value_or_nullary {
            bodies.push(body);
        }
    }
    bodies
}

/// Collapse duplicate faults — the SAME issue reported by more than one collection pass. A fault is
/// keyed by `(code, anchor node)`: the type-check walk and the reached-poison walk both visit an
/// unconditionally-evaluated position, so an unbound name (or any fault) in a REACHABLE spot is found
/// by both and would otherwise be reported twice at the same spot. Two faults with the same code AND
/// the same anchor are the one issue bubbling up along two paths — keep the first (stable order),
/// drop the rest. DISTINCT occurrences (same code, DIFFERENT node — e.g. two separate unbound uses)
/// are NOT duplicates and both survive. An UNANCHORED fault (`at == None`) dedups by code+message, so
/// two different unanchored declines still both show.
fn dedup_faults(db: &Db, faults: Vec<Reject>, has_bakeable_type_export: bool) -> Vec<Reject> {
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
    // A member-op wrong-argument-TYPE reject (`Symbol.of 5` / `List.at xs true` → CDZ0203 "`Module.op`
    // expects an argument of type …") makes the op's LOWERING decline "this operation's operand is not a
    // string (see the type error above)" (`runtime_string_op_decline`). That decline is EXPLICITLY the
    // weaker consequence — its own text says "see the type error above". It used to be dropped by the
    // node-keyed `coded_nodes` dedup (both stamped the app node), but once the CDZ0203 anchors at the
    // ARGUMENT (better locus, PR #399 anchoring), the two land on DIFFERENT nodes and the node-keyed drop
    // misses it. Drop it by FLAG whenever a member-op wrong-arg-type CDZ0203 is present — the decline is
    // the same defect surfacing at lowering. (Keyed on the stable "expects an argument of type" phrase the
    // member-op arm always emits.)
    let has_member_op_arg_type_reject = faults.iter().any(|r| {
        r.code == Some(Code::TypeMismatch) && r.message.contains("expects an argument of type")
    });
    // A `?` on a non-fallible operand (`(try 3.14)`, `(try "hi")`) is reported by `infer` as the coded
    // CDZ0203 `TRY_NON_FALLIBLE_PREFIX` naming the real defect (the operand's type). Its non-sum CONSTANT
    // core also misses the `Resolved::Try` `SumNew` fold arm in `lower`, so the emit path ALSO returns the
    // uncoded `TRY_RUNTIME_OPERAND_DECLINE_PREFIX` "lowers only a constant operand" — the same fault
    // reported more weakly AND misleadingly (it blames constness, but the operand IS constant; its problem
    // is the TYPE). Drop that decline whenever the CDZ0203 is present so an ill-typed `?` is ONE primary
    // `error:`. Gated on the reject existing — a genuinely-RUNTIME fallible operand (no CDZ0203) keeps its
    // honest BRICK-3b decline (it is the only report of the not-yet-lowered runtime `?`).
    let has_try_non_fallible_reject = faults.iter().any(|r| {
        r.code == Some(Code::TypeMismatch)
            && r.message.starts_with(crate::diag::TRY_NON_FALLIBLE_PREFIX)
    });
    // A TYPE VALUE used where a runtime value is wanted — `(+ Color 1)`, a first-class type in an
    // arithmetic/operand position — is reported by `infer` as the coded CDZ0201 kind-boundary ("a Type and
    // an Int64 are different types …"). Lowering that same operand ALSO hits the SPANLESS uncoded "a type
    // value has no runtime form" decline (the type-value has no machine form), which is a CONSEQUENCE of
    // the very fault the CDZ0201 names, not an independent limitation. The node-based `coded_nodes` dedup
    // below can't drop it (the decline is spanless — `.at` is None), so it leaked as a second `error:`
    // line for `+` (but not `<`, which never lowers the operand as a runtime value). Drop the spanless
    // TYPE_VALUE decline whenever a coded reject NAMING `Type` at the kind boundary is present, so a type
    // in a value position is ONE primary `error:`. Keyed on the CDZ0201 message mentioning `Type` + the
    // kind-boundary phrasing, so an unrelated coded reject does not spuriously suppress a genuine
    // unbuilt-type-value decline (e.g. a bare type export with no boundary error keeps its own handling).
    let has_type_kind_boundary_reject = faults.iter().any(|r| {
        r.code.is_some()
            && (
                // The `(+ Color 1)` kind-boundary CDZ0201 ("a Type and an Int64 are different types …").
                (r.message.contains("are different types") && r.message.contains("Type"))
                // A checked VALUE position (`if`/`and`/`not`/guard condition, a member/tuple/record
                // operand) that received the kind `Type` — "… must be Bool, found Type" / "… requires a
                // record, found Type" etc. A type-valued `t` used in such a position (`(if t 1 2)`) is the
                // same type-in-value-position fault as `(+ t 1)`, and its lowering leaks the same
                // no-runtime-form decline family. Keyed on the actual/found type being exactly `Type` (the
                // messages render it as the trailing `found Type`), so an unrelated reject that merely
                // mentions the word does not match.
                || r.message.ends_with("found Type")
            )
    });
    // Likewise: the evaluator's uncoded "applied more arguments than the function accepts" DECLINE is
    // redundant when `infer` proved the over-application (the coded CDZ0203 `applied N arguments to a
    // function of arity M …` reject). Drop the weaker decline so over-application is ONE primary error.
    let has_over_application_reject = faults.iter().any(|r| {
        r.code.is_some()
            && (r.message.contains(crate::diag::OVER_APPLICATION_MARKER)
                || r.message
                    .contains(crate::diag::MEMBER_OVER_APPLICATION_MARKER))
    });
    // A NAMED-MEMBER-OP over-application specifically (`Int64.of`/`List.push` — the `… were given` phrasing,
    // NOT the bare-operator `arguments to a function of arity`). A member-op's over-application ALSO makes
    // the emit path return a coded arity reject (`of takes exactly 1 operand`), which is redundant with the
    // op-naming CDZ0203 — drop it only for a MEMBER over-application. A bare OPERATOR (`+`) instead has a
    // resolve-path CDZ0201 (`+ takes exactly 2 operands`) that IS the primary (kept), so it must NOT match
    // this flag — hence keying on the member marker, not the general over-application flag.
    let has_member_over_application_reject = faults.iter().any(|r| {
        r.code.is_some()
            && r.message
                .contains(crate::diag::MEMBER_OVER_APPLICATION_MARKER)
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
    // Likewise: a handler arm that binds the WRONG NUMBER of parameters (CDZ0201) also makes the handler
    // unfoldable — the same relationship the malformed-handler and resume-result rejects have with the
    // "not yet reducible" decline. Suppress the decline when such a reject is present so a wrong-arity arm
    // reports ONE primary error naming the real defect (the arm's parameter count), not a coded reject
    // shadowed by the leaky fold-decline.
    let has_arm_arity_reject = faults.iter().any(|r| {
        r.code == Some(Code::Malformed) && r.message.contains(crate::diag::HANDLER_ARM_ARITY_MARKER)
    });
    // An ABORTIVE-arm value-type mismatch (CDZ0203 — the abort value disagrees with the op result type or
    // the handle body type) ALSO makes the handler unfoldable, so `lower` emits the uncoded "not yet
    // reducible" decline alongside — the same relationship the malformed-handler / resume-result /
    // arm-arity rejects have. The CDZ0203 is the primary (it names the real type defect); drop the
    // consequent fold-decline so an ill-typed abort reports ONE primary error. Crucial ORDERING fix too:
    // the fold-decline anchors at the handle HEAD (sorts before the CDZ0203 at the abort value), so without
    // this drop `first_error_diag` picks the weaker CDZ0900 and the case grades Todo instead of Pass.
    let has_abort_type_reject = faults.iter().any(|r| {
        r.code == Some(Code::TypeMismatch)
            && r.message
                .contains(crate::diag::ABORT_ARM_TYPE_MISMATCH_MARKER)
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
    // Likewise: an operation declared with NO type (`(op get)`) is rejected CDZ0201 at the op name.
    // Performing it leaks BOTH the internal op-record (`OP_VALUE_RECORD_LEAK`) into a "TYPE, not a runtime
    // value" export fault AND a CDZ0401 no-home (the untyped op has no valid perform semantics) — both
    // CONSEQUENCES of the untyped declaration. Drop them for the declaration-site reject (with its
    // add-a-type fix), the missing-type companion of the non-arrow suppression.
    let has_missing_op_type_reject = faults.iter().any(|r| {
        r.code == Some(Code::Malformed)
            && r.message.starts_with(crate::diag::MISSING_OP_TYPE_PREFIX)
    });
    // Likewise: a DUPLICATE OPERATION in an effect (`(effect E (op get …) (op get …))`) is rejected
    // CDZ0201 "operation `get` is declared more than once in effect `E`" (with a delete fix). PERFORMING
    // that effect projects its SYNTHESIZED RECORD (whose fields ARE the op names), which re-reports the
    // SAME duplicate as a "record names field `get` more than once" — a CONSEQUENT that leaks the internal
    // record representation (the author wrote an effect operation, not a record field). Collect the op
    // names with a dup-op reject; the record-field-dup consequent for one of those names is dropped below,
    // keeping the declaration-site op reject (with its actionable delete fix) as the ONE primary.
    let dup_op_names: std::collections::HashSet<String> = faults
        .iter()
        .filter_map(|r| {
            r.message
                .strip_prefix("operation `")
                .and_then(|rest| rest.split('`').next())
                .filter(|_| r.message.contains("declared more than once in effect"))
                .map(str::to_string)
        })
        .collect();
    // Likewise: a MALFORMED `host` (missing effect-list/body, non-list effects, or too many operands) is
    // rejected CDZ0201 at the form. The malformed host never resolved as a delegation, so its body's
    // perform is seen by the entrypoint no-home walk as un-delegated → a CONSEQUENT CDZ0401 that misdirects
    // (the author DID write a `host`). Drop that CDZ0401 whenever a malformed-host reject is present,
    // keeping the CDZ0201 that says how to fix the host — the `host` analogue of the noncanonical-handle
    // CDZ0401 suppression.
    let has_malformed_host_reject = faults.iter().any(|r| {
        r.code == Some(Code::Malformed) && r.message.starts_with(crate::diag::MALFORMED_HOST_PREFIX)
    });
    // Likewise: a STRAY `resume` (outside any handler arm) that is ALSO malformed (`(resume 5)` — missing
    // next-state, or `(resume v s extra)` — too many) reports TWO CDZ0201s at the SAME resume node: the
    // resolve-path ARITY poison AND the stray-PLACEMENT reject. The placement is the ROOT defect — the
    // resume does not belong here at all — whereas the arity message ("has no next-state") misleads (it
    // reads as if adding an argument would fix it). The same-node dedup keeps only whichever anchored fault
    // comes first (the arity poison, produced earlier), so collect the nodes carrying a stray-placement
    // reject and DROP the arity poison at those nodes below — the misplaced resume then reports its
    // fundamental cause. (A WELL-PLACED but malformed resume inside an arm has no stray reject at its node,
    // so its arity poison is kept — the only report of its real defect.)
    let stray_resume_nodes: std::collections::HashSet<u32> = faults
        .iter()
        .filter(|r| r.message == crate::diag::STRAY_RESUME_MESSAGE)
        .filter_map(|r| r.at.map(|s| s.0))
        .collect();
    // Likewise: a COMPARISON whose operands are a genuine TYPE MISMATCH (`(< 1 "x")`) is rejected by
    // `infer` as a coded "… are different types …" (CDZ0201/CDZ0203 naming the kind boundary). Because one
    // operand is a compound/text the emit path cannot fold to a scalar, `lower` ALSO returns the uncoded
    // "comparison of a compound value needs a heap walk" decline — a CONSEQUENCE of the mismatch, not an
    // independent unbuilt-feature limit. Drop it when a comparison type-mismatch reject is present, keeping
    // the coded reject (which names the real defect) as the ONE primary error.
    let has_different_types_comparison_reject = faults.iter().any(|r| {
        r.code.is_some()
            && r.message
                .contains(crate::diag::DIFFERENT_TYPES_COMPARISON_MARKER)
    });
    // Likewise: a TUPLE accessed by NAME — `(. (tuple 1 2) foo)` — is rejected by `infer` with the precise,
    // actionable "a tuple is accessed by position, not by name `foo` — use a numeric index …" at the def.
    // When that def is CALLED from an exported body, the emit path's reached-poison walk lowers the reduced
    // `(. (tuple 1 2) foo)`, which cannot fold, and returns the BARE "member access requires a record"
    // decline at the CALL SITE — a DIFFERENT node with a DIFFERENT (weaker) message than the infer reject,
    // so neither the same-node dedup nor the reduced-body baseline-diff (which matches on message) collapses
    // it. It is the SAME defect reached again through lowering. Drop the bare decline program-wide when the
    // tuple-by-position reject is present, keeping the precise infer message as the ONE primary. The bare
    // decline (no ", found <T>") is emitted ONLY by lowering — `infer`'s own non-record message always names
    // the type ("… requires a record, found Int64") — so this cannot swallow a genuine standalone primary.
    let has_tuple_by_name_reject = faults
        .iter()
        .any(|r| r.message.contains(crate::diag::TUPLE_BY_NAME_MARKER));
    // A DIRECT member access on a non-record scalar (`(. 5 x)`) produces BOTH `infer`'s rich reject
    // "member access requires a record, found Int64" (names the type) AND the emit path's BARE
    // `MEMBER_NOT_RECORD_DECLINE` "member access requires a record" (no ", found <T>") — at DIFFERENT nodes
    // (the projection vs the enclosing apply), so the node-keyed dedup misses the pair and BOTH leak. The
    // rich one is the primary; drop the bare one whenever the rich "…, found <T>" form is present. Keyed on
    // the ", found" tail the rich message always carries and the bare lowering decline never does — so this
    // can never swallow a genuine standalone bare decline (there is none without the rich twin here). This
    // generalizes the tuple-by-name / value-head drops above to the plain scalar-member-access case.
    let has_member_not_record_found = faults.iter().any(|r| {
        r.message
            .starts_with(crate::diag::MEMBER_NOT_RECORD_DECLINE)
            && r.message.contains(", found ")
    });
    // The exact TUPLE-INDEX analogue: `(. (tuple 1 2) 5)` leaks `infer`'s rich "tuple index 5 is out of
    // range for a 2-element tuple" (names the arity) AND `lower`'s bare "tuple index 5 is out of range"
    // (no arity), at different nodes → both leak. Drop the bare form when the rich "… for a N-element
    // tuple" form is present. Keyed on the shared "tuple index " prefix + the " for a " tail the rich
    // message carries and the bare lowering one never does.
    let has_tuple_index_oob_with_arity = faults
        .iter()
        .any(|r| r.message.starts_with("tuple index ") && r.message.contains(" for a "));
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
        // The invariant core is `<subject> has no <word> \`k\`` (the subject/word vary by operand category
        // — "record … field", "effect `E` … operation", "the `List` module … member", "the type `T` …
        // variant" — but the infer + emit copies of ONE absent-member fault build the IDENTICAL core, and
        // the did-you-mean tail begins ` — `). Key on the whole `has no …`-onward core (past an optional
        // ` — did you mean …?` tail), so every category collapses its twin. Anchored to `has no ` so a
        // message that merely contains those words elsewhere is not mistaken for a member fault.
        msg.find(" has no ")
            .map(|i| msg[i..].split(" — ").next().unwrap_or(&msg[i..]))
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
    // Nodes carrying a genuine coded REJECT (an `error:` that says the program is WRONG) — the set the
    // "drop a weaker decline shadowed by a coded fault at the same node" rule (below) keys on. A coded
    // DECLINE (`Reject::unsupported`, `CDZ0900` — a not-yet-built construct) is EXCLUDED: it carries a
    // `code` but `is_decline()` is true, so counting it here would add its OWN node and then drop it by
    // that very rule (self-suppression) — a lone `CDZ0900` at a node with no real reject would VANISH from
    // `diagnostics()`, silently masking a not-yet-built decline (seq-286: a decline MUST be visible/coded,
    // never dropped). A CDZ0900 decline is the PRIMARY report of its fault, not a weaker consequence of a
    // co-located reject, so it must not be shadowed by the presence of itself. A CDZ0900 decline co-located
    // with a genuine reject is still dropped (the reject's node IS in this set), which is correct — the
    // reject names the real defect.
    let coded_nodes: std::collections::HashSet<u32> = faults
        .iter()
        .filter(|r| r.code.is_some() && !r.is_decline())
        .filter_map(|r| r.at.map(|s| s.0))
        .collect();
    // A BARE "unbound name `X`" reject is SUPERSEDED by an ENRICHED unbound reject for the same name at the
    // SAME node. `enrich_unbound` (infer.rs) rewrites a bare unbound into a teaching message for a recognized
    // form — `(eval …)` ("`eval` executes only a COMPILE-TIME-VISIBLE AST …"), a width slot, a type-var
    // annotation, etc. — but a DIFFERENT collect path can ALSO push the un-enriched bare "unbound name `X`"
    // at the same (code CDZ0101, node). Both collapse to ONE via `seen` below, and the SURVIVOR was just
    // whichever was pushed first — often the BARE copy, so the teaching text was lost (adv-58: a match-
    // scrutinee `(eval q)` gave the misleading bare "unbound name `eval`"). Drop the bare copy when an
    // enriched sibling exists. NARROW by design: keyed to the EXACT bare form `unbound name `X`` (nothing
    // after the closing backtick) so it only ever suppresses the literal un-enriched copy — NOT two distinct
    // same-node rejects that merely share a code (e.g. a nameless-effect-op's "must be named" vs "has no
    // type", which are DIFFERENT defects, neither the bare-unbound form). A bare unbound with NO enriched
    // sibling (an ordinary unbound name) is untouched (no sibling to supersede it).
    let bare_unbound_superseded: std::collections::HashSet<(u32, String)> = {
        // Nodes+names that carry an ENRICHED unbound reject (CDZ0101, mentions the name, but is NOT the bare
        // form). Keyed by (node, name) so only the SAME name's bare copy at that node is dropped.
        let bare_of = |m: &str| -> Option<String> {
            // The exact bare form is "unbound name `X`" with nothing trailing.
            m.strip_prefix("unbound name `")
                .and_then(|rest| rest.strip_suffix('`'))
                .map(str::to_string)
        };
        let enriched: std::collections::HashSet<(u32, String)> = faults
            .iter()
            .filter(|r| r.code == Some(Code::Unbound))
            .filter_map(|r| {
                let node = r.at?.0;
                // An ENRICHED unbound names the variable somewhere but is not the exact bare string.
                // Recover the name from a bare SIBLING at the same node instead (below); here just record
                // that this node has a NON-bare CDZ0101.
                (bare_of(&r.message).is_none()).then_some((node, r.message.clone()))
            })
            .map(|(n, _)| (n, String::new()))
            .collect();
        // For each BARE unbound, mark it superseded iff its node also carries a non-bare CDZ0101.
        faults
            .iter()
            .filter(|r| r.code == Some(Code::Unbound))
            .filter_map(|r| {
                let node = r.at?.0;
                let name = bare_of(&r.message)?;
                enriched
                    .iter()
                    .any(|(n, _)| *n == node)
                    .then_some((node, name))
            })
            .collect()
    };
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
            // Drop a stray resume's ARITY poison ("this resume has no next-state" / "… has no value …" /
            // "… has too many operands") at a node that ALSO carries the stray-PLACEMENT reject — the
            // placement is the root defect (a resume outside a handler arm does not belong here at all),
            // so it is the ONE primary error, not the misleading arity complaint. (`STRAY_RESUME_MESSAGE`
            // itself never starts with "this resume", so it is never dropped here.)
            if r.message.starts_with("this resume has")
                && r.at.is_some_and(|s| stray_resume_nodes.contains(&s.0))
            {
                return false;
            }
            if has_not_a_function_reject
                && r.is_decline()
                && r.message == crate::diag::NOT_APPLYABLE_DECLINE
            {
                return false;
            }
            // Drop the "`?`/`try` lowers only a constant operand" decline when a `?`-non-fallible-operand
            // CDZ0203 is present — the ill-typed operand's non-sum constant core missed the `SumNew` fold arm,
            // so the decline is the same defect at emit, and misleading (it blames constness, not the type).
            // A runtime fallible operand (no CDZ0203) keeps its honest BRICK-3b decline.
            if has_try_non_fallible_reject
                && r.is_decline()
                && r.message
                    .starts_with(crate::diag::TRY_RUNTIME_OPERAND_DECLINE_PREFIX)
            {
                return false;
            }
            // Drop any "(see the type error above)" operand decline when a member-op wrong-arg-type CDZ0203
            // is present — this self-describing marker (String's `runtime_string_op_decline`, and the
            // generic collection/typed `ill_typed_operand_decline`) means the decline EXPLICITLY defers to
            // that type error (and once the CDZ0203 anchors at the argument, they no longer share a node for
            // the node-keyed dedup below). Keyed on the marker so every typed-family site is covered.
            if has_member_op_arg_type_reject
                && r.is_decline()
                && r.message.contains("(see the type error above)")
            {
                return false;
            }
            // Drop the conversion-lowering "a conversion of a non-scalar operand has no meaning" decline
            // when a member-op wrong-arg-type CDZ0203 is present. A CHECKED/wrapping conversion applied to a
            // non-scalar — `(Int8.wrap 3.5)`, `(UInt8.wrap "hi")` — is rejected by `check_application` with
            // "`Int8.wrap` expects an argument of type Int64, but a value of type Float64 was given" (the
            // same "expects an argument of type" family), and the conversion's LOWERING then ALSO declines
            // "a conversion of a non-scalar operand has no meaning" — the identical wrong-operand defect
            // surfacing at emit, anchored at the op node (not the arg), so the node-keyed dedup below misses
            // it. Drop it by flag so a mis-typed conversion is ONE primary `error:` (the coded CDZ0203),
            // not a coded reject shadowed by an emit-path decline (`reference-compiler.md` §Outcomes Are
            // Ordered By Safety).
            if has_member_op_arg_type_reject
                && r.is_decline()
                && r.message == "a conversion of a non-scalar operand has no meaning"
            {
                return false;
            }
            // Drop the no-runtime-form decline FAMILY when a coded Type-kind-boundary reject (`(+ Color 1)`,
            // or a `(: t Type)` param used in a value position → CDZ0201/CDZ0203) already names the real
            // fault — those declines are the same type-in-value-position defect surfacing at lowering, not
            // independent limitations. `(+ t 1)` with a type-valued `t` reduces the operand to the erased
            // type value and hits NOT ONLY the spanless "a type value has no runtime form" decline but also
            // its siblings — `PRIM_AS_VALUE` (the `+` builtin reached as a value once its operand is a
            // type), `NULLARY_LAMBDA_NO_CLOSURE`, and `CLOSURE_PARAM_NO_REPR` (the erased-type-param body
            // lowered as a closure) — all consequences of the coded kind-boundary fault, so a single such
            // reject used to leak 2–4 uncoded `error:` lines. Drop the SAME family the type-export/bakeable
            // arms below already drop (spanless, so the node-based `coded_nodes` dedup misses them). Gated on
            // the kind-boundary reject existing, so a program with a GENUINE independent closure/prim-value
            // decline (no Type-kind-boundary fault) keeps its honest report.
            if has_type_kind_boundary_reject
                && r.is_decline()
                && matches!(
                    r.message.as_str(),
                    crate::diag::TYPE_VALUE_NO_RUNTIME_DECLINE
                        | crate::diag::PRIM_AS_VALUE_DECLINE
                        | crate::diag::NULLARY_LAMBDA_NO_CLOSURE_DECLINE
                        | crate::diag::CLOSURE_PARAM_NO_REPR_DECLINE
                        | crate::diag::CLOSURE_RESULT_NO_REPR_DECLINE
                        | crate::diag::CLOSURE_CAPTURE_NO_REPR_DECLINE
                )
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
            // The emit-path CONVERSION arity reject (`of takes exactly 1 operand`, from `lower`'s
            // `lower_conversion`/`lower_float_of`/…) is a CODED CDZ0201 that fires alongside `infer`'s
            // MEMBER-op over-application CDZ0203 (`Int64.of takes 1 argument, but 2 were given`, with a
            // delete fix). The CDZ0203 names the op and carries the fix, so it is the primary; drop the
            // bare emit-path arity reject. Gated on the MEMBER over-application flag (not the general one):
            // a bare OPERATOR `(+ 1 2 3)` has a resolve-path CDZ0201 `+ takes exactly 2 operands` that IS
            // its primary (kept — its CDZ0203 sibling is dropped by the operator-arity path below), so it
            // must not match here; only a `Module.op` over-application (the `were given` phrasing) does.
            if has_member_over_application_reject
                && r.code == Some(Code::Malformed)
                && r.message.contains(crate::diag::EMIT_OPERAND_ARITY_MARKER)
                && r.message.contains("operand")
            {
                return false;
            }
            // An OVER-APPLIED operation performed inside a handle (`(E.set 1 2)` for a 1-arg op) ALSO makes
            // the handler unfoldable — the same relationship the malformed-handler / resume-result /
            // arm-arity rejects have with the "not yet reducible" decline. The member-op over-application
            // CDZ0203 (with its delete fix) is the primary; drop the consequent fold-decline so a mistyped
            // perform reports ONE actionable error, not a coded reject shadowed by an unbuilt-feature decline.
            if (has_malformed_handler_reject
                || has_resume_result_reject
                || has_arm_arity_reject
                || has_abort_type_reject
                || has_member_over_application_reject)
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
            // A BAKEABLE type-valued export crosses via the constant escape, so the SAME no-runtime-form
            // cascade its body's lowering produces (a bare type-value / built-in-op-as-value / nullary
            // lambda has no runtime form) is not a fault — the escape bakes `(: <TypeName> Type)` from the
            // reduced type. Drop that cascade when a bakeable type export is present (no reject to anchor
            // it — the escape is the answer). Same decline family as the type-export-reject case above.
            if has_bakeable_type_export
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
            // non-arrow OR untyped op declaration; drop it in favor of the declaration-site CDZ0201.
            if (has_non_arrow_op_type_reject || has_missing_op_type_reject)
                && r.message.contains(crate::diag::OP_VALUE_RECORD_LEAK)
            {
                return false;
            }
            // An untyped op declaration (`(op get)`) additionally makes its perform reach the entrypoint
            // with no valid home — a consequent CDZ0401. Drop it for the declaration-site reject (the real,
            // fixable defect is the missing type, not a missing handler).
            if has_missing_op_type_reject && r.code == Some(Code::EffectNoHome) {
                return false;
            }
            // Performing an effect with a DUPLICATE OPERATION projects its synth record, which re-reports the
            // same dup as a "record names field `<op>` more than once" — a consequent that leaks the internal
            // record. Drop it when the declaration-site dup-op reject named that op (the op reject, with its
            // delete fix, is the primary; the author wrote an operation, not a record field).
            if !dup_op_names.is_empty()
                && r.message
                    .strip_prefix("record names field `")
                    .and_then(|rest| rest.split('`').next())
                    .filter(|_| r.message.contains("more than once"))
                    .is_some_and(|field| dup_op_names.contains(field))
            {
                return false;
            }
            // A malformed `host` never resolved as a delegation, so its body's perform looks un-delegated
            // to the no-home walk — a consequent CDZ0401. Drop it for the malformed-host CDZ0201 (the real,
            // fixable defect is the host's shape, not a missing handler).
            if has_malformed_host_reject && r.code == Some(Code::EffectNoHome) {
                return false;
            }
            // A mismatched-type comparison (`(< 1 "x")`) reports the coded "… are different types" reject;
            // the emit path ALSO declines — either "comparison of a compound value needs a heap walk"
            // (equality path) OR the ordering carve-out ("… has no total order, so it cannot be ordered …")
            // (one operand is a compound/text it cannot fold). Drop that misleading decline for the coded
            // reject (recognize BOTH decline forms so a mismatched-type ORDERING compare doesn't double-error).
            if has_different_types_comparison_reject
                && r.is_decline()
                && (r.message.contains(crate::diag::COMPOUND_COMPARISON_DECLINE)
                    || r.message
                        .contains(crate::diag::COMPOUND_ORDERING_NO_TOTAL_ORDER_DECLINE))
            {
                return false;
            }
            // A tuple accessed by NAME whose def is CALLED (`(. (tuple …) foo)` in a called body) leaks the
            // bare "member access requires a record" decline at the call site through the reached-poison
            // walk — the same defect the precise "a tuple is accessed by position" reject already names at
            // the def. Drop the bare decline (an EXACT match — the lowering form has no ", found <T>" tail,
            // which infer's own non-record message always carries, so this never hides a genuine primary).
            if has_tuple_by_name_reject && r.message == crate::diag::MEMBER_NOT_RECORD_DECLINE {
                return false;
            }
            // A direct scalar member access leaks the BARE "member access requires a record" alongside
            // `infer`'s rich "…, found <T>" — same defect, weaker (typeless) message. Drop the bare one when
            // the rich form is present (EXACT match — the bare form has no ", found <T>" tail).
            if has_member_not_record_found && r.message == crate::diag::MEMBER_NOT_RECORD_DECLINE {
                return false;
            }
            // The tuple-index-out-of-range twin: drop `lower`'s bare "tuple index N is out of range" when
            // `infer`'s rich "… for a M-element tuple" is present (bare = no " for a " tail).
            if has_tuple_index_oob_with_arity
                && r.message.starts_with("tuple index ")
                && !r.message.contains(" for a ")
            {
                return false;
            }
            // A `handle` whose HEAD is not an effect leaks a consequent from the desugared `(. head op)`
            // projections — the SHAPE of the consequent depends on what the head IS:
            //   • a VALUE head (`foo`, a scalar) → "member access requires a record" + the fold-decline;
            //   • a TYPE head (`T`, a sum with variants — a record-like value) → `(. T a)` reads the arm-op
            //     name as a MISSING VARIANT ("the type `T` has no variant `a`" / "record has no field").
            // All are consequences of the non-effect head — drop them for the clean head reject (the one
            // primary that names the real problem: the head must be an effect).
            if has_value_head_reject
                && (r.message == crate::diag::HANDLER_NOT_REDUCIBLE_DECLINE
                    || r.message.contains("member access requires a record")
                    || r.message.contains("has no variant")
                    || r.message.starts_with(crate::diag::NO_FIELD_PREFIX))
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
            // Drop a BARE "unbound name `X`" when an ENRICHED unbound sibling exists at the same node (see
            // `bare_unbound_superseded`) — so the teaching text survives, not the bare copy (adv-58).
            if r.code == Some(Code::Unbound)
                && let Some(s) = r.at
                && let Some(rest) = r.message.strip_prefix("unbound name `")
                && let Some(name) = rest.strip_suffix('`')
                && bare_unbound_superseded.contains(&(s.0, name.to_string()))
            {
                return false;
            }
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
    // NB: a `Resolved::Resume` node's `RESUME_NOT_REDUCIBLE_DECLINE` poison (reached here directly OR via a
    // containing node's `core_of`) is NOT special-cased in this walk — it is dropped CENTRALLY at the
    // `collect_faults` chokepoint (the `faults.retain` on that const), which covers this walk AND the
    // type_errors walks uniformly. A resume's real diagnostic is always reported elsewhere (the enclosing
    // handle's fold outcome, or the upstream `STRAY_RESUME` CDZ0201), so nothing is lost.
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
        | Core::StrCmp { lhs, rhs, .. }
        | Core::FloatCompare { lhs, rhs, .. }
        | Core::ValueEq { lhs, rhs }
        | Core::ValueCmp { lhs, rhs, .. }
        | Core::ValueEqShaped { lhs, rhs, .. } => {
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
            for (_, value) in bindings.iter().copied() {
                collect_reached_poisons(db, value, out);
            }
            collect_reached_poisons(db, body, out);
        }
        // A runtime call: its arguments are unconditionally evaluated, so descend into each. The
        // CALLEE's own body faults surface when it is collected (a reachable def is checked on its own
        // — `collect_faults` covers every def body), so we do not re-enter the callee here.
        Core::Call { args, .. } => {
            for &arg in args.iter() {
                collect_reached_poisons(db, arg, out);
            }
        }
        // A host call OR a cross-component call unconditionally evaluates its arguments before crossing
        // the boundary — descend into each (the call itself is a boundary import, not a def whose body
        // could fault).
        Core::HostCall { args, .. } => {
            for &arg in args.iter() {
                collect_reached_poisons(db, arg, out);
            }
        }
        // A sequencing block unconditionally evaluates every statement AND the tail — descend into each.
        Core::Seq { stmts, tail } => {
            for &s in stmts.iter() {
                collect_reached_poisons(db, s, out);
            }
            collect_reached_poisons(db, tail, out);
        }
        // A boundary block / break — the body / break value is reached, so descend for any poison inside.
        Core::Block { body, .. } => collect_reached_poisons(db, body, out),
        Core::Break { value } => collect_reached_poisons(db, value, out),
        // A match: the scrutinee is unconditionally evaluated (descend), but each arm BODY is guarded
        // (only the matching arm runs) — so a provable trap inside an arm is NOT a build failure, the
        // same reachability rule as an `if`'s branches. Do not descend into the arm bodies.
        Core::Match { scrutinee, .. } => {
            collect_reached_poisons(db, scrutinee, out);
        }
        // A tuple's elements are all unconditionally part of the value; a projection's operand is
        // unconditionally evaluated. Descend into each.
        Core::Tuple { elems } | Core::ListNew { elems } | Core::BytesOf { elems } => {
            for &e in elems.iter() {
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
        Core::BinIntRead {
            bytes, off_plus, ..
        }
        | Core::BinRestRead {
            bytes, off_plus, ..
        } => {
            collect_reached_poisons(db, bytes, out);
            if let Some(op) = off_plus {
                collect_reached_poisons(db, op, out);
            }
        }
        Core::BinSizedRead {
            bytes,
            off_plus,
            len,
            ..
        } => {
            collect_reached_poisons(db, bytes, out);
            if let Some(op) = off_plus {
                collect_reached_poisons(db, op, out);
            }
            collect_reached_poisons(db, len, out);
        }
        Core::Proj { operand, .. }
        | Core::ListLen { operand }
        | Core::BytesLen { operand }
        | Core::StrScalarLen { operand } => collect_reached_poisons(db, operand, out),
        // `List.push`/`prepend`/`concat` unconditionally evaluate both operands — descend into each.
        Core::ListPush { list, elem } | Core::ListPrepend { list, elem } => {
            collect_reached_poisons(db, list, out);
            collect_reached_poisons(db, elem, out);
        }
        Core::ListConcat { lhs, rhs } | Core::MapMerge { lhs, rhs } => {
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
        Core::StrScalarAt { operand, index, .. } => {
            collect_reached_poisons(db, operand, out);
            collect_reached_poisons(db, index, out);
        }
        Core::StrSlice {
            string, start, end, ..
        } => {
            collect_reached_poisons(db, string, out);
            collect_reached_poisons(db, start, out);
            collect_reached_poisons(db, end, out);
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
        Core::CharToInt { operand } | Core::IntToCharChecked { operand, .. } => {
            collect_reached_poisons(db, operand, out)
        }
        Core::RationalOfIntWiden { value } => collect_reached_poisons(db, value, out),
        Core::RationalNum { operand } | Core::RationalDen { operand } => {
            collect_reached_poisons(db, operand, out)
        }
        Core::BytesSlice {
            bytes, start, len, ..
        } => {
            collect_reached_poisons(db, bytes, out);
            collect_reached_poisons(db, start, out);
            collect_reached_poisons(db, len, out);
        }
        Core::BytesCompact { operand } => collect_reached_poisons(db, operand, out),
        // `Blake3.of` unconditionally evaluates its bytes operand (the runtime hash reads it) — descend.
        Core::Blake3Of { operand } => collect_reached_poisons(db, operand, out),
        // `Ast.print` (runtime) unconditionally evaluates its Ast operand (the runtime render reads it) — descend.
        // The baked `discs` is a `ConstBytes` (no poison), so only the operand needs walking.
        Core::AstPrint { operand, .. } => collect_reached_poisons(db, operand, out),
        // `Ast.encode` (runtime) unconditionally serializes its Ast operand (the runtime codec reads it) — descend.
        // The baked `discs` is a `ConstBytes` (no poison), so only the operand needs walking.
        Core::AstEncode { operand, .. } => collect_reached_poisons(db, operand, out),
        // `Ast.decode` (runtime) unconditionally evaluates its bytes operand (the runtime op reads it) — descend.
        // The baked `discs` is a `ConstBytes` (no poison), so only the operand needs walking.
        Core::AstDecode { operand, .. } => collect_reached_poisons(db, operand, out),
        // `String.from-bytes` unconditionally evaluates its bytes operand (the runtime op reads it) — descend.
        Core::StrFromBytes { bytes, .. } => collect_reached_poisons(db, bytes, out),
        // `String.to-bytes` unconditionally evaluates its string operand (the runtime flatten reads it) — descend.
        Core::StrToBytes { string } => collect_reached_poisons(db, string, out),
        Core::NfcNormalize { string } => collect_reached_poisons(db, string, out),
        // `Value.encode`/`decode` unconditionally evaluate their single value/bytes operand — descend.
        Core::ValueEncode { value, .. } => collect_reached_poisons(db, value, out),
        Core::ValueDecode { bytes, .. } => collect_reached_poisons(db, bytes, out),
        // A map construction's entry keys AND values are all unconditionally part of the value — descend
        // into each `(key, value)` pair.
        Core::MapNew { entries, .. } => {
            for (k, v) in entries.iter().copied() {
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
            for &e in elems.iter() {
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
        Core::SetToList { set, .. } => collect_reached_poisons(db, set, out),
        Core::MapToList { map, .. } => collect_reached_poisons(db, map, out),
        // A set-algebra op unconditionally evaluates both operand sets — descend into each.
        Core::SetAlgebra { lhs, rhs, .. } => {
            collect_reached_poisons(db, lhs, out);
            collect_reached_poisons(db, rhs, out);
        }
        // A sum construction's payloads are all unconditionally part of the value — descend into each.
        Core::SumNew { payloads, .. } => {
            for &p in payloads.iter() {
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
            for &c in captures.iter() {
                collect_reached_poisons(db, c, out);
            }
        }
        Core::CallClosure { closure, args } => {
            collect_reached_poisons(db, closure, out);
            for &arg in args.iter() {
                collect_reached_poisons(db, arg, out);
            }
        }
        // A parameter, a let-binding reference, or a CAPTURED-variable read is a runtime read — no
        // sub-poison to collect.
        // `trap` is an EXPLICIT runtime divergence (`Core::Trap` → `unreachable`), not a compile-provable
        // trap the build must reject — the honest "this halts here" primitive whose defined outcome IS the
        // runtime trap (like `expect`'s absent branch), so it carries no poison to collect.
        // The abort VALUE is evaluated before the non-local branch, so a poison in it is reached.
        Core::HandleAbort { value, .. } => collect_reached_poisons(db, value, out),
        Core::Captured { .. }
        | Core::LocalRef { .. }
        | Core::Param { .. }
        | Core::ConstInt(_)
        | Core::ConstRational(_, _)
        | Core::ConstBool(_)
        | Core::ConstStr(_)
        | Core::ConstBytes(_)
        | Core::ConstChar(_)
        | Core::ConstFloat(_)
        | Core::ConstFloatNan
        | Core::ConstFloatInf
        | Core::Trap
        | Core::TrapDivZero
        | Core::TrapOverflow
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
    /// A FIXED-arity list pattern `(list p0…p_{n-1})` whose leading elements are ALL bare binders/wildcards
    /// — covers exactly length `n`. A refining leading element (a literal/ctor/nested pattern) is NOT this
    /// (covers only part of that length), so it is not classified. A duplicate exact-length arm, or one
    /// shadowed by an earlier rest arm of lead ≤ `n`, is dead.
    ListExact(usize),
    /// A REST list pattern `(list p0…p_{k-1} .. rest)` whose leading elements are ALL bare binders/wildcards
    /// — covers every length ≥ `k`. A later list arm (exact or rest) whose lengths all fall in `[k, ∞)` is
    /// shadowed. `k == 0` (`(list .. rest)`) covers EVERY list — a whole-list catch-all.
    ListFrom(usize),
    /// A canonical STRUCTURAL KEY of a TUPLE or REFINING-CONSTRUCTOR pattern — a `(tuple …)` or a ctor with
    /// a refining payload sub-pattern (`(tuple true a)`, `(Some (Some x))`), which the coarser covers above
    /// do not classify. The key renders the pattern with every binder/`_` normalized to `_` and each literal
    /// by value, so two arms of the SAME shape (`(tuple true a)` and `(tuple true b)`) share a key. Used ONLY
    /// for EXACT-DUPLICATE detection: an identical key later in the arm list matches exactly the same region,
    /// so it is unreachable. (A BROADER earlier arm subsuming a narrower later one is product-subsumption, not
    /// modeled — this catches only structural repeats, the high-signal case; a non-repeat never false-flags.)
    Shape(String),
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
    // A LIST pattern `(list p… [.. rest])` — covers a length (exact for a fixed pattern, a `≥ k` ray for a
    // rest pattern) ONLY when every LEADING element is a bare binder/wildcard (a refining element — a
    // literal/ctor/nested pattern — covers only part of that length, so it is not classified: conservative,
    // never a false redundancy). This is the list analogue of the full-variant `Variant` cover. `is_list_
    // pattern` matches both the `(list …)` alias and the reserved `"list"` symbol head.
    if let Some(es) = db
        .ast
        .compound_form_of(pat, CompoundCtor::List)
        .map(<[_]>::to_vec)
    {
        let marker = db.ast.rest_marker(&es);
        let lead = marker.map(|(i, _, _)| i).unwrap_or(es.len());
        // A malformed rest (a `..` not second-to-last, or >1 binder after it) is not decidably a cover.
        if let Some((_, _, trailing_start)) = marker
            && trailing_start != es.len()
        {
            return None;
        }
        // Every LEADING element must be a bare binder / `_` (no refining sub-pattern).
        for &e in &es[..lead] {
            db.ast.as_name(e)?;
        }
        return Some(if marker.is_some() {
            ArmCover::ListFrom(lead) // `(list p… .. rest)` covers every length ≥ lead
        } else {
            ArmCover::ListExact(lead) // `(list p…)` covers exactly length lead
        });
    }
    // A TUPLE pattern `(tuple p0 p1 …)`. An ALL-IRREFUTABLE tuple (`(tuple x y)` / `(tuple _ (tuple a b))`)
    // matches EVERY value of its tuple type — it is a whole-type CatchAll, so it shadows every later arm
    // (the product-subsumption whole-tuple case). Otherwise it is a STRUCTURAL-KEY candidate: two identical
    // tuple arms (`(tuple true a)`/`(tuple true b)`) are exact duplicates. `is_irrefutable_cover` decides.
    if db
        .ast
        .compound_form_of(pat, crate::ast::CompoundCtor::Tuple)
        .is_some()
    {
        if is_irrefutable_cover(db, pat) {
            return Some(ArmCover::CatchAll);
        }
        return pattern_shape_key(db, pat).map(ArmCover::Shape);
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
        // Every payload sub-pattern an IRREFUTABLE COVER (a bare binder/wildcard, OR an all-wildcard tuple
        // `(Some (tuple _ _))`) → a FULL-variant cover: the arm matches every value of that variant, so it
        // shadows any later same-variant arm. A REFINING sub-pattern (a nested literal/ctor, `(Some (Some
        // x))`) covers only PART of the variant — not a full cover, but a structural-key candidate (an
        // identical refining arm later is an exact duplicate).
        if children[payload_start.min(children.len())..]
            .iter()
            .all(|&sub| is_irrefutable_cover(db, sub))
        {
            return Some(ArmCover::Variant(disc));
        }
        return pattern_shape_key(db, pat).map(ArmCover::Shape);
    }
    None
}

/// Whether `pat` matches EVERY value of its type — a whole-type cover, so an arm with this pattern is a
/// CatchAll that shadows every later arm. TRUE for a bare binder / `_` (matches anything), or a TUPLE whose
/// every element is itself an irrefutable cover (`(tuple x y)`, `(tuple _ (tuple a b))` — a tuple has one
/// shape, so covering every element covers the whole tuple). FALSE for a GUARDED pattern (conditional), a
/// literal (one value), a CONSTRUCTOR (one variant of a multi-variant sum — `(Some _)` misses `None`; a
/// bare nullary-variant NAME like `None` likewise covers one variant), or any map/list/record pattern. A
/// bare NAME that is a nullary-variant ctor is NOT a cover — checked via `variant_disc_of`.
fn is_irrefutable_cover(db: &mut Db, pat: StructId) -> bool {
    // A guard makes the arm conditional — never an unconditional cover.
    if db.ast.as_form(pat, "guard").is_some() {
        return false;
    }
    // A bare binder / `_` covers everything — UNLESS it is a nullary-variant name (`None`), which covers
    // only its own variant.
    if db.ast.as_name(pat).is_some() {
        return crate::eval::variant_disc_of(db, pat).is_none();
    }
    // A tuple covers its whole type iff every element is itself a whole-type cover.
    if let Some(elems) = db
        .ast
        .compound_form_of(pat, CompoundCtor::Tuple)
        .map(<[_]>::to_vec)
    {
        return elems.iter().all(|&e| is_irrefutable_cover(db, e));
    }
    // A literal / constructor / map / list / record pattern is refutable — not a whole-type cover.
    false
}

/// A CANONICAL STRUCTURAL KEY of a pattern, for EXACT-DUPLICATE redundant-arm detection — two patterns
/// share a key exactly when they match the same region (a binder and `_` both normalize to `_`, a literal
/// keys by value, a ctor by its discriminant + sub-keys, a tuple by its element sub-keys). `None` for a
/// pattern whose region is NOT decidable from the shape alone — a GUARDED sub-pattern (conditional), or a
/// shape this function does not model (a map/record/list sub-pattern) — so an unclassifiable pattern never
/// yields a spurious duplicate. Recurses to any depth (`(Some (Some 0))`).
fn pattern_shape_key(db: &mut Db, pat: StructId) -> Option<String> {
    // A guarded sub-pattern is conditional — its region is not decidable, so no exact-duplicate claim.
    if db.ast.as_form(pat, "guard").is_some() {
        return None;
    }
    // A bare binder / `_` — matches everything at this position; normalize both to `_`.
    if db.ast.as_name(pat).is_some() {
        // A bare NULLARY-VARIANT name (`None`) keys by its discriminant, not as a wildcard.
        if let Some(disc) = crate::eval::variant_disc_of(db, pat) {
            return Some(format!("v{disc}"));
        }
        return Some("_".to_string());
    }
    // A scalar literal keys by value (the same value-unique encoding `arm_cover` uses).
    match crate::resolve::resolved_of(db, pat) {
        crate::resolved::Resolved::Int(v) => {
            return Some(format!("i{}:{:x?}", v.negative, v.magnitude));
        }
        crate::resolved::Resolved::Bool(b) => return Some(format!("b{b}")),
        crate::resolved::Resolved::Str(s) => return Some(format!("s{s}")),
        _ => {}
    }
    // A TUPLE `(tuple p0 …)` — key = `(t <sub-key>…)`, recursing each element.
    if let Some(elems) = db
        .ast
        .compound_form_of(pat, CompoundCtor::Tuple)
        .map(<[_]>::to_vec)
    {
        let mut key = String::from("(t");
        for e in elems {
            key.push(' ');
            key.push_str(&pattern_shape_key(db, e)?);
        }
        key.push(')');
        return Some(key);
    }
    // A CONSTRUCTOR `(C.V p…)` / bare-member `(. Sum V)` — key = `(c<disc> <payload-sub-key>…)`.
    if let crate::ast::Struct::List(children) = db.ast.get(pat) {
        let children = children.to_vec();
        let (head, payload_start) = match children.first().copied() {
            Some(first) if db.ast.as_name(first) == Some(".") => (pat, children.len()),
            Some(first) => (first, 1),
            None => return None,
        };
        let disc = crate::eval::variant_disc_of(db, head)?;
        let mut key = format!("(c{disc}");
        for &sub in &children[payload_start.min(children.len())..] {
            key.push(' ');
            key.push_str(&pattern_shape_key(db, sub)?);
        }
        key.push(')');
        return Some(key);
    }
    // A map/record/list sub-pattern — not modeled here (conservative → no duplicate claim).
    None
}

/// The TOP-LEVEL constructor discriminant of a ctor-headed pattern — a bare nullary-variant name (`None`),
/// an applied ctor `(Some …)` / `(C.V …)`, or a bare member `(. Sum V)` used whole — else `None` (a tuple,
/// list, literal, or plain binder). Used by [`collect_redundant_arm_warnings`] to test whether a REFINING
/// ctor arm (`(Some (Some x))`, an `ArmCover::Shape`) is shadowed by an EARLIER FULL-variant cover of the
/// same variant (`(Some _)`): the full cover matched every value of that variant, so the later refinement
/// is unreachable. Mirrors `arm_cover`'s ctor-head extraction, but yields only the discriminant.
fn ctor_pattern_variant(db: &mut Db, pat: StructId) -> Option<u32> {
    // A bare NAME that is a nullary variant (`None`) — its discriminant; a plain binder yields `None`.
    if db.ast.as_name(pat).is_some() {
        return crate::eval::variant_disc_of(db, pat);
    }
    // A constructor-headed list `(Some x)` / `((. Sum V) x)` / a whole bare member `(. Sum V)`.
    if let crate::ast::Struct::List(children) = db.ast.get(pat) {
        let head = match children.first().copied() {
            // A whole bare member `(. Sum V)` is the pattern itself; else the first child is the ctor head.
            Some(first) if db.ast.as_name(first) == Some(".") => pat,
            Some(first) => first,
            None => return None,
        };
        return crate::eval::variant_disc_of(db, head);
    }
    None
}

/// The number of DISTINCT full-variant / bool covers that EXHAUST the scrutinee's type — `Some(n)` for a
/// FINITE type (a `Ty::Sum` with `n` variants, or `Bool` with 2), `None` for an OPEN type (Int/String/…,
/// which no finite literal set exhausts). Used by [`collect_redundant_arm_warnings`] to flag a catch-all /
/// arm that is unreachable because the SPECIFIC arms before it already cover every value of the type — the
/// dual of the exhaustiveness check (which faults a match MISSING coverage; this warns on coverage the
/// arms make REDUNDANT). Reads the sum's declaration by its `decl` occurrence (the same source of truth
/// `lower`'s exhaustiveness uses), so the count agrees exactly with what a full cover needs.
fn finite_cover_size(db: &mut Db, scrutinee: StructId) -> Option<usize> {
    // Read the variant count off a SUM declaration, whether the scrutinee's type is a boxed `Ty::Sum` or an
    // ERASED single-variant newtype `Ty::Nominal { decl }` (whose `decl` is still that sum — a newtype has
    // exactly one variant, so its sole constructor arm saturates it). A nominal wrapping a NON-sum (a
    // newtype over a scalar) has no variant set → not finite here.
    match crate::infer::type_of(db, scrutinee) {
        crate::ty::Ty::Bool => Some(2),
        crate::ty::Ty::Sum { decl, .. } | crate::ty::Ty::Nominal { decl, .. } => {
            db.type_decl_by_occ(decl).and_then(|d| {
                // An OPEN sum (`(type T … .. r)`) has NO finite cover size — the row variable stands for
                // variants not named, so its value set is not closed and a `_` arm over it is NEVER
                // redundant (it is the only cover for the open tail, `type-system.md §206`). Returning
                // `None` here keeps the redundant-arm pass from ever flagging that `_` (an open sum's `_`
                // never closes finite coverage). A CLOSED sum keeps its variant count as the finite size.
                if d.open_tail.is_some() {
                    return None;
                }
                (!d.variants.is_empty()).then_some(d.variants.len())
            })
        }
        _ => None,
    }
}

/// Collect REDUNDANT-ARM warnings (CDZ0213) across every `match` in every def body — an arm an EARLIER
/// arm (or set of arms) already fully covers, so first-match-wins makes it dead. Walks all user nodes
/// (like the unused-binding pass) rather than only reached bodies, so a redundant arm in an uncalled
/// helper is surfaced too. For each match, scan arms left to right keeping the set of already-covered keys
/// plus whether coverage is CLOSED — a catch-all has appeared, OR (for a FINITE scrutinee type — a sum or
/// Bool) the distinct full-variant/bool covers already SATURATE the type. An arm whose cover is subsumed
/// (a repeat of a covered literal/variant) OR any arm after coverage closed warns. Conservative: an
/// unclassifiable arm (`arm_cover` → `None`) neither shadows, saturates, nor is flagged, so a
/// guarded/refining/tuple arm never yields a false positive.
fn collect_redundant_arm_warnings(db: &mut Db) -> Vec<Diagnostic> {
    use crate::resolved::Resolved;
    let node_count = db.ast.structure.len();
    let mut out = Vec::new();
    for i in 0..node_count {
        let id = StructId(i as u32);
        if !db.is_user_node(id) {
            continue;
        }
        let Resolved::Match { scrutinee, arms } = crate::resolve::resolved_of(db, id) else {
            continue;
        };
        // A match whose lowering POISONS — a malformed/typo'd arm pattern (a bare name that is a variant
        // TYPO, `(match c (Rd 1) …)` on `(type Color Red Green)` → CDZ0201 "did you mean `Red`?"), a
        // wrong-arity ctor, a non-linear binder — is being REJECTED; a later arm reading "unreachable"
        // because the typo'd arm was (mis)read as a catch-all binder is CONSEQUENT noise, not an
        // INDEPENDENT problem. Skip the whole match's redundant-arm pass, deferring to the poison the SAME
        // lowering produces (the CDZ0201 the two can never disagree with) — the redundant-arm twin of the
        // guard `collect_unused`'s match-binder pass already applies.
        if matches!(core_of(db, id), Core::Poison(_)) {
            continue;
        }
        // The scrutinee's finite cover size — `Some(n)` iff the specific arms CAN exhaust the type (a sum
        // of `n` variants, or Bool = 2). `None` for an open type, where only a catch-all closes coverage.
        let cover_size = finite_cover_size(db, scrutinee);
        // Coverage is CLOSED once a catch-all is seen OR the distinct full-variant/bool covers reach the
        // finite type's size — either way every subsequent arm is unreachable.
        let mut coverage_closed = false;
        // A HASH SET of already-covered literal/variant keys — an O(1) membership probe per arm. A `Vec`
        // + `contains` was O(covered) per arm → O(arms²) for a match over an N-variant sum (each of N
        // distinct-variant arms scanned the growing covered list).
        let mut covered: std::collections::HashSet<ArmCover> = std::collections::HashSet::new();
        // The subset of `covered` that are FULL-TYPE-value covers (`Variant`/`Lit`) — the only ones that
        // count toward a FINITE type's saturation. A `Shape`/list cover is a PARTIAL/region cover and must
        // not close variant coverage (see the insert site below).
        let mut finite_covers: std::collections::HashSet<ArmCover> =
            std::collections::HashSet::new();
        // The SMALLEST rest-arm lead seen so far — a prior `(list p… .. rest)` of lead `k` covers every
        // length ≥ k, so it SHADOWS any later list arm whose lengths all fall in `[k, ∞)` (a later exact
        // `(list …n…)` with n ≥ k, or a later rest of lead ≥ k). `None` until a rest arm appears. Tracked
        // alongside `covered` (which only catches EXACT-key duplicates) to add list-length subsumption.
        let mut min_list_from: Option<usize> = None;
        // The discriminants of variants an earlier arm covered in FULL (an `ArmCover::Variant(disc)` — a
        // ctor whose payload is all-irrefutable, `(Some _)`). A later REFINING arm of that same variant (a
        // `Shape` whose top ctor is `disc`, `(Some (Some x))`) is unreachable — the full cover already
        // matched every value of the variant. This is variant-refinement subsumption (a BROADER earlier arm
        // shadowing a narrower same-variant later one), beyond the exact-duplicate `Shape` check.
        let mut full_variants: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for (pat, _) in &arms {
            let cover = arm_cover(db, *pat);
            let redundant = match &cover {
                // Any arm after coverage closed (a catch-all, or the type saturated) is unreachable.
                _ if coverage_closed => true,
                // A later list arm whose every length is ≥ an earlier rest arm's lead is shadowed by it.
                Some(ArmCover::ListExact(n)) if min_list_from.is_some_and(|k| k <= *n) => true,
                Some(ArmCover::ListFrom(j)) if min_list_from.is_some_and(|k| k <= *j) => true,
                // A REFINING ctor arm (`ArmCover::Shape` from a ctor payload, `(Some (Some x))`) whose top
                // variant an EARLIER arm already covered in full (`(Some _)`) is shadowed by that full cover.
                Some(ArmCover::Shape(_))
                    if ctor_pattern_variant(db, *pat)
                        .is_some_and(|d| full_variants.contains(&d)) =>
                {
                    true
                }
                // A repeat of an already-covered literal / full-variant / exact-length / rest cover.
                Some(c) => covered.contains(c),
                // Unclassifiable — not provably redundant.
                None => false,
            };
            if redundant && db.is_user_node(*pat) {
                let mut diag = crate::abi_bridge::diagnostic_warning(
                    crate::diag::Code::RedundantArm,
                    "this match arm is unreachable — the earlier arms already cover every value it \
                     would match (a duplicate, a pattern shadowed by an earlier catch-all, or a \
                     catch-all after the specific arms already cover the whole type)",
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
                    diag = diag.with_fix(crate::abi_bridge::diagnostic_fix_from_fix(
                        &crate::diag::Fix::delete_heuristic(arm, "remove this unreachable arm"),
                    ));
                }
                out.push(diag);
            }
            match cover {
                Some(ArmCover::CatchAll) => coverage_closed = true,
                Some(c) => {
                    // A REST list arm `(list p… .. rest)` of lead `k` covers `[k, ∞)` — record the smallest
                    // such lead so it shadows later list arms whose lengths lie in that ray (the list
                    // analogue of a variant/catch-all closing coverage). A lead-0 `(list .. rest)` covers
                    // EVERY length, so it shadows all later list arms (min becomes 0). Still `.insert` it so
                    // an exact-key DUPLICATE rest arm is also caught by `covered.contains`.
                    if let ArmCover::ListFrom(k) = c {
                        min_list_from = Some(min_list_from.map_or(k, |m| m.min(k)));
                    }
                    // Count only FULL-TYPE-value covers toward finite-type saturation: a `Variant` (a whole
                    // sum variant) or a `Lit` (a whole Bool value — `bool_exhaustive`). A `Shape` (a REFINING
                    // ctor / tuple — covers only PART of a variant) or a `ListExact`/`ListFrom` (a list-length
                    // region, not a finite type) must NOT close variant coverage: e.g. `(Some (Some x)) +
                    // (Some (None)) + (None)` over `(Option (Option Int))` has two `Shape`s + one `Variant`,
                    // and counting the Shapes would wrongly saturate the 2-variant type and flag `(None)`.
                    if matches!(c, ArmCover::Variant(_) | ArmCover::Lit(_)) {
                        finite_covers.insert(c.clone());
                    }
                    // Record a FULL-variant cover's discriminant so a later REFINING arm of the SAME variant
                    // (a `Shape` — `(Some (Some x))` after `(Some _)`) is flagged as shadowed above.
                    if let ArmCover::Variant(disc) = c {
                        full_variants.insert(disc);
                    }
                    covered.insert(c);
                    // If the distinct FULL covers now saturate a FINITE type, coverage is closed: any later
                    // arm (including a catch-all) is unreachable.
                    if cover_size.is_some_and(|n| finite_covers.len() >= n) {
                        coverage_closed = true;
                    }
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
            out.push(crate::abi_bridge::diagnostic_warning(
                Code::DeadTrap,
                msg,
                at,
            ));
        } else {
            walk_for_dead_traps(db, child, out, seen);
        }
    };
    match crate::resolve::resolved_of(db, id) {
        // Value-discarding positions: each constituent whose value may be dropped.
        Resolved::Tuple { elems } | Resolved::List { elems } | Resolved::Set { elems } => {
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
            for (_, init) in bindings.iter() {
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
        Resolved::Annot { expr, .. } | Resolved::ConstBlock { expr } => {
            walk_for_dead_traps(db, expr, out, seen)
        }
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
        // `(try e)` unconditionally evaluates its operand (the fallible value is always computed before
        // the disc check that may short-circuit), so descend into it exactly like `not`.
        Resolved::Try { operand } => walk_for_dead_traps(db, operand, out, seen),
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
        | Resolved::Rational { .. }
        | Resolved::Unit
        | Resolved::Prim(_)
        | Resolved::Param { .. }
        | Resolved::TypeVal(_)
        | Resolved::Lambda { .. }
        | Resolved::SumPayload { .. }
        | Resolved::BinField { .. }
        | Resolved::MapField { .. }
        | Resolved::RecordField { .. }
        | Resolved::RecordRest { .. }
        | Resolved::SetRest { .. }
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

/// Collect POTENTIALLY-REACHABLE-TRAP warnings (CDZ0309) — the conditional-branch companion of the
/// dropped-value dead-trap warning. Per the operator ruling (cn02), a compile-provable trap (a divide-by-zero
/// / overflow / out-of-bounds the fold discovered) in an `if` branch or `match` arm guarded by a RUNTIME
/// condition is NOT a compile error: it demotes to a runtime trap that fires only when the branch is taken
/// (`lower::demote_conditional_trap`). But the author did not write it — the fold SYNTHESIZED it — so warn
/// that the operation could trap along a reachable path (a likely defect). Walks every def body; at each
/// RUNTIME `if`/`match` (its lowered core is a `Core::If`/`Core::Match`/`Core::MatchSum`, so a branch is
/// genuinely reachable — a const-condition `if` folds to one arm and DROPS the other), a branch/arm whose
/// core is a `ConstTrap` poison warns. An explicit user `(trap …)` lowers to a plain `Core::Trap` (not a
/// provable-trap poison), so it never warns — exactly the const-fold-origin discrimination the ruling asks
/// for. Anchored at the offending operation. Fires at the SAME positions `demote_conditional_trap` demotes.
fn collect_reachable_const_trap_warnings(db: &mut Db) -> Vec<Diagnostic> {
    let bodies: Vec<StructId> = db.defs.iter().filter_map(|d| d.body).collect();
    let mut warnings = Vec::new();
    let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
    for body in bodies {
        walk_for_reachable_const_traps(db, body, &mut warnings, &mut seen);
    }
    warnings
}

/// Warn (CDZ0309) if `branch` — an occurrence in a CONDITIONALLY-reached position — folds to a `ConstTrap`
/// (a const-fold-origin provable trap; NOT an explicit `Core::Trap`, which never enters here).
fn warn_reachable_const_trap(db: &mut Db, branch: StructId, out: &mut Vec<Diagnostic>) {
    if let Core::Poison(r) = core_of(db, branch)
        && r.code == Some(Code::ConstTrap)
    {
        let at = dropped_trap_anchor(db, branch).filter(|&n| db.is_user_node(n));
        // Name the SPECIFIC trap kind (operator 2026-08-27: "say what kind of trap it is" — the same
        // tag-the-trap intent behind `Core::TrapDivZero`/`TrapOverflow`). The `ConstTrap` message leads with
        // the cause ("divide by zero — …", "the result overflows Int64 — …", "shift count N is out of range
        // 0..64 — …"); take the phrase before the " — " repair hint as the kind label so the author is told
        // WHICH trap the demoted operation will raise. Keeps the "potentially reachable trap" wording the
        // CDZ0309 corpus grade matches on (a `contains` check).
        let kind = r.message.split(" — ").next().unwrap_or(&r.message).trim();
        out.push(crate::abi_bridge::diagnostic_warning(
            Code::ReachableTrap,
            format!(
                "this operation resulted in a potentially reachable trap ({kind}): it always traps when this \
                 branch is taken, and whether the branch is taken depends on a runtime value — guard the \
                 operand or remove the operation"
            ),
            at,
        ));
    }
}

/// Walk the RESOLVED tree from `id`, warning at each RUNTIME `if` branch / `match` arm whose value folds to a
/// `ConstTrap` (the demoted-to-runtime const-fold trap). Descends EVERY sub-position — including the guarded
/// branches themselves — so a demoted trap nested at any depth is found (unlike `walk_for_dead_traps`, which
/// stops at control-flow boundaries because it hunts DROPPED values, not reachable ones). `seen` dedups a
/// shared occurrence.
fn walk_for_reachable_const_traps(
    db: &mut Db,
    id: StructId,
    out: &mut Vec<Diagnostic>,
    seen: &mut std::collections::HashSet<u32>,
) {
    use crate::resolved::Resolved;
    if !seen.insert(id.0) {
        return;
    }
    match crate::resolve::resolved_of(db, id) {
        Resolved::If { cond, then_, else_ } => {
            // Only a RUNTIME `if` has genuinely-reachable branches — a const-condition `if` folds to the taken
            // branch (the untaken one, ConstTrap or not, is DROPPED, unreachable). Its lowered core being
            // `Core::If` witnesses the runtime condition (and the demote that ran there).
            if matches!(core_of(db, id), Core::If { .. }) {
                warn_reachable_const_trap(db, then_, out);
                warn_reachable_const_trap(db, else_, out);
            }
            walk_for_reachable_const_traps(db, cond, out, seen);
            walk_for_reachable_const_traps(db, then_, out, seen);
            walk_for_reachable_const_traps(db, else_, out, seen);
        }
        Resolved::Match { scrutinee, arms } => {
            let runtime = matches!(core_of(db, id), Core::Match { .. } | Core::MatchSum { .. });
            for (_, body) in arms.iter() {
                if runtime {
                    warn_reachable_const_trap(db, *body, out);
                }
                walk_for_reachable_const_traps(db, *body, out, seen);
            }
            walk_for_reachable_const_traps(db, scrutinee, out, seen);
        }
        Resolved::Tuple { elems } | Resolved::List { elems } | Resolved::Set { elems } => {
            for e in elems.iter() {
                walk_for_reachable_const_traps(db, *e, out, seen);
            }
        }
        Resolved::Map { entries } => {
            for &(k, v) in entries.iter() {
                walk_for_reachable_const_traps(db, k, out, seen);
                walk_for_reachable_const_traps(db, v, out, seen);
            }
        }
        Resolved::Record { fields } => {
            for &v in fields.values() {
                walk_for_reachable_const_traps(db, v, out, seen);
            }
        }
        Resolved::Bin { segs } => {
            for s in segs.iter() {
                walk_for_reachable_const_traps(db, s.slot, out, seen);
                match &s.kind {
                    crate::resolved::SegKind::Bytes { size: Some(n) } => {
                        walk_for_reachable_const_traps(db, *n, out, seen)
                    }
                    crate::resolved::SegKind::Utf8 { size } => {
                        walk_for_reachable_const_traps(db, *size, out, seen)
                    }
                    _ => {}
                }
            }
        }
        Resolved::Let { bindings, body } => {
            for &(_, init) in bindings.iter() {
                walk_for_reachable_const_traps(db, init, out, seen);
            }
            walk_for_reachable_const_traps(db, body, out, seen);
        }
        Resolved::Apply { head, args } => {
            walk_for_reachable_const_traps(db, head, out, seen);
            for a in args.iter() {
                walk_for_reachable_const_traps(db, *a, out, seen);
            }
        }
        Resolved::Proj { operand, .. } | Resolved::Member { operand, .. } => {
            walk_for_reachable_const_traps(db, operand, out, seen);
        }
        Resolved::Annot { expr, .. } | Resolved::ConstBlock { expr } => {
            walk_for_reachable_const_traps(db, expr, out, seen);
        }
        Resolved::Ref { value } => walk_for_reachable_const_traps(db, value, out, seen),
        // A short-circuit connective's RIGHT operand is guarded, but `demote_conditional_trap` does NOT demote
        // an `and`/`or` rhs (only `if`/`match`), so a ConstTrap there is still an ERROR, not a demoted runtime
        // trap — do not warn it here (descend to find nested if/match). The LHS is unconditional.
        Resolved::And { lhs, rhs, .. } => {
            walk_for_reachable_const_traps(db, lhs, out, seen);
            walk_for_reachable_const_traps(db, rhs, out, seen);
        }
        Resolved::Not { operand } | Resolved::Try { operand } => {
            walk_for_reachable_const_traps(db, operand, out, seen);
        }
        Resolved::Handle { .. } | Resolved::Host { .. } | Resolved::Resume { .. } => {}
        Resolved::Int(_)
        | Resolved::Bool(_)
        | Resolved::Str(_)
        | Resolved::SymbolConst(_)
        | Resolved::Bytes(_)
        | Resolved::Char(_)
        | Resolved::Float(_)
        | Resolved::Rational { .. }
        | Resolved::Unit
        | Resolved::Prim(_)
        | Resolved::Param { .. }
        | Resolved::TypeVal(_)
        | Resolved::Lambda { .. }
        | Resolved::SumPayload { .. }
        | Resolved::BinField { .. }
        | Resolved::MapField { .. }
        | Resolved::RecordField { .. }
        | Resolved::RecordRest { .. }
        | Resolved::SetRest { .. }
        | Resolved::Poison(_) => {}
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

    // MATCH-ARM PATTERN BINDERS. A variant/tuple pattern binder — `x` in `(match o ((Some x) …) …)` —
    // that its arm body never references is unused, exactly like an unused `let` binding or parameter, and
    // should be `_`-prefixed. It is NOT tracked by the `used`-occ set: a reference to a match binder
    // resolves to a `SumPayload` (reading the scrutinee) or a scalar-match `Ref` (to the scrutinee), never
    // to the binder's OWN occurrence. So — like the parameter path — use a scope-correct NAME check: a
    // binder is used iff some name occurrence in its arm BODY resolves to a match binder of that name
    // (`resolves_to_match_binder`). Collected per `(match …)` arm; a `_`-prefixed binder is skipped (the
    // shared loop filters `_` too, but a variant head / literal in the pattern is not a binder anyway).
    for i in 0..node_count {
        let id = StructId(i as u32);
        if !db.is_user_node(id) {
            continue;
        }
        let Resolved::Match { arms, .. } = crate::resolve::resolved_of(db, id) else {
            continue;
        };
        // A match whose lowering POISONS — a malformed pattern (`(tuple a b c)` against a 2-tuple →
        // CDZ0201), a non-linear binder, an unbound scrutinee — is being REJECTED; its arm binders never
        // bind, so "unused binding" on them is CONSEQUENT noise, not an INDEPENDENT problem. Skip the
        // whole match's binder pass, deferring to the poison the SAME lowering produces (the authority the
        // CDZ0201 comes from — the two can never disagree). This closes a check≡compile discrepancy: the
        // `compile` path bails at the first fault set before any warning is collected, so it shows ONLY the
        // CDZ0201; the `diagnostics()`/`cdz check` path collects faults AND warnings, and without this
        // guard it appended spurious CDZ0306s for the binders inside the rejected pattern.
        //= spec/capabilities/diagnostics.md#diagnosis-reports-the-maximal-independent-set-in-one-pass
        //# The compiler MUST recover from an error and report the maximal set of independent problems in one pass rather than only the first.
        if matches!(crate::lower::core_of(db, id), crate::core::Core::Poison(_)) {
            continue;
        }
        for (pat, body) in arms.iter() {
            // A GUARD pattern `(guard <pat> <cond>)` binds names in `<pat>`, and its `<cond>` is a USE
            // site (`(guard x (> x 0))` reads `x`) — NOT a second binding. Split them: binders come from
            // the inner `<pat>`, and the guard `<cond>` is scanned for uses alongside the arm body.
            // `arm_pattern_binders` over the WHOLE guard form would wrongly collect the cond's `x` as a
            // binder AND miss it as a use — a false "unused" (a guard binder used only in the cond).
            let (binder_pat, guard_cond) = match db.ast.as_form(*pat, "guard") {
                Some(g) if g.len() == 2 => (g[0], Some(g[1])),
                _ => (*pat, None),
            };
            // The arm's pattern binders (variant payloads, tuple elements, scalar binders); a `_`/`..`
            // separator and a literal bind nothing.
            let pat_binders = crate::resolve::arm_pattern_binders(db, binder_pat);
            if pat_binders.is_empty() {
                continue;
            }
            // The binder NAMES referenced in the arm body OR the guard cond (both resolving to a match
            // binder) — a binder used in either is used.
            let mut referenced = used_match_binder_names(db, *body);
            if let Some(cond) = guard_cond {
                referenced.extend(used_match_binder_names(db, cond));
            }
            // A `(bin …)` pattern's DEPENDENT-SIZE operand is a USE of an earlier segment binder, not a
            // binder itself: `(bin (u8 n) (bytes body n))` reads `n` as the size of the `body` segment. That
            // use lives IN THE PATTERN (not the arm body), so `used_match_binder_names(body)` misses it —
            // leaving `n` falsely flagged CDZ0306 "never used" (v-lsp: red squiggles in the guide's
            // length-prefixed-frame examples). Collect the size-operand OCCURRENCES so their NAMES count as
            // uses AND the occurrences are excluded from the binder candidates below (a size operand also
            // arrives via the syntactic `arm_pattern_binders` walk as a bogus second "binder").
            let size_occs = bin_pattern_size_occs(db, binder_pat);
            for &so in &size_occs {
                if let Some(nm) = db.ast.as_name(so) {
                    referenced.insert(nm.to_string());
                }
            }
            for (name, name_occ) in pat_binders {
                if name.starts_with('_') || referenced.contains(&name) {
                    continue;
                }
                // A bin-segment SIZE operand (`n` in `(bytes body n)`) is a use of an earlier binder, not a
                // binder — skip it as a candidate (its name is already counted used above).
                if size_occs.contains(&name_occ) {
                    continue;
                }
                // `arm_pattern_binders` is deliberately SYNTACTIC — it does NOT resolve ctor-vs-binder, so a
                // bare NULLARY VARIANT CONSTRUCTOR pattern (`D` in `((A) 1) ((B) 2) (D …)`, or a bare `None`)
                // arrives here looking like an unused binder. It binds NOTHING (it is a refutable ctor
                // match), so it can't be "unused" — flagging it and auto-prefixing `_D` would silently
                // DOWNGRADE the precise variant arm to a catch-all wildcard. Skip it, matching the
                // ctor-vs-binder authority `lower::collect_pattern_binders` uses (`eval::variant_disc_of`).
                if crate::eval::variant_disc_of(db, name_occ).is_some() {
                    continue;
                }
                binders.push(Binder {
                    name_occ,
                    target: name_occ,
                    name,
                    kind: "match binding",
                    precomputed_unused: true, // decided by the name scan, not the `used` occ set
                });
            }
        }
    }

    // ANONYMOUS-LAMBDA PARAMETERS. A `(fn (x) …)` param never referenced in the lambda body is unused,
    // exactly like an unused DEF parameter — but a lambda is not in `db.defs`, so the def-param loop above
    // misses it. Gated on `head_name == "fn"`: a DEF's signature list also resolves to a `Lambda` (its
    // params are already checked by the def-param loop), so only an ANONYMOUS `(fn …)` is handled here (no
    // double-report). Uses the same name-based check the def-param path does (`used_param_names` over the
    // body — a reference resolves to a `Param`, synthesis-independent), which is scope-correct.
    for i in 0..node_count {
        let id = StructId(i as u32);
        if !db.is_user_node(id) || db.ast.head_name(id) != Some("fn") {
            continue;
        }
        let Resolved::Lambda { params, body } = crate::resolve::resolved_of(db, id) else {
            continue;
        };
        if params.is_empty() {
            continue;
        }
        let referenced = used_param_names(db, body);
        for &p in params.iter() {
            let name_occ = param_name_occ(db, p);
            let Some(name) = db.ast.as_name(name_occ).map(str::to_string) else {
                continue;
            };
            if name.starts_with('_') || referenced.contains(&name) {
                continue;
            }
            binders.push(Binder {
                name_occ,
                target: name_occ,
                name,
                kind: "parameter",
                precomputed_unused: true, // decided by the reference-name set, not the `used` occ set
            });
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
    // A FAILED export (a `(export …)` naming no definition) whose name is a NEAR-MISS of a real definition
    // — `(export mian)` for `(def (main) …)` — means the author INTENDED to export that definition; the
    // real defect is the export typo (its own CDZ0101 "export `mian` … did you mean `main`?"), not that
    // `main` is "unused". Flagging the intended-export target CDZ0306 "unused definition" is CONSEQUENT,
    // misleading noise (the def IS meant to be reached — through the export the author misspelled). So
    // treat a def that a missing export names as its nearest match as "exported" for the unused check.
    // The nearest-match pool + cutoff are the SAME `suggest::nearest` over the defined names the export
    // error itself uses (`collect_faults`' missing-export loop), so the suppression fires exactly when the
    // export error offers that def as its "did you mean?" fix — the two can never disagree. A FAR-miss
    // export (`(export zzzzz)`) has no near def, so no def is suppressed and a genuinely-unused def still
    // warns.
    let defined_names: Vec<String> = db.defs.iter().map(|d| d.name.clone()).collect();
    let intended_export_targets: std::collections::HashSet<String> = db
        .exports
        .iter()
        .filter(|e| e.def.is_none())
        .filter_map(|e| crate::diag::suggest::nearest(&e.name, &defined_names))
        .collect();
    let def_binders: Vec<Binder> = db
        .defs
        .iter()
        .filter(|d| d.params.is_empty()) // nullary value defs only (see note above)
        .filter(|d| !exported.contains(d.name.as_str()))
        .filter(|d| !intended_export_targets.contains(&d.name))
        // A `@test` definition is an ENTRY POINT (of the `cdz test` build), so it is "used" exactly as an
        // exported def is — it is invoked by the test runner, not by another def in this program. Flagging
        // it "never used" is a false positive introduced by the test marker, so skip a def whose body is in
        // the `@test` set (`db.tests`, keyed by body occ). A non-test unused def still warns.
        .filter(|d| !d.body.is_some_and(|b| db.tests.contains(&b)))
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
            crate::abi_bridge::diagnostic_warning(
                Code::UnusedBinding,
                format!(
                    "unused {}: `{}` is never used (prefix with `_` to silence)",
                    b.kind, b.name
                ),
                Some(b.name_occ),
            )
            .with_fix(crate::abi_bridge::diagnostic_fix_from_fix(&fix)),
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
            // A `@param` site `(: (@ (param …) name) Type)` is also a DECLARATION — a runtime input the
            // `param_sidecar` pass consumes to generate the `Param` effect, not an expression evaluated for a
            // value. It reaches here as a non-final colon-annotation form (not a `def`/`effect` head), so the
            // head-name skip above misses it; without this guard EVERY `@param` declaration spuriously warns
            // CDZ0307 "computed but discarded" (its declared type is read as a thrown-away value). Skip it.
            if crate::param_sidecar::is_param_site(&db.ast, s) {
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
                crate::abi_bridge::diagnostic_warning(
                    Code::DiscardedValue,
                    format!(
                        "this `{}`-typed value is computed but discarded — a non-final form of a \
                         sequencing block is evaluated only for its effect, and this form has none \
                         (bind it with a `let` if you meant to use it, or remove it)",
                        ty.render_name(&db.name_ctx())
                    ),
                    Some(s),
                )
                .with_fix(crate::abi_bridge::diagnostic_fix_from_fix(&fix)),
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

/// The NAME occurrences in an arm-body / guard-cond subtree that could REFERENCE a match-arm binder — a
/// plain name-presence scan (every name occurrence that is not itself a binding-declaration name). Used
/// to decide whether a match-arm pattern binder is used: a match binder is scoped to its OWN arm, and its
/// declaration occurrences live in the PATTERN (not the body/cond) — so ANY occurrence of the binder name
/// in the arm body/cond subtree is a use of it. A resolution-KIND check is NOT reliable here (a scalar /
/// whole-value binder resolves to `Ref { value: scrutinee }` — indistinguishable from an ordinary
/// variable `Ref` without knowing the scrutinee shape, which varies: a param → `Ref`-to-`Param`, a
/// literal → `Ref`-to-`Int`, …). The name-presence scan is robust regardless of scrutinee shape;
/// `is_let_binding_name` skips an inner `let`'s own NAME position (a declaration, not a use). CONSERVATIVE
/// on inner shadowing: a binder shadowed by an inner same-named binder that is then used counts the outer
/// as "used" (under-reports the rare shadow case — never a false "unused"), the right side to err on for a
/// warning.
/// The DEPENDENT-SIZE operand occurrences of every `(bin …)` segment reachable in a match arm's PATTERN —
/// `n` in `(bytes body n)` / `(utf8 s n)`. Such an operand is a USE of an earlier segment binder (the size
/// to read), NOT a binder itself; the unused-binding scan must count it as a use and must not treat it as a
/// candidate binder (the syntactic `arm_pattern_binders` walk collects it as a bogus second binder). Walks
/// the pattern subtree so a `(bin …)` nested in a tuple/ctor pattern is reached too.
fn bin_pattern_size_occs(db: &mut Db, pat: StructId) -> std::collections::HashSet<StructId> {
    let mut out = std::collections::HashSet::new();
    let node_ids: Vec<StructId> = collect_subtree(db, pat);
    for id in node_ids {
        if db.ast.head_name(id) != Some("bin") {
            continue;
        }
        if let crate::resolved::Resolved::Bin { segs } = crate::resolve::resolved_of(db, id) {
            for s in segs.iter() {
                match s.kind {
                    crate::resolved::SegKind::Bytes { size: Some(occ) } => {
                        out.insert(occ);
                    }
                    crate::resolved::SegKind::Utf8 { size } => {
                        out.insert(size);
                    }
                    _ => {}
                }
            }
        }
    }
    out
}

/// Every occurrence in the subtree rooted at `id` (inclusive), in arena order. A cheap syntactic walk used
/// where a helper must inspect every node of a pattern/expression form.
fn collect_subtree(db: &Db, id: StructId) -> Vec<StructId> {
    let mut out = Vec::new();
    fn walk(db: &Db, id: StructId, out: &mut Vec<StructId>) {
        out.push(id);
        if let crate::ast::Struct::List(kids) = db.ast.get(id) {
            for c in kids.clone() {
                walk(db, c, out);
            }
        }
    }
    walk(db, id, &mut out);
    out
}

fn used_match_binder_names(db: &mut Db, body: StructId) -> std::collections::HashSet<String> {
    fn walk(db: &mut Db, id: StructId, out: &mut std::collections::HashSet<String>) {
        if let Some(name) = db.ast.as_name(id).map(str::to_string)
            && !is_let_binding_name(db, id)
        {
            out.insert(name);
        }
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
/// The module NAME the bundled kernel exports under (its `(module "verify-kernel" …)` head).
const VERIFY_KERNEL_NAME: &str = "verify-kernel";
/// Whether the compiler actually LINKS the bundled kernel for a verification-using program. OFF until a3/b3
/// (the discharge-eval + oracle) consume it — linking the kernel BEFORE its consumers exist only changes a
/// verification-using program's compile (single-file → linked package) with no benefit, which regresses the
/// existing `@test @ensures` programs (v-property-testing's TESTED tier, which uses `@ensures` but does NOT
/// yet need kernel discharge). The mechanism (scan + prepend + the `.bin`) lands behavior-neutral now; a3
/// flips this to `true` together with the discharge wiring, so the kernel is linked exactly when consumed.
const VERIFY_KERNEL_LINKING_ENABLED: bool = false;
/// The compiler-bundled verification KERNEL as PRE-ENCODED codec bytes (Inc-b (A1), design §9) — the trusted
/// HOL kernel the compiler links + compile-time-evals to discharge `@requires`/`@ensures`/`@trap_free`/
/// `@invariant` obligations. Embedded as codec BYTES (not source text) because rcdzc must NOT depend on the
/// `cadenza-syntax` reader in lib code (the "COPY, DON'T DEPEND" directive — rcdzc vendors `codec.rs`); the
/// bytes are `cadenza_syntax::codec::encode(sexpr::read(verify_kernel.cdz))`, decoded here by rcdzc's OWN
/// `codec::decode`. Regenerated at BUILD TIME from `src/verify_kernel.cdz` by `build.rs` (into `OUT_DIR`),
/// so there is no committed golden to go stale (was a recurring stale-golden fleet-red: #6236, #6460).
const VERIFY_KERNEL_BIN: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/verify_kernel.bin"));

/// Decode the bundled verification-kernel bytes into an rcdzc `Arenas` (rcdzc's own codec — no reader dep).
/// `None` if the embedded asset fails to decode — a build-time invariant (checked in), so the caller treats
/// `None` as "kernel unavailable, skip verification linking" rather than a user error.
fn bundled_verify_kernel_arena() -> Option<crate::ast::Arenas> {
    crate::codec::decode(VERIFY_KERNEL_BIN)
}

/// Whether `arena` USES a verification annotation (`@requires`/`@ensures`/`@trap_free`/`@invariant`) — a
/// cheap top-level-ish scan of the raw arena for the annotation head. GATES kernel-linking (design §9): a
/// program that uses NO verification annotation stays on the untouched fast path (the kernel is not linked,
/// `is_linked_package` stays false, zero blast radius); only a verification-USING program links the kernel.
/// The annotation reifies as `(@ (requires …) …)` etc., so the head `requires`/`ensures`/`trap-free`/
/// `invariant` appears as a `Name` leaf; a whole-arena leaf scan is cheap and conservative (a false
/// positive only over-links, still sound; the names are reserved verification heads).
fn uses_verification_annotation(arena: &crate::ast::Arenas) -> bool {
    arena.leaves.iter().any(|leaf| {
        matches!(
            leaf,
            crate::ast::Leaf::Name(n)
                if n.as_ref() == "requires" || n.as_ref() == "ensures" || n.as_ref() == "trap-free" || n.as_ref() == "trap_free"
                    || n.as_ref() == "invariant"
        )
    })
}

/// The result of [`link_inputs`]: the linked (possibly-merged) arena, its linkage (`None` for the
/// single-file fast path), and the per-file pre-resolve SOURCE snapshots (indexed to match `Db::file_of`,
/// for the `Ast.module` self-reflection fill).
type LinkedInputs = (
    crate::ast::Arenas,
    Option<crate::link::Linkage>,
    Vec<Option<std::rc::Rc<crate::ast::Arenas>>>,
);

/// The per-file self-reflection SOURCE snapshot for [`Db::source_snapshots`](crate::db::Db::source_snapshots):
/// a clone of the comment-stripped arena ONLY when the file actually contains an `(. Ast module)` form, else
/// `None`. Gating the clone on real `Ast.module` use keeps the overwhelmingly common (non-self-reflecting)
/// program clone-free; a self-reflecting file pays one clone so the `Prim::ReflectModule` fill can reflect its
/// pre-mutation source (the live arena is rewritten in place before lowering).
fn module_snapshot(arena: &crate::ast::Arenas) -> Option<std::rc::Rc<crate::ast::Arenas>> {
    // Captured for `Ast.module` self-reflection OR `Type.ast`/`Type.ast-generic` type→AST reflection —
    // both reflect a file's PRE-RESOLVE source, so either use demands the one snapshot clone.
    (crate::quote::contains_ast_module(arena) || crate::quote::contains_type_ast(arena))
        .then(|| std::rc::Rc::new(arena.clone()))
}

fn link_inputs(ast_arts: &[&Artifact], entry_name: Option<&str>) -> Result<LinkedInputs, Reject> {
    match ast_arts {
        // No `ast` artifact in the input list — the source tree the tool requires to derive a component
        // is absent, so this is a diagnostic (`compile` turns the `Reject` into an error `Diagnostic`),
        // never an empty or arbitrary component.
        //= spec/contracts/build-tool-interface.md#the-tool-s-inputs-are-a-kinded-artifact-list
        //# An input artifact list that omits the source tree the tool requires to derive a component MUST be reported as a diagnostic rather than producing an empty or arbitrary output.
        [] => Err(Reject::decline("no `ast` input artifact")),
        // The overwhelmingly common case: exactly one file, no package framing. Decode it as-is — flat
        // namespace, no linkage — so a one-file program compiles through the identical path it always
        // did. EXCEPTION (Inc-b (A1), design §9): if the single file USES a verification annotation
        // (`@requires`/`@ensures`/`@trap_free`/`@invariant`), LINK the bundled verification kernel as a
        // package member so the compiler can compile-time-discharge the obligations against the real
        // kernel — and so the kernel's `Thm` keeps its unforgeable opacity (`is_linked_package` true only
        // for a verification-using program; a non-verification program stays on the fast path unchanged).
        [only] if entry_name.is_none() => {
            let mut user = crate::codec::decode(&only.bytes)
                .ok_or_else(|| Reject::decline("binary AST failed to decode"))?;
            // Comment-strip the raw arena (idempotent with `Db::load`'s own strip) BEFORE capturing the
            // self-reflection SOURCE snapshot, so a comment change never alters the reflected `Ast.module`
            // value — the same canonical form `__ast__`/`quote` reflect. The snapshot is the module's
            // pre-resolve source; the fill (`core_of` `Prim::ReflectModule`) reflects it, because the live
            // arena is mutated by `Db::load` before lowering. Snapshots are ordered to match `file_of`.
            crate::db::strip_comments(&mut user);
            if VERIFY_KERNEL_LINKING_ENABLED
                && uses_verification_annotation(&user)
                && let Some(kernel) = bundled_verify_kernel_arena()
            {
                // Link kernel + user as a package; the user file is the entry. The kernel exports its
                // rules + abstract `Thm`; the user imports them (and the compiler synthesizes the
                // discharge program against them at a3/b3). Snapshots in the SAME order as `files`
                // ([kernel, user]) so `file_of(occ)` indexes the right module for `Ast.module`. Cloned
                // ONLY for a file that actually contains an `(. Ast module)` form (the kernel never does).
                let snapshots = vec![module_snapshot(&kernel), module_snapshot(&user)];
                let files = vec![
                    (VERIFY_KERNEL_NAME.to_string(), kernel),
                    (only.name.clone(), user),
                ];
                let linked = crate::link::link(&files, &only.name)?;
                let linkage = linked.linkage();
                Ok((linked.arenas, Some(linkage), snapshots))
            } else {
                // No verification annotation (or linking not yet enabled) — the fast path: flat namespace,
                // no linkage. One file, so its snapshot is index 0 (`file_of` returns `None` here → the
                // fill uses index 0). Snapshot the pre-mutation source ONLY if the program self-reflects
                // (`Ast.module`) — the common program clones NOTHING; `Db::load` mutates the returned arena
                // in place, so a self-reflecting program keeps an independent clone.
                let snapshot = module_snapshot(&user);
                Ok((user, None, vec![snapshot]))
            }
        }
        // A package: decode every file, then splice. The entry defaults to the sole file's name when
        // exactly one file was supplied (a single-file package needs no explicit entry); otherwise the
        // caller must name the entry. A single-file package still carries linkage (its `(import …)`
        // clauses, if any, are validated), but with one file there is no cross-file scoping to enforce.
        _ => {
            let mut files = Vec::with_capacity(ast_arts.len());
            let mut snapshots = Vec::with_capacity(ast_arts.len());
            for art in ast_arts {
                let mut arena = crate::codec::decode(&art.bytes).ok_or_else(|| {
                    Reject::decline(format!("binary AST for `{}` failed to decode", art.name))
                })?;
                // Peel `(comment "…" <form>)` wrappers BEFORE `link` scans this file's top-level items —
                // `link` reads `(import …)`/`(export …)` off the raw arena (before `Db::load`'s own
                // `strip_comments` runs), so a `//`/`///` comment on an `(import …)` would leave it wrapped
                // and unrecognized → spliced as an unmodeled top-level form → "`import` … not modeled".
                // The db-level strip already fixes the single-file/def/type/export cases; this extends the
                // same peel to the LINK scan so a commented import in a package resolves identically.
                crate::db::strip_comments(&mut arena);
                // Capture each file's comment-stripped SOURCE as its self-reflection snapshot — the merge
                // flattens the file roots, so `Ast.module` in THIS file must reflect THIS file's own module
                // root. Pushed in file order, matching the `FileSpan` index `file_of` returns. Cloned ONLY
                // for a file that contains an `(. Ast module)` form (a file that never self-reflects → `None`,
                // no clone).
                snapshots.push(module_snapshot(&arena));
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
            Ok((linked.arenas, Some(linkage), snapshots))
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
        diagnostics: rejects
            .iter()
            .map(crate::abi_bridge::diagnostic_from_reject)
            .collect(),
        // An early failure — no emit ran, so no CSE partition compares happened.
        cse_partition_core_eq_calls: 0,
        value_range_uncached_calls: 0,
        param_apply_extra_handled_calls: 0,
        is_cse_shareable_uncached_calls: 0,
    }
}

#[cfg(test)]
mod dedup_selfsuppress_tests {
    use super::*;
    use crate::ast::StructId;
    use crate::diag::{Code, Reject};

    /// A lone CDZ0900 DECLINE (`Reject::unsupported`) must SURVIVE `dedup_faults` — it is the primary
    /// report of a not-yet-built construct (seq-286: a decline MUST be visible/coded, never silently
    /// masked), not a weaker consequence of a co-located reject. Regression pin for the self-suppression
    /// bug: a CDZ0900 is BOTH `is_decline()` and `code.is_some()`, so `coded_nodes` used to include its own
    /// node and then `:4916` dropped it — a lone CDZ0900 vanished from `diagnostics()`. Now `coded_nodes`
    /// counts only genuine coded REJECTS, so a lone CDZ0900 survives while one co-located with a real reject
    /// is still dropped (the reject names the real defect).
    #[test]
    fn a_lone_cdz0900_decline_survives_dedup_but_one_shadowed_by_a_reject_is_dropped() {
        let db = crate::db::Db::load(crate::testkit::parse(
            "(module m (def (main) 0) (export main))",
        ));
        let node_a = StructId(1); // carries a genuine reject + a co-located CDZ0900 decline
        let node_c = StructId(2); // carries a LONE CDZ0900 decline
        let faults = vec![
            Reject::coded(Code::Malformed, "a genuine reject at node A").at(node_a),
            Reject::unsupported("a CDZ0900 decline SHADOWED by the reject at node A").at(node_a),
            Reject::unsupported("a LONE CDZ0900 decline at node C").at(node_c),
        ];
        let out = dedup_faults(&db, faults, false);
        let has = |needle: &str| out.iter().any(|r| r.message.contains(needle));
        assert!(
            has("a genuine reject at node A"),
            "the genuine reject is kept"
        );
        assert!(
            has("a LONE CDZ0900 decline at node C"),
            "a lone CDZ0900 decline MUST survive dedup (self-suppression regression, seq-286)"
        );
        assert!(
            !has("a CDZ0900 decline SHADOWED"),
            "a CDZ0900 decline co-located with a genuine reject is still dropped"
        );
    }
}
