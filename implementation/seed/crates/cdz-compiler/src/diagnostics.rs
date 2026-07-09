//! Machine-readable diagnostics: the coded-span-record schema.
//!
//! Every diagnostic carries a stable machine-readable code, a precise source span, and
//! the rule it enforces (constitution §XI; options/diagnostics-schema/coded-span-record.md).
//! The seed, being dynamic, emits only the two front-end rejections that need no static
//! typing — an unbound name (CDZ0101) and an undeclared capability (CDZ0401) — which every
//! generation makes. The typed rejections (CDZ0201/0202/0203/0210/0301) are a later
//! generation's; the seed instead traps at runtime where such programs have a defined
//! dynamic outcome.
//!
//! (diagnostics.md is NOT in the ignition requirement subset, so these citations are not
//! gate-load-bearing for the seed; they document the rule each rejection enforces, which
//! constitution §XI requires of the diagnostic itself.)

use std::fmt;

/// A stable diagnostic code, `CDZ` + four digits (options/diagnostics-schema/).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Code {
    /// Reference to a name with no enclosing binding.
    UnboundName,
    /// A program that reaches a host operation its manifest does not enumerate.
    UndeclaredCapability,
}

impl Code {
    pub fn as_str(&self) -> &'static str {
        match self {
            Code::UnboundName => "CDZ0101",
            Code::UndeclaredCapability => "CDZ0401",
        }
    }
    /// The rule reference `<spec-file>#<section-slug>` the diagnostic enforces.
    pub fn rule(&self) -> &'static str {
        match self {
            Code::UnboundName => "spec/capabilities/core-semantics.md#binding-is-lexical",
            Code::UndeclaredCapability => {
                "spec/capabilities/capabilities-and-effects.md#undeclared-capability-is-a-compile-time-error"
            }
        }
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A source span, by line/column and byte offset (options/diagnostics-schema/). The seed
/// works on the canonical AST, which carries no source text, so a synthesized span is
/// used; the code and rule are the load-bearing machine-actionable fields.
#[derive(Clone, Debug, Default)]
pub struct Span {
    pub path: String,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Severity {
    Error,
    Warning,
    Note,
    Help,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Note => "note",
            Severity::Help => "help",
        }
    }
}

/// A diagnostic record: stable code, severity, span, message, and the rule it enforces.
#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub code: Code,
    pub severity: Severity,
    pub span: Span,
    pub message: String,
}

impl Diagnostic {
    pub fn error(code: Code, message: impl Into<String>) -> Diagnostic {
        Diagnostic { code, severity: Severity::Error, span: Span::default(), message: message.into() }
    }

    /// Machine-readable JSON projection of the coded-span-record.
    pub fn to_json(&self) -> String {
        format!(
            "{{\"code\":\"{}\",\"severity\":\"{}\",\"message\":{:?},\"rule\":\"{}\"}}",
            self.code.as_str(),
            self.severity.as_str(),
            self.message,
            self.code.rule()
        )
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} [{}]: {} ({})", self.severity.as_str(), self.code, self.message, self.code.rule())
    }
}
