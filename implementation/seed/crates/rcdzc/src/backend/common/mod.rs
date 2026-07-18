//! Backend-AGNOSTIC analyses + policy shared by every backend — the shared space below the
//! `rust`/`wasm` seam (operator directive, 2026-07-18: common backend code is HOISTED to a shared
//! module, never made `pub` on one backend and cross-called by the other, never copied). A backend
//! reads these; none of them touches a backend-specific representation (no wasm `Lir`, no Rust source),
//! so adding a third backend — or removing one — leaves them untouched.
//!
//! - [`diverge`] — divergence + flow-refinement analyses over the structured `Core` (both backends need
//!   "does this body/continuation diverge on every path?" and "what interval does a branch guarantee?").

pub(crate) mod diverge;
