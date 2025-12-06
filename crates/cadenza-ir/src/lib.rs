//! Intermediate Representation for Cadenza
//!
//! This module provides a typed, SSA-like IR suitable for optimization and code generation.
//! The IR is target-independent and can be lowered to various backends including:
//! - TypeScript/JavaScript (for browser)
//! - Rust (for native compilation)
//! - WebAssembly
//! - LLVM IR
//!
//! The IR uses Single Static Assignment (SSA) form where each value is assigned exactly once.
//! Control flow is represented explicitly using basic blocks with terminator instructions.

mod builder;
mod types;

pub use builder::*;
pub use types::*;

#[cfg(test)]
mod tests;

