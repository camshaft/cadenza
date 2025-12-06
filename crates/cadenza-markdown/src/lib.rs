//! Markdown parser as alternative Cadenza syntax.
//!
//! This crate treats Markdown as an alternative lexer/parser for Cadenza, producing
//! Cadenza-compatible AST directly that can be evaluated by cadenza-eval.
//!
//! # Example
//!
//! ```rust
//! use cadenza_markdown::parse;
//! use cadenza_eval::{eval, Compiler, Env};
//!
//! let markdown = "# Hello\n\nWorld!";
//! let parse_result = parse(markdown);
//! let root = parse_result.ast();
//!
//! let mut compiler = Compiler::new();
//! let mut env = Env::new();
//! // Register Markdown element macros...
//! let results = eval(&root, &mut env, &mut compiler);
//! ```

pub mod error;
pub mod syntax;

#[cfg(test)]
pub mod testing;

mod generated;

pub use error::{Error, Result};
pub use syntax::parse;
