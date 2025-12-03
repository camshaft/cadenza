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

## Leave the Codebase Better Than You Found It

**Always strive to keep CI green.** A passing CI build gives everyone confidence that the codebase is in a good state.

### Fix Existing CI Failures

If you encounter failing CI checks (e.g., Clippy warnings, rustfmt issues, failing tests) that are **unrelated to your current task**, fix them as part of your change. Don't ignore them or work around them.

**Why this matters:**
- Broken CI creates a culture of ignoring failures
- Unfixed issues accumulate and become harder to resolve
- Green CI signals that the codebase is healthy and maintainable

### Guidelines

- **Before starting**: Run `cargo xtask ci` to check the current state
- **Fix unrelated issues**: If you see Clippy warnings or formatting issues in files you're working on or nearby, fix them
- **Don't introduce new issues**: Always run CI checks before committing to ensure your changes don't break anything
- **When in doubt**: It's better to fix an extra issue than to leave the codebase worse than you found it

This applies to all types of CI failures:
- Rustfmt formatting issues (`cargo xtask fmt`)
- Clippy lints (`cargo xtask ci clippy`)
- Failing tests (`cargo xtask ci test`)
- Unused dependencies (`cargo xtask ci udeps`)

## After Completing Work

After completing work on a task from a crate's status document (e.g., `crates/cadenza-eval/STATUS.md`), update the status document to mark the task as complete. Use strikethrough (`~~`) to mark the task title and add checkmarks (`[x]`) to indicate completed sub-items.

## PR Title Guidelines

When creating or updating pull requests, use semantic commit conventions in PR titles to clearly communicate the nature of the changes:

```
<type>: <description>
```

### Common Types

- `feat`: A new feature or capability
- `fix`: A bug fix
- `refactor`: Code restructuring without changing behavior
- `docs`: Documentation changes only
- `test`: Adding or updating tests
- `chore`: Maintenance tasks, dependency updates, tooling changes
- `perf`: Performance improvements
- `style`: Code style/formatting changes (not CSS)

### Examples

```
feat: add syntax node tracking to runtime values
fix: resolve type mismatch in comparison operators
refactor: unify macro and special form types
docs: update AGENTS.md with semantic commit guidelines
test: add snapshot tests for error diagnostics
chore: update dependencies to latest versions
```

Use lowercase for the type and description. Keep the description concise and imperative (e.g., "add" not "added" or "adds").

## Test Writing Guidelines

When writing tests, follow these principles to ensure high-quality, maintainable test code:

### What to Test

- **Focus on behavior, not implementation**: Test observable behavior and contracts, not internal implementation details
- **Test complex logic and edge cases**: Prioritize testing non-trivial logic, error handling, and boundary conditions
- **Avoid testing the type system**: Don't write tests for things the compiler already guarantees (e.g., type correctness)
- **Skip trivial constructors/getters/setters**: Simple data structure manipulation rarely needs explicit testing

### How to Test

- **Use property-based testing for patterns**: When you find yourself writing many similar unit tests with slight variations, consider using property-based testing with `bolero` instead. Property tests assert relationships between inputs and outputs across a wide range of values.
  
  Example: Instead of 10 unit tests for different integer inputs, write one property test:
  ```rust
  #[test]
  fn property_addition_commutative() {
      bolero::check!().with_type::<(i64, i64)>().for_each(|(a, b)| {
          assert_eq!(add(a, b), add(b, a));
      });
  }
  ```

- **Make test intent clear**: Each test should have a clear purpose. The test name and structure should communicate what is being tested and why it exists.

- **One or two solid property tests > 100 specific unit tests**: Property tests provide better coverage and catch edge cases you might not think of.

- **Use snapshot testing when appropriate**: Snapshot tests are ideal for:
  - Complex output that would be tedious to check all fields, especially with large nested trees
  - Output that shouldn't change without explicit acknowledgement
  - Source file processing (lex, parse, eval, compile, etc.) - place these in the `test-data` directory
  
  Benefits of using `test-data` directory:
  - Builds a large corpus of files in the language being developed
  - Useful for other purposes like benchmarking
  - Enables syntax highlighting support in the future

### What to Avoid

- **Don't test the same thing repeatedly**: If multiple tests are checking the same behavior with trivial variations, consolidate them or use property testing
- **Don't test trivial wrappers**: If a function just calls another function or wraps a value without logic, it doesn't need a dedicated test
- **Don't assert on internal state**: Test public APIs and observable behavior, not private implementation details

## Snapshot Testing Guidelines

### Critical Rules

- **NEVER commit broken snapshots**: Snapshots with errors in them lock in broken behavior and make it hard to track what's working
- **Snapshots are living documentation**: They serve as a status document showing what currently works
- **Fix errors before accepting snapshots**: If a snapshot shows diagnostic errors, fix the underlying issue before running `INSTA_UPDATE=always`
- **Review snapshot diffs carefully**: Always examine snapshot changes to ensure they represent intended behavior

### When to Update Snapshots

- Update snapshots with `INSTA_UPDATE=always` only after:
  1. Verifying the code changes are correct
  2. Ensuring all diagnostics/errors in output are intentional
  3. Confirming the new behavior is what you want to lock in

### Exception

- During initial development of a feature, temporary diagnostic errors may be acceptable if:
  1. Core functionality works despite the errors
  2. The errors are documented as known issues in STATUS.md
  3. There's a plan to fix them before the feature is considered complete
