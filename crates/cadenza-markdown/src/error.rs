//! Error types for Markdown parsing.

use miette::Diagnostic;
use thiserror::Error;

/// Result type for Markdown parsing operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Error types that can occur during Markdown parsing.
#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    /// Generic parsing error
    #[error("Parse error: {0}")]
    Parse(String),
}
