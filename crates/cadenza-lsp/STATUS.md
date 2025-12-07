# Cadenza LSP - Status Document

This document tracks the current state of the `cadenza-lsp` crate and remaining LSP implementation work.

## Overview

The LSP crate implements Language Server Protocol support for Cadenza that can be used both natively (via tower-lsp) and in the browser (via WASM). It provides shared utilities for position/offset conversion and diagnostic generation.

## Current State

### ✅ Completed

**Core Utilities:**
- `offset_to_position()` - Converts byte offset to LSP Position (line, character)
- `position_to_offset()` - Converts LSP Position to byte offset
- `parse_to_diagnostics()` - Converts Cadenza parse errors to LSP diagnostics
- Re-exports lsp_types for consumers

**Native LSP Server (cadenza CLI):**
- Full tower-lsp backend implementation
- Document synchronization (full document sync)
- Real-time diagnostics on document open/change
- Hover provider (basic symbol identification)
- Completion provider (basic keyword completions: `let`, `fn`)
- Stdio transport for editor integration

**WASM LSP (cadenza-web):**
- `lsp_diagnostics()` - Export diagnostics for Monaco
- `lsp_hover()` - Export hover information for Monaco
- `lsp_completions()` - Export completions for Monaco
- TypeScript bindings and types

**Monaco Integration:**
- Cadenza language registration
- Syntax highlighting (keywords, operators, strings, numbers)
- Real-time diagnostic markers
- Hover tooltips
- Autocomplete suggestions

### ⚠️ Known Issues & Shortcuts

**Performance Concerns:**
1. **Position/Offset Conversion (CRITICAL)** - `offset_to_position()` and `position_to_offset()` functions are O(n) with document size. These are called frequently by LSP operations (every diagnostic, hover, completion). For large files, this will become a bottleneck.
   - **Impact**: Runs on every diagnostic (multiple per document change), every hover, every completion request
   - **Mitigation needed**: Consider:
     - Caching line start offsets in a LineIndex structure
     - Only recompute when document changes
     - Store line index per document in the LSP backend

2. **Monaco Language Configuration** - Keywords and operators are hardcoded in TypeScript instead of being generated from Rust code. This creates maintenance burden and risk of drift.
   - **Current**: Manual lists in `monaco-cadenza.ts`
   - **Needed**: Generate from Rust special form registry and operator definitions
   - **See**: Comments on lines 16, 42, 46 in `monaco-cadenza.ts`

3. **Comment Syntax** - Monaco configuration has incorrect comment syntax
   - **Current**: Claims to support `//` line comments and `/* */` block comments
   - **Actual**: Cadenza only supports `#` line comments
   - **Status**: Needs correction

### ❌ Not Implemented

**LSP Features:**
- Go to definition
- Find references
- Rename symbol
- Document symbols / outline
- Workspace symbols
- Code actions / quick fixes
- Formatting
- Signature help
- Semantic tokens / semantic highlighting
- Incremental document sync (currently full sync only)
- Code lens
- Folding ranges
- Selection ranges
- Document links
- Color provider
- Inlay hints

**Diagnostics:**
- Type errors (only parse errors currently)
- Unused variable warnings
- Unreachable code warnings
- Custom lint rules

**Completions:**
- Context-aware completions (only basic keywords now)
- Completions from scope (variables, functions)
- Member completions (record fields, methods)
- Snippet completions
- Import completions
- Documentation in completion items

**Hover:**
- Type information (currently just identifies symbol)
- Documentation strings
- Function signatures
- Value previews for constants

**Configuration:**
- LSP configuration options
- Workspace settings
- Per-file settings

### 🔄 Refactoring Needed

1. **Position/Offset Conversion** - Replace naive linear search with LineIndex structure
   ```rust
   pub struct LineIndex {
       line_starts: Vec<usize>,
   }
   
   impl LineIndex {
       pub fn new(text: &str) -> Self { ... }
       pub fn offset_to_position(&self, offset: usize) -> Position { ... }
       pub fn position_to_offset(&self, pos: Position) -> usize { ... }
   }
   ```

2. **Monaco Configuration Codegen** - Generate Monaco language configuration from Rust
   - Extract keywords from special form registry
   - Extract operators from parser binding power definitions
   - Generate JSON/TypeScript file during build
   - Consider using build.rs in cadenza-web

3. **Type System Integration** - Connect type inference to LSP
   - Hover should show inferred types
   - Diagnostics should include type errors
   - Completions should be type-aware

## Architecture

### Crates
- **cadenza-lsp** (this crate): Shared LSP utilities
- **cadenza** (CLI): Native LSP server using tower-lsp
- **cadenza-web**: WASM LSP functions for Monaco

### Dependencies
- `lsp-types` 0.94 (re-exported)
- `cadenza-syntax` for parsing
- `cadenza-eval` for evaluation (not yet used in LSP)
- `rowan` for CST traversal

### Consumer Integration

**Native (tower-lsp):**
```rust
use cadenza_lsp::core;
let diagnostics = core::parse_to_diagnostics(source);
let position = core::offset_to_position(source, offset);
```

**WASM (Monaco):**
```typescript
import { lsp_diagnostics, lsp_hover, lsp_completions } from 'cadenza-web';
const diagnostics = lsp_diagnostics(source);
const hover = lsp_hover(source, line, character);
const completions = lsp_completions(source, line, character);
```

## Testing

### ✅ Tests Passing
- `test_offset_to_position` - Position conversion
- `test_position_to_offset` - Offset conversion
- `test_parse_to_diagnostics` - Diagnostic generation with parse errors

### ⚠️ Test Coverage Gaps
- No tests for hover functionality
- No tests for completion functionality
- No tests for Monaco integration
- No performance benchmarks for position/offset conversion
- No tests with large documents
- No tests with multi-byte Unicode characters

## Priority Work Items

### High Priority
1. **Performance**: Implement LineIndex for O(1) position/offset conversion
2. **Correctness**: Fix Monaco comment configuration (`#` only, no block comments)
3. **Maintainability**: Generate Monaco keywords/operators from Rust code

### Medium Priority
4. Type-aware diagnostics (integrate with type inference)
5. Context-aware completions (from scope)
6. Go to definition support
7. Hover with type information

### Low Priority
8. Find references
9. Rename symbol
10. Code actions
11. Semantic highlighting

## References

- LSP Specification: https://microsoft.github.io/language-server-protocol/
- tower-lsp: https://github.com/ebkalderon/tower-lsp
- Monaco Editor: https://microsoft.github.io/monaco-editor/
