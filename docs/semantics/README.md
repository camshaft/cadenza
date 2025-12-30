# Cadenza Language Semantics

This directory contains the **executable specification** of the Cadenza language.

Each feature is documented in a single markdown file that includes:

1. **Explanation** - What the feature does and why
2. **Examples** - Inline code examples showing usage
3. **Tests** - Executable test cases with expected output

## Structure

```
docs/semantics/
├── 01-literals.md        # Integer, float, string, boolean literals
├── 02-variables.md       # Let bindings, scope, shadowing
├── 03-operators.md       # Arithmetic, comparison, logical operators
├── 04-functions.md       # Definition, application, closures
├── 05-control-flow.md    # Match expressions, blocks, conditionals
├── 06-collections.md     # Lists, records, tuples
├── 07-types.md           # Type system, typeof, structs
├── 08-measures.md        # Units of measure, dimensional analysis
├── 09-special.md         # Assertions, field access, special forms
└── README.md             # This file
```

## Markdown Format

Each feature document uses a standard format with embedded test cases:

````markdown
# Feature Name

Description of what this feature does.

## Syntax

How to write it.

## Examples

Inline examples showing usage (not tests).

## Test: Description

Test cases use this format:

**Input:**

```cadenza
code here
```
````

**Output:**

```
expected output
```

Multiple tests can be included in each document.

````

## Running Tests

### Extract Tests
```console
$ cargo xtask semantics extract
````

This generates:

- `test-data/semantics/*.cdz` - Test input files
- `test-data/semantics/*.expected` - Expected output files

### Run Tests and Update Progress Report

```bash
$ cargo xtask semantics report
```

## Example Feature Document

Here's what `01-literals.md` looks like:

```markdown
# Integer Literals

Integers are whole numbers without decimal points.

## Syntax
```

digit+

````

Underscores for readability: `1_000_000`

## Test: Basic integer

**Input:**

```cadenza
42
````

**Output:**

```
42 : Integer
```

## Test: Integer with underscores

**Input:**

```cadenza
1_000_000
```

**Output:**

```
1_000_000 : Integer
```

````

## Benefits

1. **Single source of truth** - One file per feature
2. **Readable** - Markdown is easy to read and write
3. **Executable** - Tests can be extracted and run
4. **Organized** - Numbered files show progression
5. **Versioned** - Git tracks all changes
6. **Documented** - Explanation and tests together
7. **Trackable** - Status markers show progress

## Workflow

### Adding a new feature

1. Create `NN-feature-name.md`
2. Write documentation and tests
3. Extract tests
4. Run tests
5. Update status report based on results
6. Commit

### Migrating from test-data

The existing `crates/cadenza-eval/test-data/` will be gradually documented here.
Each test file gets:
- A markdown document explaining it
- Test status tracking
- Connection to meta-compiler queries

## Meta-Compiler Integration

Each feature document should note which queries are needed:

```markdown
## Meta-Compiler Queries

This feature requires:
- `eval(expr, env)` - Evaluation semantics
- `type_of(expr)` - Type inference
````

This helps track what needs to be implemented in `crates/cadenza-compiler/build/main.rs`.
