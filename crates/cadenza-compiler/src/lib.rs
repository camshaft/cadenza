//! Cadenza Compiler - The canonical definition of Cadenza language semantics.
//!
//! This crate defines the complete semantics of the Cadenza programming language
//! using the cadenza-meta framework. The semantic definitions are compiled at
//! build time into efficient query-based implementations.

use cadenza_tree::InternedString;
use std::sync::Arc;

pub mod prelude;

mod generated;

// Re-export generated queries
pub use generated::semantics::*;

/// Database trait for query implementation
pub trait Database {
    // Queries will be added here as they're implemented
}

/// Value type for Cadenza expressions
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Integer(i128),
    Float(f64),
    String(Arc<str>),
    Bool(bool),
    Symbol(InternedString),
    Apply {
        callee: Box<Value>,
        args: Vec<Value>,
    },
    Function {
        params: Vec<InternedString>,
        body: NodeId,
    },
    Tuple(Vec<Value>),
    List(Vec<Value>),
    Record(Vec<(InternedString, Value)>),
}

/// Type representation for Cadenza
#[derive(Clone, Debug, PartialEq)]
pub enum Type {
    Integer,
    // More variants will be added as semantics expand
}

/// NodeId from the syntax tree (placeholder for now)
pub type NodeId = u32;

/// Diagnostic information
#[derive(Clone, Debug)]
pub struct Diagnostics {
    pub errors: Vec<String>,
}
