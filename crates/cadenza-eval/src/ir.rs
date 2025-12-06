//! Intermediate Representation (IR) module.
//!
//! This module provides a typed, SSA-like IR suitable for optimization and code generation.
//! The IR is target-independent and designed for WASM code generation with WasmGC.

mod builder;
mod generator;
mod optimize;
mod types;
mod wasm;

pub use builder::*;
pub use generator::*;
pub use optimize::*;
pub use types::*;
pub use wasm::*;

#[cfg(test)]
mod tests;
