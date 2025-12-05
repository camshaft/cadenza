//! Abstract Syntax Tree types for GCode.

use std::fmt;

/// A complete GCode program consisting of multiple lines.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub lines: Vec<Line>,
}

/// A single line in a GCode file.
#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    /// A command with optional parameters
    Command(Command),
    /// A comment line
    Comment(String),
    /// An empty line
    Empty,
}

/// A GCode command (G-code, M-code, etc.).
#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    /// The command letter and number (e.g., "G1", "M104")
    pub code: CommandCode,
    /// Parameters for the command
    pub parameters: Vec<Parameter>,
}

/// The type of GCode command.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CommandCode {
    /// G-code (geometric/movement commands)
    G(u32),
    /// M-code (machine/miscellaneous commands)
    M(u32),
    /// T-code (tool selection)
    T(u32),
    /// Custom command (for extensibility)
    Custom(String),
}

impl fmt::Display for CommandCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandCode::G(n) => write!(f, "G{}", n),
            CommandCode::M(n) => write!(f, "M{}", n),
            CommandCode::T(n) => write!(f, "T{}", n),
            CommandCode::Custom(s) => write!(f, "{}", s),
        }
    }
}

/// A parameter for a GCode command.
#[derive(Debug, Clone, PartialEq)]
pub struct Parameter {
    /// The parameter letter (e.g., 'X', 'Y', 'F')
    pub letter: char,
    /// The parameter value
    pub value: ParameterValue,
}

/// The value of a parameter.
#[derive(Debug, Clone, PartialEq)]
pub enum ParameterValue {
    /// Integer value
    Integer(i64),
    /// Floating point value
    Float(f64),
}

impl ParameterValue {
    /// Get the value as a float, converting integers if necessary.
    pub fn as_float(&self) -> f64 {
        match self {
            ParameterValue::Integer(i) => *i as f64,
            ParameterValue::Float(f) => *f,
        }
    }
}

impl fmt::Display for ParameterValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParameterValue::Integer(i) => write!(f, "{}", i),
            ParameterValue::Float(fl) => write!(f, "{}", fl),
        }
    }
}
