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
//! - [`EvalContext`]: Consolidated evaluation context for all eval arguments
//! - [`Eval`]: Trait for evaluatable expressions
//! - [`eval`]: The main evaluation function
//! - [`typeinfer`]: Hindley-Milner type inference

mod compiler;
mod context;
mod diagnostic;
mod env;
mod eval;
mod generated;
pub mod interner;
pub mod ir;
mod map;
pub mod special_form;
pub mod typeinfer;
pub mod unit;
mod value;

pub use compiler::Compiler;
pub use context::{Eval, EvalContext};
pub use diagnostic::{
    BoxedDiagnosticExt, Diagnostic, DiagnosticKind, DiagnosticLevel, Result, StackFrame,
};
// Backwards compatibility aliases
pub use diagnostic::{Error, ErrorKind};
pub use env::Env;
pub use eval::{
    builtin_add, builtin_assign, builtin_div, builtin_eq, builtin_field_access, builtin_fn,
    builtin_gt, builtin_gte, builtin_lt, builtin_lte, builtin_match, builtin_measure, builtin_mul,
    builtin_ne, builtin_pipeline, builtin_record, builtin_sub, eval,
};
pub use interner::InternedString;
pub use map::Map;
pub use special_form::BuiltinSpecialForm;
pub use typeinfer::{Constraint, InferType, Substitution, TypeEnv, TypeInferencer, TypeVar};
pub use unit::{DerivedDimension, Dimension, Unit, UnitRegistry};
pub use value::{BuiltinFn, BuiltinMacro, SourceInfo, TrackedValue, Type, UserFunction, Value};

#[cfg(test)]
mod testing;

#[cfg(test)]
mod tests;
