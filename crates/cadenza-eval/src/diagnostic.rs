//! Diagnostic types for the Cadenza evaluator.
//!
//! Diagnostics are structured with a `DiagnosticKind` that describes the issue,
//! wrapped in a `Diagnostic` that carries severity, source location, and stack trace.
//! Uses miette for standardized diagnostic reporting.

use crate::interner::InternedString;
use cadenza_syntax::span::Span;
use miette::{Diagnostic as MietteDiagnostic, Severity};
use std::fmt;
use thiserror::Error;

/// Result type for evaluator operations.
pub type Result<T> = std::result::Result<T, Diagnostic>;

/// The severity level of a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiagnosticLevel {
    /// An error that prevents compilation or execution.
    #[default]
    Error,
    /// A warning that doesn't prevent compilation but indicates a potential issue.
    Warning,
    /// An informational hint or suggestion.
    Hint,
}

impl From<DiagnosticLevel> for Severity {
    fn from(level: DiagnosticLevel) -> Self {
        match level {
            DiagnosticLevel::Error => Severity::Error,
            DiagnosticLevel::Warning => Severity::Warning,
            DiagnosticLevel::Hint => Severity::Advice,
        }
    }
}

/// A stack frame in the evaluation stack trace.
#[derive(Debug, Clone)]
pub struct StackFrame {
    /// The name of the function or macro being called, if known.
    pub name: Option<InternedString>,
    /// The source file where this call occurred, if known.
    pub file: Option<InternedString>,
    /// The source span where this call occurred.
    pub span: Option<Span>,
}

impl StackFrame {
    /// Creates a new stack frame with a name, file, and span.
    pub fn new(
        name: Option<InternedString>,
        file: Option<InternedString>,
        span: Option<Span>,
    ) -> Self {
        Self { name, file, span }
    }

    /// Creates an anonymous stack frame (e.g., for top-level expressions).
    pub fn anonymous(span: Option<Span>) -> Self {
        Self {
            name: None,
            file: None,
            span,
        }
    }

    /// Creates a stack frame with a file but no function name.
    pub fn in_file(file: InternedString, span: Option<Span>) -> Self {
        Self {
            name: None,
            file: Some(file),
            span,
        }
    }
}

/// The kind of diagnostic that occurred during evaluation.
#[derive(Debug, Clone, Error)]
pub enum DiagnosticKind {
    /// An undefined variable was referenced.
    #[error("undefined variable: {0}")]
    UndefinedVariable(InternedString),

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

    /// A parse error from the parser.
    #[error("parse error: {0}")]
    ParseError(String),

    /// An internal error in the evaluator.
    #[error("internal error: {0}")]
    InternalError(String),
}

/// A diagnostic message with source location and stack trace.
///
/// This is the primary type for reporting issues during evaluation.
/// It supports multiple severity levels (error, warning, hint) and
/// integrates with miette for rich diagnostic output.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    /// The kind of diagnostic.
    pub kind: DiagnosticKind,
    /// The severity level of this diagnostic.
    pub level: DiagnosticLevel,
    /// The source file where the diagnostic occurred, if known.
    pub file: Option<InternedString>,
    /// The source span where the diagnostic occurred, if known.
    pub span: Option<Span>,
    /// The evaluation stack trace at the time of the diagnostic.
    pub stack_trace: Vec<StackFrame>,
}

