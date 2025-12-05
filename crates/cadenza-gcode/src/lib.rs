//! GCode parser as alternative Cadenza syntax.
//!
//! This crate treats GCode as an alternative lexer/parser for Cadenza, producing
//! Cadenza-compatible AST directly that can be evaluated by cadenza-eval.
//!
//! # Example
//!
//! ```rust
//! use cadenza_gcode::parse;
//! use cadenza_eval::{eval, Compiler, Env};
//!
//! let gcode = "G28\nG1 X100 Y50\n";
//! let parse_result = parse(gcode);
//! let root = parse_result.ast();
//!
//! let mut compiler = Compiler::new();
//! let mut env = Env::new();
//! // Register GCode command macros...
//! let results = eval(&root, &mut env, &mut compiler);
//! ```

pub mod error;
pub mod syntax;

mod generated;

pub use error::{Error, Result};
pub use syntax::parse;
