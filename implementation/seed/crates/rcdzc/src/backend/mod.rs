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
    /// Rust source — a self-contained `.rs` module (one `pub fn` per export) that links into an
    /// existing Rust codebase as ordinary source, with no component boundary and no FFI. The
    /// structured second backend (`backends-and-targets.md` §A Backend Linearizes The Core Only If Its
    /// Target Is Linear).
    Rust,
}

impl Target {
    /// The artifact kind a target produces — how the emitted artifact is tagged (`build-tool-
    /// interface.md`; `backends-and-targets.md` §The Emitted Artifact Is Self-Describing By Kind).
    pub fn artifact_kind(self) -> &'static str {
        match self {
            Target::Wasm => "component",
            Target::Rust => "rust",
        }
    }
}

/// Emit the artifact for `target` from the program in `db` under `layout`. The seam: dispatch to the
/// chosen backend, each a producer of the artifact column over the same upstream columns.
pub fn emit(target: Target, db: &mut Db, layout: &Layout) -> Result<Vec<u8>, Reject> {
    let result = match target {
        Target::Wasm => wasm::emit(db, layout),
        Target::Rust => rust::emit(db, layout),
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
