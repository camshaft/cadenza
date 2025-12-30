//! Cadenza Meta-Compiler
//!
//! This crate provides a framework for defining compiler semantics declaratively
//! using Rust data structures, which are then analyzed and compiled into efficient
//! query-based implementations.
//!
//! # Overview
//!
//! The meta-compiler operates in three stages:
//! 1. **Definition**: Construct semantic rules as Rust data structures
//! 2. **Analysis**: Validate and optimize the rule definitions
//! 3. **Generation**: Produce efficient Rust code implementing the queries
//!
//! # Example
//!
//! ```ignore
//! use cadenza_meta::*;
//!
//! let semantics = Semantics::new()
//!     .add_query(
//!         query("eval")
//!             .input(node_id())
//!             .output(value_type())
//!             .rule(
//!                 integer(capture("val"))
//!                     .then(construct("Value::Integer", [var("val")]))
//!             )
//!     );
//! ```

mod analysis;
mod bindings;
mod builders;
mod codegen;
mod tree;
mod types;

pub use analysis::*;
pub use bindings::*;
pub use builders::*;
pub use codegen::*;
pub use tree::*;
pub use types::*;
