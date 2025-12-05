//! GCode parser implementation.

use crate::{
    ast::{Command, CommandCode, Line, Parameter, ParameterValue, Program},
    error::{Error, Result},
};

/// Parse a GCode string into a Program.
///
/// This collects all parsed lines into a Program. For streaming/iterative parsing,
/// use `parse_gcode_lines` instead.
pub fn parse_gcode(input: &str) -> Result<Program> {
    let lines = parse_gcode_lines(input).collect::<Result<Vec<_>>>()?;
    Ok(Program { lines })
}

/// Parse GCode lines as an iterator.
///
/// This allows for streaming/incremental parsing without allocating a Vec upfront.
///
/// # Example
/// ```
/// use cadenza_gcode::parse_gcode_lines;
///
/// let gcode = "G28\nG1 X100\n";
/// for line in parse_gcode_lines(gcode) {
///     let line = line.unwrap();
///     // Process line
/// }
/// ```
pub fn parse_gcode_lines(input: &str) -> impl Iterator<Item = Result<Line>> + '_ {
    input.lines().map(parse_line)
}

/// Parse a single line of GCode.
fn parse_line(line: &str) -> Result<Line> {
    let line = line.trim();

    // Handle empty lines
    if line.is_empty() {
        return Ok(Line::Empty);
    }

    // Handle comments (semicolon or parentheses)
    if let Some(stripped) = line.strip_prefix(';') {
        return Ok(Line::Comment(stripped.trim().to_string()));
    }

    // Handle inline comments - extract comment part
    let (code_part, _comment) = if let Some(pos) = line.find(';') {
        let (code, cmt) = line.split_at(pos);
        (
            code.trim(),
            Some(cmt.strip_prefix(';').unwrap_or(cmt).trim()),
        )
    } else {
        (line, None)
    };

    // If only comment remains after splitting, treat as comment
    if code_part.is_empty() {
        if let Some(cmt) = _comment {
            return Ok(Line::Comment(cmt.to_string()));
        }
        return Ok(Line::Empty);
    }

    // Parse as command
    let command = parse_command(code_part)?;

    // For now, we ignore inline comments during parsing
    // They could be preserved in the AST in a future enhancement
    Ok(Line::Command(command))
}

/// Parse a GCode command.
fn parse_command(input: &str) -> Result<Command> {
    let input = input.trim();
    if input.is_empty() {
        return Err(Error::InvalidCommand("Empty command".to_string()));
    }

    // Split into tokens using iterator
    let mut tokens = input.split_whitespace();
    
    // Parse the command code
    let code_token = tokens
        .next()
        .ok_or_else(|| Error::InvalidCommand("No tokens found".to_string()))?;
    let code = parse_command_code(code_token)?;

    // Parse parameters using iterator
    let parameters = tokens
        .map(parse_parameter)
        .collect::<Result<Vec<_>>>()?;

    Ok(Command { code, parameters })
}

/// Parse a command code (G1, M104, etc.).
fn parse_command_code(token: &str) -> Result<CommandCode> {
    if token.is_empty() {
        return Err(Error::InvalidCommand("Empty command code".to_string()));
    }

    let first_char = token.chars().next().unwrap();
    let number_part = &token[1..];

    match first_char.to_ascii_uppercase() {
        'G' => {
            let num = number_part
                .parse::<u32>()
                .map_err(|_| Error::InvalidCommand(format!("Invalid G-code number: {}", token)))?;
            Ok(CommandCode::G(num))
        }
        'M' => {
            let num = number_part
                .parse::<u32>()
                .map_err(|_| Error::InvalidCommand(format!("Invalid M-code number: {}", token)))?;
            Ok(CommandCode::M(num))
        }
        'T' => {
            let num = number_part
                .parse::<u32>()
                .map_err(|_| Error::InvalidCommand(format!("Invalid T-code number: {}", token)))?;
            Ok(CommandCode::T(num))
        }
        _ => {
            // Allow custom commands for extensibility
            Ok(CommandCode::Custom(token.to_string()))
        }
    }
}

