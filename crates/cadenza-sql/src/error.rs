//! Error types for SQL parsing.

use miette::Diagnostic;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("Parse error: {message}")]
    Parse { message: String },
}
