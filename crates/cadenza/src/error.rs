//! Error types for the Cadenza compiler.

use cadenza_syntax::{ast::Error as SyntaxError, source_file::SourceFile, span::Span};
use std::fmt;

/// Compiler error with source information.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Error {
    /// The kind of error
    pub kind: ErrorKind,

    /// Source location
    pub span: Option<Span>,

    /// Source file
    pub source: Option<SourceFile>,
}

/// The kind of compiler error.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// Invalid syntax
    InvalidSyntax(String),

    /// Type error
    TypeError(String),

    /// Ownership error
    OwnershipError(String),

    /// Generic error message
    Message(String),
}

impl Error {
    /// Create an error from a syntax error node
    pub fn invalid_syntax(err: SyntaxError) -> Self {
        Self {
            kind: ErrorKind::InvalidSyntax(err.syntax().text().to_string()),
            span: Some(err.span()),
            source: None,
        }
    }

    /// Create an error with a span
    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }

    /// Create an error with a source file
    pub fn with_source(mut self, source: SourceFile) -> Self {
        self.source = Some(source);
        self
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ErrorKind::InvalidSyntax(msg) => write!(f, "Syntax error: {}", msg),
            ErrorKind::TypeError(msg) => write!(f, "Type error: {}", msg),
            ErrorKind::OwnershipError(msg) => write!(f, "Ownership error: {}", msg),
            ErrorKind::Message(msg) => write!(f, "{}", msg),
        }
    }
}

impl From<String> for Error {
    fn from(msg: String) -> Self {
        Self {
            kind: ErrorKind::Message(msg),
            span: None,
            source: None,
        }
    }
}

impl From<&str> for Error {
    fn from(msg: &str) -> Self {
        Self {
            kind: ErrorKind::Message(msg.to_string()),
            span: None,
            source: None,
        }
    }
}
