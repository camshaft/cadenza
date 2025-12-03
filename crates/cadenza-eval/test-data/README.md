# Test Data

This directory contains test data files (`.cdz` extension) used for snapshot testing the Cadenza evaluator.

## File Naming Convention

Files follow a naming pattern based on their purpose:

### Test Files (`<category>-<description>.cdz`)
These files are used for comprehensive snapshot testing. Examples:
- `arith-add.cdz` - Tests addition
- `fn-closure.cdz` - Tests function closures
- `measure-conversion.cdz` - Tests unit conversions

### Example Files (`example-<name>.cdz`)
These files are **displayed in the Compiler Explorer** UI to showcase language features. When adding new language features, create corresponding example files here so users can explore them in the web interface.

Examples:
- `example-welcome.cdz` - Welcome message and basic intro
- `example-arithmetic.cdz` - Showcases arithmetic operations
- `example-functions.cdz` - Demonstrates function definitions and closures
- `example-measures.cdz` - Shows units of measure feature

## Adding New Examples

**For Future Agents:** When implementing new language features:

1. Create comprehensive test files following the `<category>-<description>.cdz` pattern
2. **ALWAYS create a corresponding `example-<feature>.cdz` file** to showcase the feature in the Compiler Explorer
3. Keep examples clear and well-commented to help users learn the language
4. Update this README if new example categories are needed

Example files are automatically discovered by the build system and made available in the web UI's example selector dropdown.

## Build Integration

The `build/test_data.rs` script automatically:
- Loads all `.cdz` files from this directory
- Generates snapshot tests for each file
- Exports example files (with `example-` prefix) for the web UI

Files are processed at build time, so no manual registration is needed.
