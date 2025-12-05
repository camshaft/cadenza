//! Example: Execute GCode using the direct AST approach
//!
//! This demonstrates the new architecture where GCode is parsed directly into
//! a Cadenza-compatible AST, which is then evaluated like any Cadenza code.

use cadenza_eval::{BuiltinMacro, Compiler, Env, Type, Value, eval};
use cadenza_gcode::gcode_parse;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let gcode = r#"
G28
G1 X100 Y50
M104 S200
"#;

    // Parse GCode directly into Cadenza AST
    let parse = gcode_parse(gcode);
    let root = parse.ast();

    println!("Parsed {} expressions from GCode", root.items().count());

    // Debug: print the AST
    for (i, item) in root.items().enumerate() {
        println!("Expression {}: {:?}", i + 1, item);
    }

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
                use cadenza_eval::Eval;
                println!("G1: Moving with {} unevaluated parameters", args.len());
                for (i, arg) in args.iter().enumerate() {
                    println!("  Param {} (unevaluated): {:?}", i + 1, arg);
                    // Try to evaluate it
                    if let Ok(val) = arg.eval(&mut ctx.reborrow()) {
                        println!("    Evaluated to: {:?}", val);
                    }
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
                println!("M104: Setting temperature with {} parameters", args.len());
                Ok(Value::Nil)
            },
        }),
    );

    // Register millimeter unit constructor
    compiler.define_var(
        "millimeter".into(),
        Value::UnitConstructor(cadenza_eval::Unit::base("millimeter".into())),
    );

    // Evaluate the GCode AST - eval doesn't care that it came from GCode!
    println!("\nExecuting GCode through Cadenza evaluator:");
    let results = eval(&root, &mut env, &mut compiler);

    println!("\nExecuted {} commands", results.len());
    for (i, result) in results.iter().enumerate() {
        println!("Command {}: {:?}", i + 1, result);
    }

    Ok(())
}
