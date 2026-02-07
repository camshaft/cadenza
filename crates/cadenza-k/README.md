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

## What's Implemented

### ✅ Complete

1. **AST CLI Tool** - The `cadenza ast` command converts `.cdz` files to S-expressions
   - Supports all expression types: literals, identifiers, applications, operators
   - Clean S-expression output that K can parse without ambiguity
   - Example: `cadenza ast file.cdz` outputs `(Int "42")`

2. **K Framework Structure**
   - Basic K definition file (`cadenza.k`) with syntax and configuration
   - Literal evaluation rules for integers, floats, strings, booleans
   - Configuration with k cell, env cell, store cell
   - Makefile for building and testing

3. **Test Infrastructure**
   - Test runner script (`tests/run-tests.sh`)
   - Integration with `cargo xtask semantics extract`
   - Single test runner for development (`tests/run-single.sh`)

4. **CI Integration**
   - GitHub Actions workflow for K framework testing
   - K framework installation steps
   - Automated test execution on PRs

### 🔄 In Progress

- K definition compilation (needs K framework installed in CI)
- Test execution and output comparison
- More complete semantic rules

### 📋 Next Steps

1. Verify K framework compiles in CI
2. Add more semantic rules incrementally
3. Implement output comparison in test runner
4. Track test pass rates in STATUS.md

## Current Status

The infrastructure is complete and ready to use. The K definition will need refinement
as it's tested with actual K framework installation. See [STATUS.md](STATUS.md) for
detailed progress tracking.
