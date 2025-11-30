# Cadenza Web - Status Document

This document tracks the current state of the `cadenza-web` crate and remaining work items.

## Goals

1. **Interactive Compiler Development**: Provide a web-based tool for visualizing and debugging compiler stages
2. **Web-Friendly Compiler**: Ensure portability and prevent platform-specific assumptions early in development

## Current State

The crate provides WebAssembly bindings for the Cadenza compiler and a companion React application for interactive exploration.

### Completed Tasks

- [x] Create `cadenza-web` crate with proper Cargo.toml
- [x] Create DESIGN.md document
- [x] Create STATUS.md document (this file)
- [x] Implement WASM bindings for lexer (`lex`)
- [x] Implement WASM bindings for parser (`parse`)
- [x] Implement WASM bindings for AST (`ast`)
- [x] Implement WASM bindings for evaluator (`eval`)
- [x] Scaffold React+Vite+TypeScript application
- [x] Configure Tailwind CSS
- [x] Integrate Monaco Editor
- [x] Create compilation stage panels (Tokens, CST, AST, Eval)
- [x] Wire WASM bindings to UI
- [x] Build and verify WASM compilation

## Remaining Work Items

### WASM Bindings

1. **Add source location tracking to outputs**
   - Current: Basic output without spans
   - Needed: Include source positions for highlighting

2. **Improve error serialization**
   - Current: Basic error messages
   - Needed: Full diagnostic info with stack traces

3. **Add version info export**
   - Needed: Expose compiler version to the UI

### Compiler Explorer App

4. **Source location highlighting**
   - Current: No hover-to-highlight
   - Needed: Click/hover on output highlights source

5. **Persistent state**
   - Current: State lost on refresh
   - Needed: Save source to localStorage or URL

6. **Keyboard shortcuts**
   - Current: None
   - Needed: Compile, switch tabs, etc.

7. **Output formatting**
   - Current: Raw JSON output
   - Needed: Pretty tree views with syntax highlighting

8. **Error display**
   - Current: Basic error text
   - Needed: Inline error markers in editor

### Testing

9. **WASM unit tests**
   - Current: None
   - Needed: `wasm-bindgen-test` tests for bindings

10. **Integration tests**
    - Current: None
    - Needed: Playwright tests for the web app

## Priority Suggestions

### High Priority (Core Functionality)
- Items 1, 2: Source tracking and error serialization
- Items 7, 8: Better output display

### Medium Priority (UX)
- Items 4, 5: Highlighting and persistence
- Item 6: Keyboard shortcuts

### Lower Priority (Quality)
- Items 3, 9, 10: Version info and testing
