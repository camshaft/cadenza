//! `cadenza-seed` — the native host and CLI library for the seed toolchain.
//!
//! Exposes the host (runs finished components via wasmtime), the corpus loader + behavior
//! gate, and the `probe` harness (compile→validate→run in one structured outcome). The binary
//! (`main.rs`) and the integration tests (`tests/`) both build on this library, so a
//! component-emission or validation regression is caught by a test assertion rather than by
//! hand-running `emit` + `wasm-tools`.

/// Compiler selection (old `cdz-compiler` oracle vs opt-in `rcdzc`) + the common multi-diagnostic
/// `CompileOutput` currency. See `compiler.rs`.
pub mod compiler;
pub mod corpus;
pub mod host;
pub mod probe;
/// The names the host forwards from the value-heap runtime into each emitted program's imports —
/// GENERATED from the runtime WIT by `xtask build` (see xtask/src/wit_envelope.rs), the same source
/// of truth as the compiler's envelope. Do not hand-edit `runtime_funcs.rs`.
pub mod runtime_funcs;