impl std::error::Error for Diagnostic {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.kind)
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Write severity prefix
        match self.level {
            DiagnosticLevel::Error => write!(f, "error: ")?,
            DiagnosticLevel::Warning => write!(f, "warning: ")?,
            DiagnosticLevel::Hint => write!(f, "hint: ")?,
        }

        // Write the diagnostic kind with resolved names
        match &self.kind {
            DiagnosticKind::UndefinedVariable(id) => {
                write!(f, "undefined variable: {}", &**id)?;
            }
            // For other kinds, use default Display
            kind => write!(f, "{}", kind)?,
        }

        // Write location info
        if let Some(file) = &self.file {
            write!(f, " in {}", &**file)?;
        }
        if let Some(span) = self.span {
            write!(f, " at {}..{}", span.start, span.end)?;
        }

        // Write stack trace
        if !self.stack_trace.is_empty() {
            writeln!(f)?;
            writeln!(f, "Stack trace:")?;
            for (i, frame) in self.stack_trace.iter().enumerate() {
                write!(f, "  {}: ", i)?;
                if let Some(name) = &frame.name {
                    write!(f, "{}", &**name)?;
                } else {
                    write!(f, "<anonymous>")?;
                }
                if let Some(file) = &frame.file {
                    write!(f, " in {}", &**file)?;
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

impl MietteDiagnostic for Diagnostic {
    fn severity(&self) -> Option<Severity> {
        Some(self.level.into())
    }

    fn code<'a>(&'a self) -> Option<Box<dyn fmt::Display + 'a>> {
        let code = match &self.kind {
            DiagnosticKind::UndefinedVariable(_) => "E0001",
            DiagnosticKind::TypeError { .. } => "E0002",
            DiagnosticKind::ArityError { .. } => "E0003",
            DiagnosticKind::NotCallable(_) => "E0004",
            DiagnosticKind::SyntaxError(_) => "E0005",
            DiagnosticKind::ParseError(_) => "E0007",
            DiagnosticKind::InternalError(_) => "E0006",
        };
        Some(Box::new(code))
    }
}

impl Diagnostic {
    /// Creates a new error diagnostic with a kind and optional span.
    pub fn new(kind: DiagnosticKind, span: Option<Span>) -> Self {
        Self {
            kind,
            level: DiagnosticLevel::Error,
            file: None,
            span,
            stack_trace: Vec::new(),
        }
    }

    /// Creates a diagnostic with a specific level.
    pub fn with_level(kind: DiagnosticKind, level: DiagnosticLevel) -> Self {
        Self {
            kind,
            level,
            file: None,
            span: None,
            stack_trace: Vec::new(),
        }
    }

    /// Creates a diagnostic with a kind, span, and stack trace.
    pub fn with_stack_trace(
        kind: DiagnosticKind,
        span: Option<Span>,
        stack_trace: Vec<StackFrame>,
    ) -> Self {
        Self {
            kind,
            level: DiagnosticLevel::Error,
            file: None,
            span,
            stack_trace,
        }
    }

    /// Adds a stack frame to this diagnostic.
    pub fn push_frame(&mut self, frame: StackFrame) {
        self.stack_trace.push(frame);
    }

    /// Sets the span for this diagnostic.
    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }

    /// Sets the source file for this diagnostic.
    pub fn with_file(mut self, file: InternedString) -> Self {
        self.file = Some(file);
        self
    }

    /// Sets the severity level for this diagnostic.
    pub fn set_level(mut self, level: DiagnosticLevel) -> Self {
        self.level = level;
        self
    }

    /// Returns the diagnostic kind.
    pub fn kind(&self) -> &DiagnosticKind {
        &self.kind
    }

    /// Returns true if this is an error-level diagnostic.
    pub fn is_error(&self) -> bool {
        self.level == DiagnosticLevel::Error
    }

    /// Returns true if this is a warning-level diagnostic.
    pub fn is_warning(&self) -> bool {
        self.level == DiagnosticLevel::Warning
    }

    /// Returns true if this is a hint-level diagnostic.
    pub fn is_hint(&self) -> bool {
        self.level == DiagnosticLevel::Hint
    }

    /// Creates an undefined variable error from an interned ID.
    pub fn undefined_variable(id: InternedString) -> Self {
        Self::new(DiagnosticKind::UndefinedVariable(id), None)
    }

    /// Creates a type error.
    pub fn type_error(expected: impl Into<String>, actual: impl Into<String>) -> Self {
        Self::new(
            DiagnosticKind::TypeError {
                expected: expected.into(),
                actual: actual.into(),
            },
            None,
        )
    }

    /// Creates an arity error.
    pub fn arity(expected: usize, actual: usize) -> Self {
        Self::new(DiagnosticKind::ArityError { expected, actual }, None)
    }

    /// Creates a not-callable error.
    pub fn not_callable(value_type: &str) -> Self {
        Self::new(DiagnosticKind::NotCallable(value_type.to_string()), None)
    }

    /// Creates a syntax error.
    pub fn syntax(msg: impl Into<String>) -> Self {
        Self::new(DiagnosticKind::SyntaxError(msg.into()), None)
    }

    /// Creates a parse error from the parser.
    pub fn parse_error(msg: impl Into<String>, span: Span) -> Self {
        Self::new(DiagnosticKind::ParseError(msg.into()), Some(span))
    }

    /// Creates an internal error.
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::new(DiagnosticKind::InternalError(msg.into()), None)
    }
}

impl From<DiagnosticKind> for Diagnostic {
    fn from(kind: DiagnosticKind) -> Self {
        Self::new(kind, None)
    }
}

impl From<cadenza_syntax::parse::ParseError> for Diagnostic {
    fn from(err: cadenza_syntax::parse::ParseError) -> Self {
        Self::parse_error(err.message, err.span)
    }
}

