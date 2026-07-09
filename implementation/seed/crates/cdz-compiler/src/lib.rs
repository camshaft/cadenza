//! `cdz-compiler` — the reference Cadenza → WebAssembly-component compiler (`cdz-rustc`).
//!
//! The foreign-language *seed compiler* (constitution XIV; bootstrap.md). A pure function
//! from a program's canonical binary AST to a complete WebAssembly component — no host, no
//! filesystem, no wasmtime — so the same core compiles to `wasm32` and is wrapped as a
//! component exporting `compile : list<u8> -> list<u8>`, the SAME ABI the Cadenza-authored
//! compiler exports.
//!
//! Public surface:
//!   - [`ast`]      — the s-expression reader and the canonical binary AST codec.
//!   - [`codegen`]  — the AST → component-bytes lowering (`compile`, `compile_program`).
//!   - [`diagnostics`] — machine-readable diagnostic codes.

pub mod ast;
pub mod codegen;
pub mod diagnostics;

pub use ast::Node;
pub use codegen::{compile, compile_program, Decline};
