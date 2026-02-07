# Cadenza K Framework Implementation

This directory contains a reference implementation of the Cadenza language using the [K Framework](https://kframework.org/).

## Overview

The K Framework is a rewrite-based executable semantic framework. This implementation provides:

- A formal semantic definition of Cadenza in K
- Executable semantics that can evaluate Cadenza programs
- A test harness that runs all semantics tests through the K implementation

## Prerequisites

You need to install the K Framework to use this implementation.

### Using Nix (Recommended)

```bash
nix develop
```

This will set up a development shell with K framework and all dependencies.

### Manual Installation

Follow the [K Framework installation guide](https://github.com/runtimeverification/k?tab=readme-ov-file#quick-start).

## Usage

All K framework operations are available through xtask commands:

### Compile K Definition

```bash
cargo xtask k kompile
```

This compiles `cadenza.k` and outputs to `target/k/cadenza-kompiled/`.

### Run All Tests

```bash
cargo xtask k test
```

This will:
1. Extract all test cases from `docs/semantics/`
2. Convert each `.cdz` file to AST format
3. Run through the K interpreter
4. Generate a status report

### Run Single File

```bash
cargo xtask k run path/to/file.cdz
```

This converts the file to AST and runs it through K.

## Test Status

See [STATUS.md](STATUS.md) for current implementation progress.
