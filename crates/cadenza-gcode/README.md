# cadenza-gcode

GCode parser and transpiler for the Cadenza programming language.

## Overview

This crate provides functionality to parse GCode files (primarily RepRap/Marlin flavor) and transpile them to Cadenza source code. The transpiled code can then be parsed, type-checked, and executed using Cadenza's interpreter.

## Features

- **GCode Parser**: Parses standard GCode commands (G-codes, M-codes, T-codes)
- **Extensible**: Support for custom commands via registration
- **Transpilation**: Converts GCode to idiomatic Cadenza function calls
- **Units Support**: Automatically adds appropriate unit annotations (millimeter, etc.)
- **Comment Preservation**: Preserves comments from GCode as Cadenza comments

## Usage

### Basic Parsing

```rust
use cadenza_gcode::{parse_gcode, transpile_to_cadenza};

let gcode = r#"
G28              ; Home all axes
M104 S200        ; Set extruder temp
G1 X100 Y50 F3000  ; Move to position
"#;

let program = parse_gcode(gcode)?;
let cadenza_code = transpile_to_cadenza(&program)?;
println!("{}", cadenza_code);
```

Output:
```cadenza
# Generated from GCode

# Home all axes
handle_g28 state
# Set extruder temp
handle_m104 state 200
# Move to position
handle_g1 state 100millimeter 50millimeter 3000millimeter_per_minute
```

### Custom Command Handlers

```rust
use cadenza_gcode::{transpile_with_config, TranspilerConfig, CommandCode};

let mut config = TranspilerConfig::default();
config.register_handler(
    CommandCode::G(29),
    "handle_bed_leveling".to_string()
);

let cadenza_code = transpile_with_config(&program, &config)?;
```

## Supported Commands

The transpiler includes built-in support for common RepRap commands:

### G-codes (Motion)
- G0/G1: Linear move
- G28: Home axes
- G90: Absolute positioning
- G91: Relative positioning
- G92: Set position

### M-codes (Machine)
- M104/M109: Set extruder temperature
- M140/M190: Set bed temperature
- M106/M107: Fan control
- M82/M83: Extruder positioning mode

Additional commands can be registered as needed.

## Architecture

The crate is organized into several modules:

- `ast`: Abstract Syntax Tree types for representing GCode
- `parser`: GCode parsing logic
- `transpiler`: GCode to Cadenza transpilation
- `error`: Error types and diagnostics

## Vision

This crate is part of a larger vision to use Cadenza as a firmware platform for 3D printers, similar to Klipper but with the benefits of:

- **Type Safety**: Compile-time validation of printer configurations
- **Dimensional Analysis**: Automatic checking of units (mm, mm/s, etc.)
- **Ahead-of-Time Compilation**: Native code generation for performance
- **Modern Language Features**: Closures, pattern matching, strong typing

See `docs/GCODE_INTERPRETER_ENVIRONMENT.md` in the main repository for more details.

## Future Enhancements

- [ ] Support for more GCode dialects (Marlin-specific, RepRapFirmware, etc.)
- [ ] Validation against handler signatures
- [ ] Optimization of transpiled code
- [ ] Direct interpretation mode (without transpilation)
- [ ] Streaming support for large files
- [ ] Better error messages with line numbers
