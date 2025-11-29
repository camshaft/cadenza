# Agents

This file provides guidance for AI agents and contributors working on this repository.

## Before Submitting a Commit

Before submitting a commit, ensure that the CI checks pass by running:

```bash
cargo xtask ci
```

This command will run all CI checks including:

- `cargo xtask ci fmt` - Check code formatting (automatically installs nightly rustfmt for import sorting)
- `cargo xtask ci clippy` - Run clippy lints
- `cargo xtask ci udeps` - Check for unused dependencies (automatically installs nightly and cargo-udeps if needed)
- `cargo xtask ci test` - Run the test suite

You can also run individual checks by specifying the subcommand, for example:

```bash
cargo xtask ci fmt
cargo xtask ci clippy
```

The test command supports passing additional arguments to cargo test:

```bash
cargo xtask ci test --no-default-features
```
