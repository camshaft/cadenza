# Agents

This file provides guidance for AI agents and contributors working on this repository.

## Git Pre-commit Hook

To automatically run rustfmt and clippy before each commit, install the pre-commit hook:

```bash
cargo xtask hooks install
```

This will install the pre-commit hook that runs `cargo xtask precommit` before each commit, ensuring:
- Code is formatted with rustfmt (checks only - use `cargo xtask fmt` to apply formatting)
- Clippy lints pass

You can also run the precommit checks manually:

```bash
cargo xtask precommit
```

To apply rustfmt formatting to all files:

```bash
cargo xtask fmt
```

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

## After Completing Work

After completing work on a task from a crate's status document (e.g., `crates/cadenza-eval/STATUS.md`), update the status document to mark the task as complete. Use strikethrough (`~~`) to mark the task title and add checkmarks (`[x]`) to indicate completed sub-items.
