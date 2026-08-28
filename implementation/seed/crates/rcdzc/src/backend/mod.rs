//! The backend seam — the one point where the target-neutral front meets a target-specific back.
//!
//! Everything above this seam — resolution, inference, the compile-time evaluator, the boundary
//! layout — is computed the same way whatever the output is; a backend is a function of that meaning
//! and the layout, chosen HERE (`backends-and-targets.md` §The Pipeline Is Target-Neutral Up To A
//! Single Seam). A backend fills the terminal artifact column by reading the earlier columns
//! (`query-engine.md` §Producing An Artifact Is A Column A Backend Fills). Selecting one is a branch
//! on the requested [`Target`]; the query program picks it (`Compiler.compile(db, Compiler.Target.…)`).
//!
//! Two backends ship here: wasm → a WebAssembly component, and rust → Rust source. Each is one arm
//! behind this same seam (`Target`), not a fork of the pipeline — the concrete demonstration that
//! everything above the seam (resolve/infer/lower/layout) is target-neutral. The Rust backend consumes
//! the typed structured core DIRECTLY (it never builds the flat wasm `Lir`), so it also demonstrates
//! the flat rung is a property of the linearizing backend, not a shared stage
//! (`backends-and-targets.md` §The Flat Instruction Rung Is A Property Of A Linearizing Backend).

// `common` is internal-only (shared backend analyses like `diverge`/`export_name`, used by the `rust`
// and `wasm` arms within this crate) — NOT part of rcdzc's public surface, so `pub(crate)`. External
// consumers reach only `backend::wasm` (cdz/cdz-wasm/cdz-kernel/rcdzc-wasm); none names `backend::common`,
// so tightening it avoids accidentally stabilizing an internal analysis (PR#584 API-hygiene nit).
pub mod cadenza;
pub(crate) mod common;
pub mod rust;
pub mod wasm;

use crate::db::Db;
use crate::diag::Reject;
use crate::layout::Layout;
use tracing::trace;

/// The output target a compile is requested for — the value the query program names via
/// `Compiler.Target`. `Ord` so it can key a stable per-target artifact map.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Target {
    /// A WebAssembly component (the Stage-0 backend).
    Wasm,
    /// A WebAssembly component carrying EMBEDDED debug information (Mode E of
    /// `DESIGN-debug-info-rcdzc.md`) — the same component as [`Target::Wasm`] with the wasm `name`
    /// custom section (and, later increments, `.debug_*` DWARF) appended to its embedded core module.
    /// The debug sections are inert (they move no executed byte) and strippable, so a `WasmDebug`
    /// artifact stripped of custom sections is byte-identical to the `Wasm` artifact — the
    /// reproducibility anchor (§5). Its artifact kind stays `"component"`: it is a decorated component,
    /// not a new output kind (that is Mode S, [`Target::Dwarf`], a separate `"dwarf"` artifact).
    ///
    //= spec/capabilities/debug-information.md#debug-information-may-be-embedded-or-emitted-as-a-sidecar
    //# Debug information MAY be emitted embedded in the artifact it describes, so that the debug information travels with the runnable artifact as a single self-describing file.
    WasmDebug,
    /// A standalone DWARF SIDECAR module (Mode S of `DESIGN-debug-info-rcdzc.md` §9.2) — a
    /// `kind == "dwarf"` artifact SEPARATE from the runnable component: a bare core wasm module carrying
    /// only the `.debug_*` custom sections, which a debugger loads alongside the (lean, undecorated)
    /// runnable. The second enablement mode ("support both", the operator's third ruling): Mode E embeds
    /// the sections; Mode S detaches them. Its code offsets reference the runnable component's code
    /// section, which is byte-identical whether debug rides embedded or here (the sections are appended
    /// inertly after the code), so the two modes share the offset computation.
    ///
    //= spec/capabilities/debug-information.md#debug-information-may-be-embedded-or-emitted-as-a-sidecar
    //# Debug information MAY instead be emitted as a separate artifact linked to the artifact it describes, so that a deployment can ship the runnable artifact lean and the debug information alongside it.
    Dwarf,
    /// Rust source — a self-contained `.rs` module (one `pub fn` per export) that links into an
    /// existing Rust codebase as ordinary source, with no component boundary and no FFI. The
    /// structured second backend (`backends-and-targets.md` §A Backend Linearizes The Core Only If Its
    /// Target Is Linear).
    Rust,
    /// Rust source in ASYNC, GAS-METERED form — every emitted function is an `async fn` taking a
    /// caller-supplied `env: &mut impl CdzEnv` and awaiting `env.consume(1)` at entry, so the host meters
    /// fuel and can yield COOPERATIVELY at each step (the emitted computation is bounded and pausable
    /// rather than a runaway synchronous call). Same value semantics as [`Target::Rust`]; the difference
    /// is the async/`env`-threaded calling convention, so it composes into an async Rust codebase where
    /// untrusted Cadenza code must be fuel-bounded.
    RustAsync,
    /// Cadenza surface — the optimized program lowered BACK to a Cadenza binary-AST artifact
    /// (`backend::cadenza`). Not a runnable artifact: it re-emits the program itself, AFTER
    /// resolution/inference/const-fold/optimization, as the binary AST (`kind == "ast"`), so it can be
    /// fed back through `compile` for round-trip idempotence, piped into the syntax system for sexpr/ML
    /// inspection of what lowering did, or handed to the Lean oracle. The "inspect the meaning" backend.
    Cadenza,
}

