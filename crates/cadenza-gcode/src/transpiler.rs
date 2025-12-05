//! GCode to Cadenza transpiler.

use crate::{
    ast::{CommandCode, Line, Program},
    error::{Error, Result},
    handler::{CommandHandler, CommandInfo, DefaultHandler},
};
use std::collections::HashMap;

/// Configuration for transpilation.
pub struct TranspilerConfig {
    /// Mapping from command codes to handler implementations
    handlers: HashMap<CommandCode, Box<dyn CommandHandler + Send + Sync>>,
    /// Whether to include comments in the output
    pub include_comments: bool,
    /// Source file name for error reporting
    pub source_file: Option<String>,
}

// Manual Debug impl since CommandHandler doesn't implement Debug
impl std::fmt::Debug for TranspilerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TranspilerConfig")
            .field("handlers", &format!("{} handlers", self.handlers.len()))
            .field("include_comments", &self.include_comments)
            .field("source_file", &self.source_file)
            .finish()
    }
}

impl Default for TranspilerConfig {
    fn default() -> Self {
        let mut config = Self {
            handlers: HashMap::new(),
            include_comments: true,
            source_file: None,
        };

        // Register common RepRap commands with default handlers
        config.register_default_handler(CommandCode::G(0), "handle_g0");
        config.register_default_handler(CommandCode::G(1), "handle_g1");
        config.register_default_handler(CommandCode::G(28), "handle_g28");
        config.register_default_handler(CommandCode::G(90), "handle_g90");
        config.register_default_handler(CommandCode::G(91), "handle_g91");
        config.register_default_handler(CommandCode::G(92), "handle_g92");
        config.register_default_handler(CommandCode::M(82), "handle_m82");
        config.register_default_handler(CommandCode::M(83), "handle_m83");
        config.register_default_handler(CommandCode::M(104), "handle_m104");
        config.register_default_handler(CommandCode::M(106), "handle_m106");
        config.register_default_handler(CommandCode::M(107), "handle_m107");
        config.register_default_handler(CommandCode::M(109), "handle_m109");
        config.register_default_handler(CommandCode::M(140), "handle_m140");
        config.register_default_handler(CommandCode::M(190), "handle_m190");

        config
    }
}

impl TranspilerConfig {
    /// Register a custom command handler.
    ///
    /// This is a convenience method for backward compatibility.
    /// It creates a DefaultHandler with the given function name.
    pub fn register_handler(&mut self, code: CommandCode, handler_name: String) {
        self.register_default_handler(code, handler_name);
    }

    /// Register a handler using the default parameter transpilation logic.
    pub fn register_default_handler(&mut self, code: CommandCode, handler_name: impl Into<String>) {
        let handler = DefaultHandler::new(handler_name);
        self.handlers.insert(
            code,
            Box::new(handler) as Box<dyn CommandHandler + Send + Sync>,
        );
    }

    /// Register a custom command handler implementation.
    pub fn register_custom_handler(
        &mut self,
        code: CommandCode,
        handler: impl CommandHandler + Send + Sync + 'static,
    ) {
        self.handlers.insert(
            code,
            Box::new(handler) as Box<dyn CommandHandler + Send + Sync>,
        );
    }

    /// Set the source file name for error reporting.
    pub fn with_source_file(mut self, file: impl Into<String>) -> Self {
        self.source_file = Some(file.into());
        self
    }
}

/// Transpile a GCode program to Cadenza source code.
pub fn transpile_to_cadenza(program: &Program) -> Result<String> {
    transpile_with_config(program, &TranspilerConfig::default())
}

/// Transpile a GCode program to Cadenza source code with custom configuration.
pub fn transpile_with_config(program: &Program, config: &TranspilerConfig) -> Result<String> {
    let mut output = String::new();

    // Add header comment
    output.push_str("# Generated from GCode\n\n");

    for (line_num, line) in program.lines.iter().enumerate() {
        match line {
            Line::Command(cmd) => {
                let info = CommandInfo {
                    command: cmd,
                    line_number: line_num + 1,
                    source_file: config.source_file.as_deref(),
                };
                let cadenza_line = transpile_command(&info, config)?;
                output.push_str(&cadenza_line);
                output.push('\n');
            }
            Line::Comment(text) => {
                if config.include_comments {
                    output.push_str(&format!("# {}\n", text));
                }
            }
            Line::Empty => {
                if config.include_comments {
                    output.push('\n');
                }
            }
        }
    }

    Ok(output)
}

/// Transpile a single command using the configured handler.
fn transpile_command(info: &CommandInfo<'_>, config: &TranspilerConfig) -> Result<String> {
    // Look up the handler for this command
    let handler = config
        .handlers
        .get(&info.command.code)
        .ok_or_else(|| Error::UnsupportedCommand(info.command.code.to_string()))?;

    handler.transpile(info)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_gcode;

    #[test]
    fn test_transpile_g28() {
        let gcode = "G28\n";
        let program = parse_gcode(gcode).unwrap();
        let cadenza = transpile_to_cadenza(&program).unwrap();
        assert!(cadenza.contains("handle_g28 state"));
    }

    #[test]
    fn test_transpile_g1_with_params() {
        let gcode = "G1 X100 Y50 F3000\n";
        let program = parse_gcode(gcode).unwrap();
        let cadenza = transpile_to_cadenza(&program).unwrap();
        assert!(cadenza.contains("handle_g1 state"));
        assert!(cadenza.contains("100millimeter"));
        assert!(cadenza.contains("50millimeter"));
    }

    #[test]
    fn test_transpile_m104() {
        let gcode = "M104 S200\n";
        let program = parse_gcode(gcode).unwrap();
        let cadenza = transpile_to_cadenza(&program).unwrap();
        assert!(cadenza.contains("handle_m104 state"));
        assert!(cadenza.contains("200"));
    }

    #[test]
    fn test_transpile_with_comments() {
        let gcode = r#"
; Home all axes
G28
; Move to position
G1 X100 Y50
"#;
        let program = parse_gcode(gcode).unwrap();
        let cadenza = transpile_to_cadenza(&program).unwrap();
        assert!(cadenza.contains("# Home all axes"));
        assert!(cadenza.contains("handle_g28"));
        assert!(cadenza.contains("# Move to position"));
        assert!(cadenza.contains("handle_g1"));
    }

    #[test]
    fn test_custom_handler() {
        let gcode = "G29\n"; // Bed leveling
        let program = parse_gcode(gcode).unwrap();

        let mut config = TranspilerConfig::default();
        config.register_handler(CommandCode::G(29), "handle_bed_level".to_string());

        let cadenza = transpile_with_config(&program, &config).unwrap();
        assert!(cadenza.contains("handle_bed_level state"));
    }

    #[test]
    fn test_unsupported_command_error() {
        let gcode = "G999\n"; // Unknown command
        let program = parse_gcode(gcode).unwrap();
        let result = transpile_to_cadenza(&program);
        assert!(result.is_err());
    }
}
