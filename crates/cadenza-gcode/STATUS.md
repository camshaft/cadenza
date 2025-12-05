# cadenza-gcode Status

## Current Status: Initial Implementation Complete ✅

The `cadenza-gcode` crate provides full GCode parsing and transpilation to Cadenza source code.

## Completed Features

### Core Functionality
- ✅ GCode Parser
  - Supports G-codes, M-codes, T-codes
  - Handles comments (semicolon-style)
  - Handles empty lines
  - Supports inline comments
  - Supports flag parameters (e.g., `G28 X Y`)
  - Extensible to custom command types
  
- ✅ GCode to Cadenza Transpiler
  - Generates Cadenza function calls
  - Adds unit annotations (millimeter, millimeter_per_minute)
  - Preserves comments
  - Configurable command-to-handler mappings
  - Support for custom handlers

### Supported Commands

**G-codes (Motion)**:
- G0/G1: Linear/rapid move
- G28: Home axes
- G90: Absolute positioning
- G91: Relative positioning
- G92: Set position

**M-codes (Machine)**:
- M82: E absolute positioning
- M83: E relative positioning
- M104: Set extruder temperature (non-blocking)
- M109: Set extruder temperature (blocking)
- M106: Fan on
- M107: Fan off
- M140: Set bed temperature (non-blocking)
- M190: Set bed temperature (blocking)

### Testing
- ✅ 15 unit tests for parser
- ✅ 6 unit tests for transpiler
- ✅ 3 integration tests
- ✅ 2 snapshot tests with real GCode files
- ✅ All tests passing
- ✅ Code formatted and clippy clean

### Documentation
- ✅ README with usage guide
- ✅ Inline documentation for all public APIs
- ✅ Example: basic transpilation
- ✅ Example: custom handler registration

## Known Limitations

### Parser
- ⚠️ Does not support parenthesis-style comments `(comment)`
- ⚠️ Does not support no-space parameter format `G1X100Y50` (some slicers use this)
- ⚠️ No checksum validation for commands
- ⚠️ No line number tracking

### Transpiler
- ⚠️ Feedrate conversion (mm/min → mm/s) not implemented
- ⚠️ No parameter validation against command requirements
- ⚠️ No optimization of redundant commands
- ⚠️ State variable name is hardcoded as "state"

### General
- ⚠️ No streaming/incremental parsing support
- ⚠️ No direct interpretation mode (only transpilation)
- ⚠️ Limited error reporting (no line numbers in errors)

## Future Work

### Near-term Enhancements
- [ ] Add parenthesis-style comment support: `(comment)`
- [ ] Improve parameter format handling: `G1X100Y50`
- [ ] Add line number tracking for better error messages
- [ ] Add parameter validation (required vs optional)
- [ ] Implement feedrate unit conversion

### Medium-term Goals
- [ ] Add more GCode dialects (Marlin-specific, RepRapFirmware)
- [ ] Command optimization (remove redundant commands)
- [ ] Configurable state variable naming
- [ ] Better error messages with source location
- [ ] Add checksum validation support

### Long-term Vision
- [ ] Direct interpretation mode (parse and execute without transpilation)
- [ ] Streaming parser for large files
- [ ] Integration with Cadenza type system for validation
- [ ] Handler signature validation at transpile time
- [ ] Generate typed Cadenza modules from GCode
- [ ] Performance profiling and optimization

## Integration with Cadenza Vision

This crate is the first step toward the larger vision documented in `docs/GCODE_INTERPRETER_ENVIRONMENT.md`:

1. **Phase 1 (Current)**: GCode parser and transpiler ✅
2. **Phase 2**: Type-checked handler definitions
3. **Phase 3**: Configuration modules with dimensional analysis
4. **Phase 4**: Runtime execution with effect system
5. **Phase 5**: AOT compilation and optimization

See the vision document for the complete roadmap toward using Cadenza as 3D printer firmware.

## Contributing

When adding new features:
1. Add tests (unit, integration, or snapshot as appropriate)
2. Update this STATUS.md
3. Update README.md if public API changes
4. Ensure `cargo xtask ci` passes

### Adding New Commands

To add support for a new GCode command:

1. Add the command mapping in `transpiler.rs` `TranspilerConfig::default()`
2. Add test cases in the appropriate test file
3. Update the README's "Supported Commands" section
4. Update this STATUS.md
