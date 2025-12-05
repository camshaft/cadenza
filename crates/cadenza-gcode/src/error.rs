//! Error types for GCode parsing and transpilation.

use miette::Diagnostic;
use thiserror::Error;

/// Result type for GCode operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur during GCode parsing and transpilation.
#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    /// Invalid command syntax
    #[error("Invalid command syntax: {0}")]
    InvalidCommand(String),

    /// Invalid parameter syntax
    #[error("Invalid parameter syntax: {0}")]
    InvalidParameter(String),

    /// Unsupported command
    #[error("Unsupported command: {0}")]
    UnsupportedCommand(String),

    /// Missing required parameter
    #[error("Missing required parameter '{0}' for command {1}")]
    MissingParameter(char, String),

    /// Invalid parameter value
    #[error("Invalid parameter value for '{0}': {1}")]
    InvalidParameterValue(char, String),

    /// Transpilation error
    #[error("Transpilation error: {0}")]
    TranspilationError(String),
}
