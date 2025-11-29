//! Error types for the Cadenza evaluator.
//!
//! Errors are structured with an `ErrorKind` that describes the type of error,
//! wrapped in an `Error` that carries source location and stack trace information.

use crate::interner::InternedId;
use cadenza_syntax::span::Span;
use std::fmt;
use thiserror::Error;

/// Result type for evaluator operations.
pub type Result<T> = std::result::Result<T, Error>;

/// A stack frame in the evaluation stack trace.
#[derive(Debug, Clone)]
pub struct StackFrame {
    /// The name of the function or macro being called, if known.
    pub name: Option<InternedId>,
    /// The source span where this call occurred.
    pub span: Option<Span>,
}

impl StackFrame {
    /// Creates a new stack frame with a name and span.
    pub fn new(name: Option<InternedId>, span: Option<Span>) -> Self {
        Self { name, span }
    }

    /// Creates an anonymous stack frame (e.g., for top-level expressions).
    pub fn anonymous(span: Option<Span>) -> Self {
        Self { name: None, span }
    }
}

/// The kind of error that occurred during evaluation.
#[derive(Debug, Clone, Error)]
pub enum ErrorKind {
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

/// An error that occurred during evaluation, with source location and stack trace.
#[derive(Debug, Clone)]
pub struct Error {
    /// The kind of error that occurred.
    pub kind: ErrorKind,
    /// The source span where the error occurred, if known.
    pub span: Option<Span>,
    /// The evaluation stack trace at the time of the error.
    pub stack_trace: Vec<StackFrame>,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.kind)?;

        if let Some(span) = self.span {
            write!(f, " at {}..{}", span.start, span.end)?;
        }

        if !self.stack_trace.is_empty() {
            writeln!(f)?;
            writeln!(f, "Stack trace:")?;
            for (i, frame) in self.stack_trace.iter().enumerate() {
                write!(f, "  {}: ", i)?;
                if let Some(_name) = frame.name {
                    // Note: Would need interner to resolve name
                    write!(f, "<function>")?;
                } else {
                    write!(f, "<anonymous>")?;
                }
                if let Some(span) = frame.span {
                    write!(f, " at {}..{}", span.start, span.end)?;
                }
                writeln!(f)?;
            }
        }

        Ok(())
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.kind)
    }
}

impl Error {
    /// Creates a new error with a kind and optional span.
    pub fn new(kind: ErrorKind, span: Option<Span>) -> Self {
        Self {
            kind,
            span,
            stack_trace: Vec::new(),
        }
    }

    /// Creates an error with a kind, span, and stack trace.
    pub fn with_stack_trace(
        kind: ErrorKind,
        span: Option<Span>,
        stack_trace: Vec<StackFrame>,
    ) -> Self {
        Self {
            kind,
            span,
            stack_trace,
        }
    }

    /// Adds a stack frame to this error.
    pub fn push_frame(&mut self, frame: StackFrame) {
        self.stack_trace.push(frame);
    }

    /// Sets the span for this error.
    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }

    /// Returns the error kind.
    pub fn kind(&self) -> &ErrorKind {
        &self.kind
    }

    /// Creates an undefined variable error.
    pub fn undefined_variable(name: &str) -> Self {
        Self::new(ErrorKind::UndefinedVariable(name.to_string()), None)
    }

    /// Creates a type error.
    pub fn type_error(expected: impl Into<String>, actual: impl Into<String>) -> Self {
        Self::new(
            ErrorKind::TypeError {
                expected: expected.into(),
                actual: actual.into(),
            },
            None,
        )
    }

    /// Creates an arity error.
    pub fn arity(expected: usize, actual: usize) -> Self {
        Self::new(ErrorKind::ArityError { expected, actual }, None)
    }

    /// Creates a not-callable error.
    pub fn not_callable(value_type: &str) -> Self {
        Self::new(ErrorKind::NotCallable(value_type.to_string()), None)
    }

    /// Creates a syntax error.
    pub fn syntax(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::SyntaxError(msg.into()), None)
    }

    /// Creates an internal error.
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::InternalError(msg.into()), None)
    }

    /// Creates an undefined variable error from an interned ID.
    pub fn undefined_variable_id(id: InternedId, interner: &crate::Interner) -> Self {
        Self::undefined_variable(interner.resolve(id))
    }
}

impl From<ErrorKind> for Error {
    fn from(kind: ErrorKind) -> Self {
        Self::new(kind, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_kind_display() {
        assert_eq!(
            ErrorKind::UndefinedVariable("foo".to_string()).to_string(),
            "undefined variable: foo"
        );
        assert_eq!(
            ErrorKind::TypeError {
                expected: "number".to_string(),
                actual: "string".to_string()
            }
            .to_string(),
            "type error: expected number, got string"
        );
        assert_eq!(
            ErrorKind::ArityError {
                expected: 2,
                actual: 3
            }
            .to_string(),
            "arity error: expected 2 arguments, got 3"
        );
    }

    #[test]
    fn error_with_span() {
        let span = Span::new(10, 20);
        let err = Error::undefined_variable("x").with_span(span);
        assert_eq!(err.span, Some(span));
        assert!(err.to_string().contains("at 10..20"));
    }

    #[test]
    fn error_with_stack_trace() {
        let span1 = Span::new(0, 5);
        let span2 = Span::new(10, 15);

        let mut err = Error::undefined_variable("x");
        err.push_frame(StackFrame::anonymous(Some(span1)));
        err.push_frame(StackFrame::anonymous(Some(span2)));

        assert_eq!(err.stack_trace.len(), 2);
        assert!(err.to_string().contains("Stack trace:"));
    }

    #[test]
    fn error_kind_accessor() {
        let err = Error::type_error("number", "string");
        assert!(matches!(err.kind(), ErrorKind::TypeError { .. }));
    }

    #[test]
    fn error_from_error_kind() {
        let kind = ErrorKind::SyntaxError("test".to_string());
        let err: Error = kind.into();
        assert!(matches!(err.kind, ErrorKind::SyntaxError(_)));
        assert!(err.span.is_none());
        assert!(err.stack_trace.is_empty());
    }

    #[test]
    fn stack_frame_creation() {
        let span = Span::new(5, 10);
        let frame = StackFrame::anonymous(Some(span));
        assert!(frame.name.is_none());
        assert_eq!(frame.span, Some(span));
    }
}
