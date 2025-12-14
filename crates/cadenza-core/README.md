# cadenza-core

HIR-based incremental compilation system for Cadenza using [Salsa](https://github.com/salsa-rs/salsa).

## Overview

`cadenza-core` provides the core compiler infrastructure for Cadenza, implementing a High-level Intermediate Representation (HIR) based compilation pipeline with incremental computation.

### Key Features

- **HIR-First**: Compilation works on a simplified, desugared representation
- **Span Tracking**: Every node preserves source location information
- **Incremental**: Salsa-based dependency tracking and memoization
- **Post-Expansion LSP**: IDE features operate on macro-expanded code
- **Pure Functions**: All queries are deterministic and cacheable

## Architecture

```text
Source → Parse (CST) → Lower (HIR) → Eval/Expand (HIR) → Type Check → LSP/Codegen
```

1. **Parse**: Source code → Concrete Syntax Tree
2. **Lower**: AST → HIR (desugaring, span preservation)
3. **Eval**: HIR → Expanded HIR (macro expansion, compile-time eval)
4. **Type Check**: Infer types on expanded HIR
5. **Queries**: LSP features and code generation on typed, expanded HIR

See [`docs/ARCHITECTURE.md`](./docs/ARCHITECTURE.md) for detailed architecture documentation.

## Status

### Implemented

- ✅ Salsa database infrastructure
- ✅ Source file tracking
- ✅ Parsing with diagnostics
- ✅ HIR definition (expressions, literals, patterns)

### TODO

- ⬜ AST → HIR lowering
- ⬜ HIR evaluation/macro expansion
- ⬜ Type inference on HIR
- ⬜ LSP queries (hover, completion, etc.)

## Usage

```rust
use cadenza_core::{CadenzaDbImpl, SourceFile, parse_file};

// Create database
let db = CadenzaDbImpl::default();

// Add source file
let source = SourceFile::new(&db, "test.cdz".to_string(), "let x = 1".to_string());

// Parse (automatically cached by Salsa)
let parsed = parse_file(&db, source);
let cst = parsed.cst(&db);

// Check for errors
let diagnostics = parse_file::accumulated::<Diagnostic>(&db, source);
```

## Comparison with cadenza-eval

`cadenza-eval` was a POC that worked directly on the AST. `cadenza-core` is the production implementation that:

- Uses HIR instead of working on raw AST
- Preserves spans throughout compilation
- Has LSP operate on post-expansion code
- Implements full Salsa pipeline

## Development

```bash
# Build
cargo build -p cadenza-core

# Test
cargo test -p cadenza-core

# Documentation
cargo doc -p cadenza-core --open
```

## License

MIT
