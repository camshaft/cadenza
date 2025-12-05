//! Trait-based command handler system for extensible GCode transpilation.

use crate::{
    ast::{Command, Parameter},
    error::Result,
};

/// Information about a GCode command being transpiled.
#[derive(Debug, Clone)]
pub struct CommandInfo<'a> {
    /// The parsed command
    pub command: &'a Command,
    /// Line number in the source file (1-indexed)
    pub line_number: usize,
    /// Optional source file name
    pub source_file: Option<&'a str>,
}

/// A trait for transpiling GCode commands to Cadenza code.
///
/// Implementations can define custom transpilation logic for specific commands,
/// allowing for flexible parameter handling, validation, and code generation.
///
/// # Example
///
/// ```rust
/// use cadenza_gcode::handler::{CommandHandler, CommandInfo};
/// use cadenza_gcode::{ast::Parameter, error::Result};
///
/// struct CustomG1Handler;
///
/// impl CommandHandler for CustomG1Handler {
///     fn transpile(&self, info: &CommandInfo<'_>) -> Result<String> {
///         // Custom logic for G1 commands
///         let mut output = "handle_g1 state".to_string();
///         for param in &info.command.parameters {
///             // Custom parameter handling
///             output.push_str(&format!(" {}", param.value.as_float()));
///         }
///         Ok(output)
///     }
/// }
/// ```
pub trait CommandHandler {
    /// Transpile a GCode command to Cadenza code.
    ///
    /// # Arguments
    /// * `info` - Information about the command being transpiled
    ///
    /// # Returns
    /// The transpiled Cadenza code as a string, or an error if transpilation fails.
    fn transpile(&self, info: &CommandInfo<'_>) -> Result<String>;
}

/// A default handler that uses parameter-to-unit mapping.
pub struct DefaultHandler {
    /// The name of the Cadenza function to call
    pub handler_name: String,
    /// The state variable name
    pub state_var: String,
}

impl DefaultHandler {
    /// Create a new default handler for a command.
    pub fn new(handler_name: impl Into<String>) -> Self {
        Self {
            handler_name: handler_name.into(),
            state_var: "state".to_string(),
        }
    }

    /// Set the state variable name.
    pub fn with_state_var(mut self, state_var: impl Into<String>) -> Self {
        self.state_var = state_var.into();
        self
    }
}

impl CommandHandler for DefaultHandler {
    fn transpile(&self, info: &CommandInfo<'_>) -> Result<String> {
        let mut call = format!("{} {}", self.handler_name, self.state_var);

        // Add parameters
        for param in &info.command.parameters {
            call.push(' ');
            call.push_str(&transpile_parameter(param)?);
        }

        Ok(call)
    }
}

/// Transpile a parameter with default unit logic.
fn transpile_parameter(param: &Parameter) -> Result<String> {
    use crate::ast::ParameterValue;

    // Handle flag parameters
    if matches!(param.value, ParameterValue::Flag) {
        return Ok("true".to_string());
    }

    // Determine the appropriate unit based on the parameter letter
    let unit = match param.letter {
        'X' | 'Y' | 'Z' | 'E' => "millimeter",
        'F' => "millimeter_per_minute",
        'S' => "",
        _ => "",
    };

    let value = param.value.as_float();

    if unit.is_empty() {
        Ok(format!("{}", value))
    } else {
        Ok(format!("{}{}", value, unit))
    }
}