impl Target {
    /// The artifact kind a target produces — how the emitted artifact is tagged (`build-tool-
    /// interface.md`; `backends-and-targets.md` §The Emitted Artifact Is Self-Describing By Kind).
    pub fn artifact_kind(self) -> &'static str {
        match self {
            // A debug-carrying component is still a `component` — a decorated one, not a new kind.
            Target::Wasm | Target::WasmDebug => "component",
            // The detached DWARF sidecar is its own output kind.
            Target::Dwarf => "dwarf",
            // Both Rust forms are `rust`-kinded `.rs` source — they differ in calling convention, not
            // in artifact kind (a consumer picks the flavor by which target it asked to emit).
            Target::Rust | Target::RustAsync => "rust",
            // The re-emitted program is a binary-AST artifact — the same `"ast"` kind an `ast` input
            // artifact carries, since it IS a Cadenza program (round-trippable straight back in).
            Target::Cadenza => "ast",
        }
    }

    /// Whether emitting this target requires the `spans` input artifact (the source side-table debug
    /// info is drawn from — `DESIGN-debug-info-rcdzc.md` §9.4). A debug target requested without spans
    /// DECLINES (`compile`), rather than silently producing an undecorated artifact. Both the embedded
    /// (`WasmDebug`) and the detached (`Dwarf`) debug modes draw from the `spans` side-table.
    pub fn needs_spans(self) -> bool {
        matches!(self, Target::WasmDebug | Target::Dwarf)
    }
}

/// Emit the artifact for `target` from the program in `db` under `layout`. The seam: dispatch to the
/// chosen backend, each a producer of the artifact column over the same upstream columns. `spans` is
/// the decoded source side-table (`DESIGN-debug-info-rcdzc.md` §2.1a), present when a debug target
/// drives the compile — the wasm backend reads it to emit the `name` + DWARF sections; other targets
/// ignore it.
pub fn emit(
    target: Target,
    db: &mut Db,
    layout: &Layout,
    spans: Option<&crate::spans::SpanData>,
    external_debug_info: Option<&str>,
) -> Result<Vec<u8>, Reject> {
    let result = match target {
        // A plain component may still carry an `external_debug_info` pointer at a detached sidecar (Mode
        // S) — the debug sections themselves stay out of it (that is the point of a lean component).
        Target::Wasm => wasm::emit(db, layout, None, external_debug_info),
        // A debug component draws its `name` + DWARF sections from the span side-table. `compile`
        // guarantees `spans` is present for a `needs_spans()` target (else it declined, §9.4). It embeds
        // its own DWARF, so it needs no external pointer.
        Target::WasmDebug => wasm::emit(db, layout, spans, None),
        // The detached DWARF sidecar (Mode S). `compile` guarantees `spans` is present (§9.4); a caller
        // that reached here without it is a bug, so decline rather than emit a positionless sidecar.
        Target::Dwarf => match spans {
            Some(s) => wasm::emit_dwarf(db, layout, s),
            None => Err(Reject::decline(
                "a `dwarf` sidecar needs the `spans` input artifact",
            )),
        },
        Target::Rust => rust::emit(db, layout, rust::Mode::Sync),
        Target::RustAsync => rust::emit(db, layout, rust::Mode::Async),
        // The Cadenza backend re-emits the optimized program as binary AST; it reads only the same
        // upstream columns + layout, and ignores `spans`/`external_debug_info` (not a debug target).
        Target::Cadenza => cadenza::emit(db, layout),
    };
    match &result {
        Ok(bytes) => {
            trace!(target: "rcdzc::backend", ?target, bytes = bytes.len(), "emitted artifact")
        }
        Err(r) => {
            trace!(target: "rcdzc::backend", ?target, reason = %r.message, "backend DECLINED")
        }
    }
    result
}
