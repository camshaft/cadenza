//! Markdown parser as alternative Cadenza syntax.
//!
//! This crate treats Markdown as an alternative lexer/parser for Cadenza, producing
//! Cadenza-compatible AST directly that can be evaluated by cadenza-eval.

pub mod error;
pub mod syntax;

#[cfg(test)]
pub mod testing;

mod generated;

pub use error::{Error, Result};
pub use syntax::parse;

#[cfg(test)]
mod fuzz;
