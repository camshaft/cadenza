# cadenza-gcode

GCode parser as alternative Cadenza syntax.

This crate treats GCode as an alternative lexer/parser for Cadenza, producing Cadenza-compatible AST directly that can be evaluated by `cadenza-eval`. GCode commands become function calls, and parameters become nested Apply nodes with the parameter letter as the receiver.

## Features

- **Direct AST Construction**: Builds `cadenza_syntax::ast::Root` from GCode using Rowan CST
- **Zero String Generation**: No intermediate Cadenza code generation or re-parsing
- **Proper Offset Tracking**: Every byte accounted for with accurate source positions
- **Comment Preservation**: Comments are included in the CST as trivia
- **Non-Positional Parameters**: Parameters represented as `[Letter, value]` for flexible handling

## Architecture

GCode is parsed directly into Cadenza's AST format:
- **GCode commands** → Apply nodes (e.g., `G1` becomes a function call)
- **Parameters** → Apply nodes with letter as receiver (e.g., `X100` → `[X, 100]`)
- **Flags** → Identifier nodes (e.g., `X` without value → `X` identifier)
- **Comments** → Comment tokens in CST

## Example

```rust
use cadenza_gcode::parse;
use cadenza_eval::{eval, BuiltinMacro, Compiler, Env, Type, Value};

let gcode = "G28\nG1 X100 Y50 F3000\n";

// Parse GCode into Cadenza AST
let parse_result = parse(gcode);
let root = parse_result.ast();

let mut compiler = Compiler::new();
let mut env = Env::new();

// Register G1 macro
compiler.define_macro("G1".into(), Value::BuiltinMacro(BuiltinMacro {
    name: "G1",
    signature: Type::function(vec![Type::Unknown, Type::Unknown, Type::Unknown], Type::Nil),
    func: |args, ctx| {
        // Args are [X, 100], [Y, 50], [F, 3000]
        // Handler can pattern match on parameter names and apply units
        Ok(Value::Nil)
    },
}));

// Register parameter letter macros (X, Y, F, etc.)
for letter in &["X", "Y", "Z", "E", "F", "S"] {
    compiler.define_macro((*letter).into(), Value::BuiltinMacro(BuiltinMacro {
        name: letter,
        signature: Type::function(vec![Type::Unknown], Type::Unknown),
        func: |args, ctx| {
            // Apply units based on context (e.g., X/Y/Z → millimeter, F → millimeter_per_minute)
            Ok(Value::Nil)
        },
    }));
}

// Evaluate - eval doesn't care this came from GCode!
let results = eval(&root, &mut env, &mut compiler);
```

### Input and Output

Input GCode:
```gcode
G28              ; Home all axes
G1 X100 Y50 F3000
M104 S200
```

Parsed AST:
```
[G28]
[G1, [X, 100], [Y, 50], [F, 3000]]
[M104, [S, 200]]
```

Handler macros receive unevaluated parameter expressions and can:
- Pattern match on parameter names (X, Y, Z, F, S, etc.)
- Apply appropriate units based on command context
- Handle missing/optional parameters
- Implement custom command semantics

## Benefits

1. **Simpler Architecture**: Direct AST construction, no string manipulation
2. **Flexible Semantics**: Handler macros control all parameter interpretation
3. **Non-Positional Parameters**: Letter-value pairs enable robust handling
4. **Natural Integration**: Full access to Cadenza's type system and dimensional analysis
5. **Better Errors**: Stack traces point to original GCode source locations

## Vision

This is the first step toward using Cadenza as type-safe 3D printer firmware, providing dimensional analysis and compile-time safety for CNC control. See `docs/GCODE_INTERPRETER_ENVIRONMENT.md` for the full vision.
