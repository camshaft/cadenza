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
2. **No Checksums**: Doesn't validate or parse checksums (`*##` suffix)
3. **Limited Error Recovery**: Basic error handling, could be more robust
4. **No Macro Expansion**: GCode macros/variables not yet supported

### 🚀 Future Enhancements

1. **Extended GCode Support**:
   - Checksums and validation
   - Variable substitution
   - Conditional execution
   - Looping constructs

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
   - Language server protocol support

## Testing

Tests are auto-generated from `test-data/*.gcode` files via build script.
Snapshots capture the AST structure for validation.

## Vision

This is the first step toward using Cadenza as type-safe 3D printer firmware. See `docs/GCODE_INTERPRETER_ENVIRONMENT.md` for the full vision of dimensional analysis and compile-time safety for CNC control.
