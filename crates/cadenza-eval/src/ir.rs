//! Intermediate Representation (IR) module.
//!
//! This module provides a typed, SSA-like IR suitable for optimization and code generation.
//! The IR is target-independent and designed for WASM code generation with wasmgc.

mod builder;
mod types;

pub use builder::*;
pub use types::*;

#[cfg(test)]
mod tests;
