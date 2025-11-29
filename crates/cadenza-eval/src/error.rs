//! Error types for the Cadenza evaluator.

use crate::interner::InternedId;
use thiserror::Error;

/// Result type for evaluator operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur during evaluation.
#[derive(Debug, Error)]
pub enum Error {
    /// An undefined variable was referenced.
    #[error("undefined variable: {0}")]
    UndefinedVariable(String),

    /// A value was used in an invalid way for its type.
    #[error("type error: expected {expected}, got {actual}")]
    TypeError { expected: String, actual: String },

    /// Wrong number of arguments to a function or macro.
    #[error("arity error: expected {expected} arguments, got {actual}")]
    ArityError { expected: usize, actual: usize },

    /// A value is not callable (not a function or macro).
    #[error("not callable: {0}")]
    NotCallable(String),

    /// Invalid syntax in AST.
    #[error("syntax error: {0}")]
    SyntaxError(String),

    /// An internal error in the evaluator.
    #[error("internal error: {0}")]
    InternalError(String),
}

impl Error {
    /// Creates an undefined variable error.
    pub fn undefined_variable(name: &str) -> Self {
        Error::UndefinedVariable(name.to_string())
    }

    /// Creates a type error.
    pub fn type_error(expected: impl Into<String>, actual: impl Into<String>) -> Self {
        Error::TypeError {
            expected: expected.into(),
            actual: actual.into(),
        }
    }

    /// Creates an arity error.
    pub fn arity(expected: usize, actual: usize) -> Self {
        Error::ArityError { expected, actual }
    }

    /// Creates a not-callable error.
    pub fn not_callable(value_type: &str) -> Self {
        Error::NotCallable(value_type.to_string())
    }

    /// Creates a syntax error.
    pub fn syntax(msg: impl Into<String>) -> Self {
        Error::SyntaxError(msg.into())
    }

    /// Creates an internal error.
    pub fn internal(msg: impl Into<String>) -> Self {
        Error::InternalError(msg.into())
    }

    /// Creates an undefined variable error from an interned ID.
    pub fn undefined_variable_id(id: InternedId, interner: &crate::Interner) -> Self {
        Error::UndefinedVariable(interner.resolve(id).to_string())
    }
}