// Type aliases for backwards compatibility
/// Alias for Diagnostic (for backwards compatibility).
pub type Error = Diagnostic;
/// Alias for DiagnosticKind (for backwards compatibility).
pub type ErrorKind = DiagnosticKind;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_kind_display() {
        let foo_id: InternedString = "foo".into();

        // Note: Without the interner, the display shows the raw ID
        // The display_with_interned_string method is used to get the resolved name
        let kind = DiagnosticKind::UndefinedVariable(foo_id);
        let display = format!("{}", kind);
        assert!(display.starts_with("undefined variable:"));

        assert_eq!(
            DiagnosticKind::TypeError {
                expected: "number".to_string(),
                actual: "string".to_string()
            }
            .to_string(),
            "type error: expected number, got string"
        );
        assert_eq!(
            DiagnosticKind::ArityError {
                expected: 2,
                actual: 3
            }
            .to_string(),
            "arity error: expected 2 arguments, got 3"
        );
    }

    #[test]
    fn diagnostic_with_span() {
        let x_id: InternedString = "x".into();
        let span = Span::new(10, 20);
        let diag = Diagnostic::undefined_variable(x_id).with_span(span);
        assert_eq!(diag.span, Some(span));
    }

    #[test]
    fn diagnostic_with_file() {
        let x_id: InternedString = "x".into();
        let file_id: InternedString = "test.cadenza".into();

        let diag = Diagnostic::undefined_variable(x_id).with_file(file_id);
        assert_eq!(diag.file, Some(file_id));
    }

    #[test]
    fn diagnostic_with_stack_trace() {
        let x_id: InternedString = "x".into();
        let span1 = Span::new(0, 5);
        let span2 = Span::new(10, 15);

        let mut diag = Diagnostic::undefined_variable(x_id);
        diag.push_frame(StackFrame::anonymous(Some(span1)));
        diag.push_frame(StackFrame::anonymous(Some(span2)));

        assert_eq!(diag.stack_trace.len(), 2);
    }

    #[test]
    fn stack_frame_with_file() {
        let file_id: InternedString = "test.cadenza".into();
        let func_id: InternedString = "my_function".into();
        let span = Span::new(10, 20);

        let frame = StackFrame::new(Some(func_id), Some(file_id), Some(span));
        assert_eq!(frame.name, Some(func_id));
        assert_eq!(frame.file, Some(file_id));
        assert_eq!(frame.span, Some(span));
    }

    #[test]
    fn diagnostic_kind_accessor() {
        let diag = Diagnostic::type_error("number", "string");
        assert!(matches!(diag.kind(), DiagnosticKind::TypeError { .. }));
    }

    #[test]
    fn diagnostic_from_kind() {
        let kind = DiagnosticKind::SyntaxError("test".to_string());
        let diag: Diagnostic = kind.into();
        assert!(matches!(diag.kind, DiagnosticKind::SyntaxError(_)));
        assert!(diag.span.is_none());
        assert!(diag.stack_trace.is_empty());
    }

    #[test]
    fn stack_frame_creation() {
        let span = Span::new(5, 10);
        let frame = StackFrame::anonymous(Some(span));
        assert!(frame.name.is_none());
        assert!(frame.file.is_none());
        assert_eq!(frame.span, Some(span));
    }

    #[test]
    fn diagnostic_levels() {
        let x_id: InternedString = "x".into();

        let error = Diagnostic::undefined_variable(x_id);
        assert!(error.is_error());
        assert!(!error.is_warning());

        let warning = Diagnostic::undefined_variable(x_id).set_level(DiagnosticLevel::Warning);
        assert!(warning.is_warning());
        assert!(!warning.is_error());

        let hint = Diagnostic::undefined_variable(x_id).set_level(DiagnosticLevel::Hint);
        assert!(hint.is_hint());
    }

    #[test]
    fn diagnostic_display() {
        let x_id: InternedString = "x".into();
        let func_name: InternedString = "my_function".into();
        let file_name: InternedString = "test.cadenza".into();

        let span = Span::new(10, 20);
        let mut diag = Diagnostic::undefined_variable(x_id)
            .with_span(span)
            .with_file(file_name);
        diag.push_frame(StackFrame::new(
            Some(func_name),
            Some(file_name),
            Some(span),
        ));
        diag.push_frame(StackFrame::anonymous(Some(Span::new(0, 5))));

        let display = diag.to_string();
        assert!(display.contains("undefined variable: x"));
        assert!(display.contains("my_function"));
        assert!(display.contains("test.cadenza"));
        assert!(display.contains("<anonymous>"));
        assert!(display.contains("Stack trace:"));
    }

    #[test]
    fn miette_diagnostic_impl() {
        use miette::Diagnostic as _;

        let x_id: InternedString = "x".into();

        let diag = Diagnostic::undefined_variable(x_id);
        assert_eq!(diag.severity(), Some(Severity::Error));
        assert!(diag.code().is_some());

        let warning = Diagnostic::undefined_variable(x_id).set_level(DiagnosticLevel::Warning);
        assert_eq!(warning.severity(), Some(Severity::Warning));
    }
}
