# Status

## Implementation Status

### ✅ Completed

- **Parser**: GCode lexer/parser producing Cadenza-compatible AST via Rowan CST
- **Direct AST Construction**: No string generation or re-parsing
- **Parameter Representation**: `[Letter, value]` structure (e.g., `X100` → `[X, 100]`)
- **Comment Handling**: Comments preserved in CST as trivia
- **Offset Tracking**: Accurate source positions for all tokens
- **Snapshot Tests**: Auto-generated from test-data/*.gcode files
- **Zero Allocations**: Iterator-based parsing without intermediate collections
- **Checksum Support**: Parses and preserves checksums (`*##` suffix) in CST
- **Klipper Format**: Named parameters with `=` syntax (e.g., `PIN=my_led`)

### 🎯 Architecture

GCode is treated as an alternative syntax for Cadenza:
- GCode commands → Apply nodes (function calls)
- Parameters → Apply nodes with letter as receiver
- Flags (no value) → Identifier nodes
- Comments → Comment tokens in CST

Example: `G1 X100 Y50` → `[G1, [X, 100], [Y, 50]]`

Handler macros receive parameter expressions and can:
- Pattern match on parameter names
- Apply units based on command semantics
- Handle optional parameters
- Implement custom logic

### 📋 Known Limitations

1. **Basic GCode Only**: Currently parses simple command + parameter structure
2. ~~**No Checksums**: Doesn't validate or parse checksums (`*##` suffix)~~ ✅ Checksums now parsed and preserved in CST
3. **No Checksum Validation**: Checksums are parsed but not validated
4. **Limited Error Recovery**: Basic error handling, could be more robust
5. **No Macro Expansion**: GCode macros/variables not yet supported

### 🚀 Future Enhancements

1. **Extended GCode Support**:
   - ~~Checksums and validation (`*##` suffix)~~ ✅ Parsing implemented
   - Checksum validation (verify XOR of bytes)
   - ~~Klipper macro format (e.g., `SET_PIN PIN=my_led VALUE=1`)~~ ✅ Implemented

2. **Better Error Messages**:
   - Detailed diagnostic messages
   - Suggestions for common mistakes
   - Context-aware error recovery

3. **Performance**:
   - Streaming parser for large files
   - Incremental re-parsing

4. **Tooling**:
   - Formatter for GCode
   - Linter with configurable rules

## Testing

Tests are auto-generated from `test-data/*.gcode` files via build script.
Snapshots capture the AST structure for validation.

## Vision

This is the first step toward using Cadenza as type-safe 3D printer firmware. See `docs/GCODE_INTERPRETER_ENVIRONMENT.md` for the full vision of dimensional analysis and compile-time safety for CNC control.
