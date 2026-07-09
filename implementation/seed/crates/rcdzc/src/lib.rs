//! `rcdzc` — the rewritten reference Cadenza → WebAssembly-component compiler.
//!
//! A from-scratch reimplementation with the spec-shaped nanopass architecture the `cdzc.cdz` rewrite
//! proved, replacing `cdz-compiler/src/codegen.rs`'s 16k-line fused-emit walk. The pipeline is a
//! ladder of single-task passes over typed IR rungs:
//!
//! ```text
//! bytes ─decode─▶ Ast ─resolve─▶ Hir ─infer─▶ typed-Hir ─lower─▶ Mir ─eval─▶ Mir ─select─▶ Lir ─serialize─▶ bytes
//! ```
//!
//! Each rung is a typed sum matched exhaustively; a variant a pass does not handle is a compile
//! error in the compiler, never a silent fall-through. Inference is a SEPARATE pass before lowering
//! (real Hindley-Milner from Phase 1); lowering READS the solved type — it never re-derives one at
//! emit. This is the structural fix for the coarse-`Kind`-re-derived-at-emit defect family
//! (ask-14/…/77; see `inference-plan-learn-from-seed-coarse-kind-mistakes`).
//!
//! ## ABI
//! Artifacts-in / {artifacts, diagnostics}-out (`build-tool-interface.md`, Amendment 0.8.0): the
//! same kinded-artifact interface the eventual self-hosted compiler exports. See [`abi`].
//!
//! ## Reused, not rewritten
//! The generated backend building blocks live in `cdz-compiler` and are consumed here so there is no
//! second copy to drift: [`cdz_compiler::ast`] (Node + codec), and — `#[path]`-included as private
//! modules — the wasm opcode table (`op`) and the scalar-component frame segments (`frame`). Only
//! the compiler's ANALYSIS is new.
//!
//! ## Status
//! Phase 0: `(module m (def (main) 42))` → the 89-byte scalar component, byte-identical to the old
//! compiler. Opt-in (`CADENZA_COMPILER=v2`) until parity; the old crate stays the byte oracle.

pub mod abi;
mod component;
mod diag;
mod fold;
mod infer;
mod ir;
mod layout;
mod lower;
pub mod pipeline;
mod prelude;
mod resolve;
mod select;
mod serialize;
mod ty;

mod heap;
mod render;
mod wasm;

// The generated backend tables, reused verbatim from the old crate's source (one derivation, no
// copy — `xtask build` owns these files). They are self-contained constant tables.
#[path = "../../cdz-compiler/src/frame.rs"]
mod frame;
#[path = "../../cdz-compiler/src/op.rs"]
mod op;
// The value-heap runtime envelope: RT_HEAD/RT_TAIL/RT_IMPORT_CONTENT/RT_MEM/RT_GLOBAL byte
// constants, `mod himport` (import indices), `rt_import_types()`, `RT_N_IMPORTS`, the required-runtime
// pin — all GENERATED from the runtime WIT by `xtask build`, self-contained, reused for the
// runtime-compound (heap-returning) component. One derivation, no drift.
// Not every generated envelope constant is consumed yet (the compile-ABI heads/tails land with the
// `compile` surface); it is a shared GENERATED file, so silence its interim dead-code here rather
// than edit the generator's output.
#[allow(dead_code)]
#[path = "../../cdz-compiler/src/heap_envelope.rs"]
mod heap_envelope;

pub use abi::{Artifact, CompileOutput, Diagnostic, Severity};
pub use pipeline::{compile, compile_bytes, compile_program};

#[cfg(test)]
mod tests;
