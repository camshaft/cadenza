//! GCode to Cadenza transpiler.

use crate::{
    ast::{Command, CommandCode, Line, Parameter, ParameterValue, Program},
    error::{Error, Result},
};
use std::collections::HashMap;

/// Configuration for transpilation.
#[derive(Debug, Clone)]
pub struct TranspilerConfig {
    /// Mapping from command codes to Cadenza function names
    pub command_handlers: HashMap<CommandCode, String>,
    /// Whether to include comments in the output
    pub include_comments: bool,
    /// State variable name (default: "state")
    pub state_var: String,
}

impl Default for TranspilerConfig {
    fn default() -> Self {
        let mut command_handlers = HashMap::new();

        // Register common RepRap commands
        // G-codes
        command_handlers.insert(CommandCode::G(0), "handle_g0".to_string()); // Rapid move
        command_handlers.insert(CommandCode::G(1), "handle_g1".to_string()); // Linear move
        command_handlers.insert(CommandCode::G(28), "handle_g28".to_string()); // Home
        command_handlers.insert(CommandCode::G(90), "handle_g90".to_string()); // Absolute positioning
        command_handlers.insert(CommandCode::G(91), "handle_g91".to_string()); // Relative positioning
        command_handlers.insert(CommandCode::G(92), "handle_g92".to_string()); // Set position

        // M-codes
        command_handlers.insert(CommandCode::M(104), "handle_m104".to_string()); // Set extruder temp
        command_handlers.insert(CommandCode::M(109), "handle_m109".to_string()); // Set extruder temp and wait
        command_handlers.insert(CommandCode::M(140), "handle_m140".to_string()); // Set bed temp
        command_handlers.insert(CommandCode::M(190), "handle_m190".to_string()); // Set bed temp and wait
        command_handlers.insert(CommandCode::M(106), "handle_m106".to_string()); // Fan on
        command_handlers.insert(CommandCode::M(107), "handle_m107".to_string()); // Fan off
        command_handlers.insert(CommandCode::M(82), "handle_m82".to_string()); // E absolute
        command_handlers.insert(CommandCode::M(83), "handle_m83".to_string()); // E relative

        Self {
            command_handlers,
            include_comments: true,
            state_var: "state".to_string(),
        }
    }
}

impl TranspilerConfig {
    /// Register a custom command handler.
    pub fn register_handler(&mut self, code: CommandCode, handler: String) {
        self.command_handlers.insert(code, handler);
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

    for line in &program.lines {
        match line {
            Line::Command(cmd) => {
                let cadenza_line = transpile_command(cmd, config)?;
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

/// Transpile a single command to a Cadenza function call.
fn transpile_command(cmd: &Command, config: &TranspilerConfig) -> Result<String> {
    // Look up the handler for this command
    let handler = config
        .command_handlers
        .get(&cmd.code)
        .ok_or_else(|| Error::UnsupportedCommand(cmd.code.to_string()))?;

    // Build the function call
    let mut call = format!("{} {}", handler, config.state_var);

    // Add parameters
    for param in &cmd.parameters {
        let param_str = transpile_parameter(param)?;
        call.push(' ');
        call.push_str(&param_str);
    }

    Ok(call)
}

/// Transpile a parameter to Cadenza syntax.
fn transpile_parameter(param: &Parameter) -> Result<String> {
    // Handle flag parameters - just use true as a boolean
    if matches!(param.value, ParameterValue::Flag) {
        return Ok("true".to_string());
    }

    // Determine the appropriate unit based on the parameter letter
    let unit = match param.letter {
        'X' | 'Y' | 'Z' | 'E' => "millimeter",
        'F' => {
            // Feedrate in GCode is typically in mm/min
            // We keep it as millimeter_per_minute for now
            // In the future, this could be converted to mm/s if needed
            "millimeter_per_minute"
        }
        'S' => {
            // S parameter is context-dependent (temperature, speed, etc.)
            // For temperatures, it's dimensionless (degrees)
            // For now, no unit
            ""
        }
        _ => "",
    };

    let value = param.value.as_float();

    if unit.is_empty() {
        Ok(format!("{}", value))
    } else {
        Ok(format!("{}{}", value, unit))
    }
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
