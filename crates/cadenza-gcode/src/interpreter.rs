//! Integrated GCode interpreter using Cadenza's eval system.
//!
//! This module provides a way to execute GCode by treating each GCode command
//! as a macro lookup in the Cadenza compiler. GCode commands like `G1`, `M104`
//! are registered as macros that take parameters and execute accordingly.

use crate::{
    ast::{Command, Line, Parameter, ParameterValue, Program},
    error::{Error, Result},
};
use cadenza_eval::{Compiler, Env, InternedString, Value, eval};
use cadenza_syntax::{ast::Root, parse::parse};

/// Execute a GCode program using the Cadenza evaluator.
///
/// Each GCode command is looked up as a macro in the compiler's scope.
/// The macro receives the parsed parameters and can perform arbitrary
/// Cadenza code execution.
///
/// # Arguments
/// * `program` - The parsed GCode program
/// * `compiler` - The Cadenza compiler with registered GCode handlers
/// * `env` - The environment for variable bindings
///
/// # Returns
/// A vector of Values, one for each executed command (typically state objects).
pub fn execute_gcode(
    program: &Program,
    compiler: &mut Compiler,
    env: &mut Env,
) -> Result<Vec<Value>> {
    let mut results = Vec::new();

    for (line_num, line) in program.lines.iter().enumerate() {
        if let Line::Command(cmd) = line {
            let result = execute_command(cmd, line_num + 1, compiler, env)?;
            results.push(result);
        }
    }

    Ok(results)
}

/// Execute a single GCode command by looking up its macro.
fn execute_command(
    cmd: &Command,
    _line_number: usize,
    compiler: &mut Compiler,
    env: &mut Env,
) -> Result<Value> {
    // Convert the command code to a string identifier for lookup
    let command_str = cmd.code.to_string();
    let command_name: InternedString = command_str.as_str().into();

    // Check if the macro exists
    if compiler.get_macro(command_name).is_none() {
        return Err(Error::UnsupportedCommand(cmd.code.to_string()));
    }

    // Generate Cadenza source code for this command
    let cadenza_code = command_to_cadenza(cmd)?;
    eprintln!("Generated Cadenza code: {}", cadenza_code);

    // Parse the generated code
    let root_syntax = parse(&cadenza_code);
    let root = Root::cast(root_syntax.syntax())
        .ok_or_else(|| Error::TranspilationError("Failed to parse generated code".to_string()))?;

    // Evaluate the code
    let values = eval(&root, env, compiler);

    // Return the last value (or Nil if empty)
    Ok(values.into_iter().last().unwrap_or(Value::Nil))
}

/// Convert a GCode command to Cadenza source code.
///
/// This generates a function call like: `(G1 (100millimeter) (50millimeter))`
/// Each parameter is wrapped in parentheses to prevent left-associative parsing.
fn command_to_cadenza(cmd: &Command) -> Result<String> {
    let mut code = String::from("(");
    code.push_str(&cmd.code.to_string());

    for param in &cmd.parameters {
        code.push_str(" (");
        code.push_str(&param_to_cadenza(param)?);
        code.push(')');
    }

    code.push(')');
    Ok(code)
}

/// Convert a GCode parameter to Cadenza code.
fn param_to_cadenza(param: &Parameter) -> Result<String> {
    match &param.value {
        ParameterValue::Integer(i) => {
            // Add unit annotation for position/extrusion parameters
            let unit = match param.letter {
                'X' | 'Y' | 'Z' | 'E' => "millimeter",
                'F' => "millimeter_per_minute",
                _ => "",
            };

            if unit.is_empty() {
                Ok(i.to_string())
            } else {
                Ok(format!("{}{}", i, unit))
            }
        }
        ParameterValue::Float(f) => {
            // Add unit annotation for position/extrusion parameters
            let unit = match param.letter {
                'X' | 'Y' | 'Z' | 'E' => "millimeter",
                'F' => "millimeter_per_minute",
                _ => "",
            };

            if unit.is_empty() {
                Ok(f.to_string())
            } else {
                Ok(format!("{}{}", f, unit))
            }
        }
        ParameterValue::Flag => Ok("true".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_gcode;
    use cadenza_eval::{BuiltinMacro, Type};

    #[test]
    fn test_execute_simple_gcode() {
        let gcode = "G28\n";
        let program = parse_gcode(gcode).unwrap();

        let mut compiler = Compiler::new();
        let mut env = Env::new();

        // Register a simple G28 macro that returns nil
        let g28_macro = Value::BuiltinMacro(BuiltinMacro {
            name: "G28",
            signature: Type::function(vec![], Type::Nil),
            func: |_args, _ctx| Ok(Value::Nil),
        });
        compiler.define_macro("G28".into(), g28_macro);

        let results = execute_gcode(&program, &mut compiler, &mut env).unwrap();
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0], Value::Nil));
    }

    #[test]
    fn test_command_to_cadenza() {
        use crate::ast::{CommandCode, Parameter, ParameterValue};

        let cmd = Command {
            code: CommandCode::G(1),
            parameters: vec![
                Parameter {
                    letter: 'X',
                    value: ParameterValue::Float(100.0),
                },
                Parameter {
                    letter: 'Y',
                    value: ParameterValue::Float(50.0),
                },
            ],
        };

        let cadenza = command_to_cadenza(&cmd).unwrap();
        assert_eq!(cadenza, "(G1 (100millimeter) (50millimeter))");
    }
}
