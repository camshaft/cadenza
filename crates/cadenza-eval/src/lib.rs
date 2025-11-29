//! Cadenza Evaluator
//!
//! A minimal tree-walk evaluator for the Cadenza language. The evaluator
//! interprets the AST produced by cadenza-syntax, supporting macro expansion
//! and providing a compiler API for building modules.
//!
//! # Core Components
//!
//! - [`Interner`]: String interning for efficient identifier comparison
//! - [`Value`]: Runtime values including functions and macros
//! - [`Env`]: Scoped environment for variable bindings
//! - [`Compiler`]: The compiler state that accumulates definitions
//! - [`eval`]: The main evaluation function

mod compiler;
mod env;
mod error;
mod eval;
mod interner;
mod map;
mod value;

pub use compiler::Compiler;
pub use env::Env;
pub use error::{Error, Result};
pub use eval::eval;
pub use interner::{InternedId, Interner};
pub use map::Map;
pub use value::Value;

#[cfg(test)]
mod tests;
