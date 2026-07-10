//! The backend seam — the one point where the target-neutral front meets a target-specific back.
//!
//! Everything above this seam — resolution, inference, the compile-time evaluator, the boundary
//! layout — is computed the same way whatever the output is; a backend is a function of that meaning
//! and the layout, chosen HERE (`backends-and-targets.md` §The Pipeline Is Target-Neutral Up To A
//! Single Seam). A backend fills the terminal artifact column by reading the earlier columns
//! (`query-engine.md` §Producing An Artifact Is A Column A Backend Fills). Selecting one is a branch
//! on the requested [`Target`]; the query program picks it (`Compiler.compile(db, Compiler.Target.…)`).
//!
//! Stage 0 ships one backend (wasm → a WebAssembly component). The `Target` enum is named from genesis
//! so a second backend is a new arm behind this same seam, not a fork of the pipeline.

pub mod wasm;

use crate::db::Db;
use crate::diag::Reject;
use crate::layout::Layout;

/// The output target a compile is requested for — the value the query program names via
/// `Compiler.Target`. `Ord` so it can key a stable per-target artifact map.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Target {
    /// A WebAssembly component (the Stage-0 backend).
    Wasm,
}

impl Target {
    /// The artifact kind a target produces — how the emitted artifact is tagged (`build-tool-
    /// interface.md`; `backends-and-targets.md` §The Emitted Artifact Is Self-Describing By Kind).
    pub fn artifact_kind(self) -> &'static str {
        match self {
            Target::Wasm => "component",
        }
    }
}

/// Emit the artifact for `target` from the program in `db` under `layout`. The seam: dispatch to the
/// chosen backend, each a producer of the artifact column over the same upstream columns.
pub fn emit(target: Target, db: &mut Db, layout: &Layout) -> Result<Vec<u8>, Reject> {
    match target {
        Target::Wasm => wasm::emit(db, layout),
    }
}
