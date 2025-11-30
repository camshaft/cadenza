//! Cadenza Evaluator
//!
//! A minimal tree-walk evaluator for the Cadenza language. The evaluator
//! interprets the AST produced by cadenza-syntax, supporting macro expansion
//! and providing a compiler API for building modules.
//!
//! # Core Components
//!
//! - [`interner::InternedString`]: Interned strings for efficient comparison
//! - [`interner::InternedInteger`]: Interned integer literals with parsed values
//! - [`interner::InternedFloat`]: Interned float literals with parsed values
//! - [`Value`]: Runtime values including functions and macros
//! - [`Type`]: Runtime types as first-class values
//! - [`Env`]: Scoped environment for variable bindings
//! - [`Compiler`]: The compiler state that accumulates definitions
//! - [`eval`]: The main evaluation function

mod compiler;
mod diagnostic;
mod env;
mod eval;
pub mod interner;
mod map;
mod value;

pub use compiler::Compiler;
pub use diagnostic::{Diagnostic, DiagnosticKind, DiagnosticLevel, Result, StackFrame};
// Backwards compatibility aliases
pub use diagnostic::{Error, ErrorKind};
pub use env::Env;
pub use eval::eval;
pub use interner::InternedString;
pub use map::Map;
pub use value::{Type, Value};

#[cfg(test)]
mod tests;
