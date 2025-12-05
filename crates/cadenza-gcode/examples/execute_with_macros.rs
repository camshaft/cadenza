//! Example: Execute GCode using Cadenza's macro system
//!
//! This demonstrates the integrated approach where GCode commands are
//! registered as Cadenza macros and executed through the evaluator.

use cadenza_eval::{BuiltinMacro, Compiler, Env, Eval, Type, Value};
use cadenza_gcode::{execute_gcode, parse_gcode};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let gcode = r#"
; Example GCode
G28           ; Home all axes
G1 X100 Y50   ; Move to position
M104 S200     ; Set temperature
"#;

    // Parse GCode
    let program = parse_gcode(gcode)?;

    println!("Parsed {} lines", program.lines.len());
    for (i, line) in program.lines.iter().enumerate() {
        println!("Line {}: {:?}", i + 1, line);
    }
    println!();

    // Create Cadenza compiler and environment
    let mut compiler = Compiler::new();
    let mut env = Env::new();

    // Register G28 (home) macro
    compiler.define_macro(
        "G28".into(),
        Value::BuiltinMacro(BuiltinMacro {
            name: "G28",
            signature: Type::function(vec![], Type::Nil),
            func: |_args, _ctx| {
                println!("Homing all axes...");
                Ok(Value::Nil)
            },
        }),
    );

    // Register G1 (move) macro
    compiler.define_macro(
        "G1".into(),
        Value::BuiltinMacro(BuiltinMacro {
            name: "G1",
            signature: Type::function(vec![Type::Unknown, Type::Unknown], Type::Nil),
            func: |args, ctx| {
                println!("G1 macro invoked with {} args:", args.len());
                for (i, arg) in args.iter().enumerate() {
                    println!("  Arg {}: {:?}", i, arg);
                    // Try to evaluate it
                    let val = arg.eval(&mut ctx.reborrow()).ok();
                    println!("    Evaluated: {:?}", val);
                }
                Ok(Value::Nil)
            },
        }),
    );

    // Register M104 (set temp) macro
    compiler.define_macro(
        "M104".into(),
        Value::BuiltinMacro(BuiltinMacro {
            name: "M104",
            signature: Type::function(vec![Type::Unknown], Type::Nil),
            func: |args, _ctx| {
                println!("Setting temperature: {} params", args.len());
                Ok(Value::Nil)
            },
        }),
    );

    // Execute the GCode through Cadenza's evaluator
    let results = execute_gcode(&program, &mut compiler, &mut env)?;

    println!("\nExecuted {} commands successfully", results.len());
    for (i, result) in results.iter().enumerate() {
        println!("Command {}: {:?}", i + 1, result);
    }

    Ok(())
}
