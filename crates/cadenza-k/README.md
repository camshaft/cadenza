# Cadenza K Framework Implementation

This directory contains a reference implementation of the Cadenza language using the [K Framework](https://kframework.org/).

## Overview

The K Framework is a rewrite-based executable semantic framework. This implementation provides:

- A formal semantic definition of Cadenza in K
- Executable semantics that can evaluate Cadenza programs
- A test harness that runs all semantics tests through the K implementation

## Structure

```
crates/cadenza-k/
├── cadenza.k           # Main K definition file
├── Makefile            # Build and test automation
├── README.md           # This file
├── STATUS.md           # Implementation progress tracking
└── tests/              # Test harness and status tracking
    └── run-tests.sh    # Script to run all semantics tests
```

## Prerequisites

You need to install the K Framework to use this implementation:

### Using Nix (Recommended)

If you're using the provided Nix flake:

```bash
nix develop
```

### Manual Installation

Follow the [K Framework installation guide](https://github.com/runtimeverification/k?tab=readme-ov-file#quick-start).

On Ubuntu/Debian:

```bash
# Install dependencies
sudo apt-get update
sudo apt-get install build-essential m4 openjdk-17-jdk libmpfr-dev \
    libgmp-dev libjemalloc-dev flex bison z3 libz3-dev maven pkg-config \
    clang

# Download and install K
bash <(curl https://kframework.org/install)
export PATH=$PATH:~/.local/bin
```

## Building

Compile the K definition:

```bash
cd crates/cadenza-k
make
```

This creates a `cadenza-kompiled/` directory with the compiled definition.

## Running Programs

To run a Cadenza program through the K interpreter:

```bash
# Convert a .cdz file to AST format
cargo run --bin cadenza ast path/to/program.cdz > program.ast

# Run through K
make run FILE=program.ast
```

Or use the convenience script:

```bash
./tests/run-single.sh path/to/program.cdz
```

## Running Tests

To run all semantics tests:

```bash
make test
```

This will:
1. Extract all test cases from `docs/semantics/`
2. Convert each `.cdz` file to AST format
3. Run through the K interpreter
4. Compare output with expected results
5. Generate a status report

## Test Status

See [STATUS.md](STATUS.md) for current implementation progress.

## K Definition Structure

The `cadenza.k` file defines:

### Syntax

S-expression syntax for the AST produced by `cadenza ast`:

```k
syntax Expr ::= Int(String)
              | Float(String)
              | String(String)
              | Ident(String)
              | Apply(Expr, Exprs)
              | ...
```

### Configuration

The runtime state structure:

```k
configuration
    <k> $PGM:Expr </k>
    <env> .Map </env>
    <store> .Map </store>
```

### Semantics

Rewrite rules that define evaluation:

```k
// Integer literal evaluates to itself
rule <k> Int(S:String) => parseInt(S) ... </k>

// Variable lookup
rule <k> Ident(X) => V ... </k>
     <env>... X |-> V ...</env>
```

## Implementation Strategy

We're implementing features incrementally in the order defined by the semantics documents:

1. ✅ Basic infrastructure (this setup)
2. 🔄 01-literals.md - Integer, Float, String, Boolean literals
3. ⏸️ 02-variables.md - Let bindings, scope, shadowing
4. ⏸️ 03-operators.md - Arithmetic, comparison, logical operators
5. ⏸️ 04-functions.md - Functions, closures, application
6. ⏸️ 05-macros.md - Macro system
7. ⏸️ 06-compound-types.md - Records, lists, tuples
8. ⏸️ 07-documentation.md - Documentation annotations
9. ⏸️ 08-measures.md - Units of measure

## Contributing

When implementing new features:

1. Add the corresponding K rules to `cadenza.k`
2. Run `make test` to see which tests pass
3. Update `STATUS.md` with current progress
4. Commit with a descriptive message

The goal is not to implement every feature perfectly, but to set up the infrastructure and demonstrate basic functionality.
