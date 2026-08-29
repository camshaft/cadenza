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

// The `Target` enum + its `artifact_kind`/`needs_spans` impls now live in the shared
// `cadenza-compile-abi` crate (the compile-boundary contract types both `rcdzc` and `cdz` agree on).
// Re-exported here so `crate::backend::Target` and `rcdzc::Target` (lib.rs) stay byte-stable and every
// existing consumer path keeps resolving. The backend `emit` seam below — which dispatches on the
// target and reads a live `Db`/`Layout`/`SpanData` — is the compiler IMPLEMENTATION and stays here.
pub use cadenza_compile_abi::Target;

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