/// Parse a parameter (e.g., "X100", "F3000", or "X" for flags).
fn parse_parameter(token: &str) -> Result<Parameter> {
    if token.is_empty() {
        return Err(Error::InvalidParameter("Empty parameter".to_string()));
    }

    let letter = token.chars().next().unwrap().to_ascii_uppercase();
    let value_str = &token[1..];

    // If there's no value, treat as a flag
    if value_str.is_empty() {
        return Ok(Parameter {
            letter,
            value: ParameterValue::Flag,
        });
    }

    // Try parsing as integer first, then as float
    let value = if let Ok(int_val) = value_str.parse::<i64>() {
        ParameterValue::Integer(int_val)
    } else if let Ok(float_val) = value_str.parse::<f64>() {
        ParameterValue::Float(float_val)
    } else {
        return Err(Error::InvalidParameterValue(
            letter,
            format!("Cannot parse '{}' as number", value_str),
        ));
    };

    Ok(Parameter { letter, value })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_empty_line() {
        let result = parse_line("").unwrap();
        assert_eq!(result, Line::Empty);
    }

    #[test]
    fn test_parse_comment() {
        let result = parse_line("; This is a comment").unwrap();
        assert!(matches!(result, Line::Comment(_)));
    }

    #[test]
    fn test_parse_g1_command() {
        let result = parse_line("G1 X100 Y50 F3000").unwrap();
        if let Line::Command(cmd) = result {
            assert_eq!(cmd.code, CommandCode::G(1));
            assert_eq!(cmd.parameters.len(), 3);
        } else {
            panic!("Expected Command");
        }
    }

    #[test]
    fn test_parse_m104_command() {
        let result = parse_line("M104 S200").unwrap();
        if let Line::Command(cmd) = result {
            assert_eq!(cmd.code, CommandCode::M(104));
            assert_eq!(cmd.parameters.len(), 1);
            assert_eq!(cmd.parameters[0].letter, 'S');
        } else {
            panic!("Expected Command");
        }
    }

    #[test]
    fn test_parse_g28_no_params() {
        let result = parse_line("G28").unwrap();
        if let Line::Command(cmd) = result {
            assert_eq!(cmd.code, CommandCode::G(28));
            assert_eq!(cmd.parameters.len(), 0);
        } else {
            panic!("Expected Command");
        }
    }

    #[test]
    fn test_parse_with_inline_comment() {
        let result = parse_line("G1 X100 ; move to X100").unwrap();
        if let Line::Command(cmd) = result {
            assert_eq!(cmd.code, CommandCode::G(1));
            assert_eq!(cmd.parameters.len(), 1);
        } else {
            panic!("Expected Command");
        }
    }

    #[test]
    fn test_parse_float_parameter() {
        let result = parse_line("G1 X100.5 Y50.25").unwrap();
        if let Line::Command(cmd) = result {
            assert_eq!(cmd.parameters.len(), 2);
            match cmd.parameters[0].value {
                ParameterValue::Float(f) => assert_eq!(f, 100.5),
                _ => panic!("Expected Float"),
            }
        } else {
            panic!("Expected Command");
        }
    }

    #[test]
    fn test_parse_complete_program() {
        let gcode = r#"
; Sample GCode
G28
G1 X100 Y50 F3000
M104 S200
"#;
        let program = parse_gcode(gcode).unwrap();
        assert!(program.lines.len() >= 4); // At least comment + 3 commands
    }

    #[test]
    fn test_parse_flag_parameters() {
        let result = parse_line("G28 X Y Z").unwrap();
        if let Line::Command(cmd) = result {
            assert_eq!(cmd.code, CommandCode::G(28));
            assert_eq!(cmd.parameters.len(), 3);
            assert_eq!(cmd.parameters[0].letter, 'X');
            assert_eq!(cmd.parameters[0].value, ParameterValue::Flag);
            assert_eq!(cmd.parameters[1].letter, 'Y');
            assert_eq!(cmd.parameters[1].value, ParameterValue::Flag);
            assert_eq!(cmd.parameters[2].letter, 'Z');
            assert_eq!(cmd.parameters[2].value, ParameterValue::Flag);
        } else {
            panic!("Expected Command");
        }
    }
}
