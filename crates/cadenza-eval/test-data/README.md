# Test Data

This directory contains test data files (`.cdz` extension) for the Cadenza evaluator.

## File Types

- **Test files** (`<category>-<description>.cdz`): Comprehensive test cases (e.g., `arith-add.cdz`, `fn-closure.cdz`)
- **Example files** (`example-##-name.cdz`): Language examples displayed in the Compiler Explorer UI

## Adding New Examples

When implementing new language features, create example files to showcase them in the Compiler Explorer.

See the **"Adding Examples to Compiler Explorer"** section in `/AGENTS.md` for detailed instructions.

## Build Integration

The build script automatically:
- Generates snapshot tests for all `.cdz` files
- Generates TypeScript code for `example-*.cdz` files
- Symlinks the generated examples to the web app
