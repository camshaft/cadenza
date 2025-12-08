# REPL Test Data

This directory contains test data files for REPL snapshot testing.

## File Format

Each `.repl` file contains input that will be piped to the REPL. The file should contain valid Cadenza expressions, one per line, that will be evaluated in sequence.

## How Tests Work

1. Build script (`build.rs`) discovers all `.repl` files in this directory
2. For each file, a test is generated that:
   - Pipes the file contents to the REPL via stdin
   - Captures the REPL's stdout output
   - Creates a snapshot of the output
3. Snapshots are stored in `src/snapshots/` using the `insta` crate

## Adding New Tests

To add a new REPL test:

1. Create a new `.repl` file in this directory with a descriptive name (e.g., `arithmetic-basic.repl`)
2. Add REPL input commands (one per line)
3. Run `cargo test -p cadenza` to generate the snapshot
4. Review the generated snapshot in `src/snapshots/`
5. If the output is correct, commit both the `.repl` file and the snapshot

## Example

File: `arithmetic-basic.repl`
```
1 + 1
2 * 3
10 / 2
```

This will produce a snapshot showing the REPL session with all outputs.
