# Cadenza Evaluator - Status Document

This document tracks the current state of the `cadenza-eval` crate and remaining work items based on code review feedback.

## Current State

The evaluator implements a minimal tree-walk interpreter for Cadenza. It can:

- Parse and evaluate literals (integers, floats, strings)
- Evaluate arithmetic and comparison operators
- Look up variables in scoped environments
- Apply builtin functions and macros
- Handle macro expansion

### Completed Tasks

- [x] Create new crate `cadenza-eval` with proper Cargo.toml
- [x] Implement `Interner` with FxHash and `InternedId` wrapper
- [x] Implement `Value` enum with Display/Debug
- [x] Implement `Env` with scoped `Map<InternedId, Value>`
- [x] Write tree-walk `eval` function handling literals, lists, applications
- [x] Add macro expansion handling for BuiltinMacro
- [x] Implement `Compiler` struct with `define_var` and `define_macro`
- [x] Create end-to-end tests
- [x] Use FxHash for all maps (compiler, env, interner) instead of std HashMap
- [x] Add `get_mut` to Env for assignment operator support
- [x] Remove `is_truthy` - only bools should be used for conditionals

## Remaining Work Items

### Error Handling & Reporting

1. ~~**Error should include syntax nodes and stack traces**~~
   - [x] COMPLETED: Restructured Error into ErrorKind with a wrapper carrying span and stack trace
   - [x] Migrated to miette for standardized diagnostics
   - [x] Renamed Error to Diagnostic with DiagnosticLevel for warnings/hints
   - [x] Added source file name (interned) to StackFrame
   - [PR Comment](https://github.com/camshaft/cadenza/pull/4#discussion_r2573079075)

2. ~~**Use InternedId instead of String in errors**~~
   - [x] COMPLETED: Changed `UndefinedVariable(String)` to `UndefinedVariable(InternedId)`
   - [x] Updated `Diagnostic::undefined_variable` to take `InternedId`
   - [x] Updated `display_with_interner` to resolve `InternedId` for display
   - [x] Updated all call sites in eval.rs
   - [PR Comment](https://github.com/camshaft/cadenza/pull/4#discussion_r2573079460)

3. ~~**Store errors in compiler instead of bailing**~~
   - [x] COMPLETED: Added `diagnostics` field to `Compiler` struct
   - [x] Added methods: `record_diagnostic`, `diagnostics`, `take_diagnostics`, `num_diagnostics`, `has_errors`, `clear_diagnostics`
   - [x] Modified `eval` function to continue evaluation on error, recording diagnostics in compiler
   - [x] Returns `Vec<Value>` instead of `Result<Vec<Value>>`, callers check `compiler.has_errors()`
   - [PR Comment](https://github.com/camshaft/cadenza/pull/4#discussion_r2573090111)

4. **Return actual parse error messages**
   - Current: Generic "syntax error" message
   - Needed: Include actual parse error details
   - [PR Comment](https://github.com/camshaft/cadenza/pull/4#discussion_r2573090448)

### Types & Values

5. **Types as eval-time values**
   - Current: Types are static strings in errors
   - Needed: Types should be eval-time values for inspection/operation
   - [PR Comment](https://github.com/camshaft/cadenza/pull/4#discussion_r2573079828)

6. **Values need syntax nodes for source tracking**
   - Current: Values have no source location info
   - Needed: Attach syntax nodes, handle multi-file tracking, add Expr as runtime value
   - [PR Comment](https://github.com/camshaft/cadenza/pull/4#discussion_r2573085238)

7. **Value comparison should error on type mismatch**
   - Current: `PartialEq` silently returns false for mismatched types
   - Needed: Dedicated comparison function returning type mismatch error
   - [PR Comment](https://github.com/camshaft/cadenza/pull/4#discussion_r2573087893)

### Eval Architecture

8. **Make eval a trait with stack trace support**
   - Current: `eval_expr` is a standalone function
   - Needed: Trait implementation with stack trace maintenance
   - [PR Comment](https://github.com/camshaft/cadenza/pull/4#discussion_r2573081207)

9. **BuiltinFn needs scope info**
   - Current: `fn(&[Value]) -> Result<Value>`
   - Needed: Access to env, compiler, interner; return `Result` with Expr as possible type
   - [PR Comment](https://github.com/camshaft/cadenza/pull/4#discussion_r2573086109)

10. **Move operators to std environment**
    - Current: Operators hardcoded in `apply_operator` function
    - Needed: Load from "std" environment at startup
    - [PR Comment](https://github.com/camshaft/cadenza/pull/4#discussion_r2573095772)

### Interner Improvements

11. **Use rust-analyzer style intern with dashmap**
    - Current: Single-threaded HashMap-based interner
    - Needed: Thread-safe dashmap, single-hash lookup on miss
    - Reference: https://github.com/rust-lang/rust-analyzer/tree/master/crates/intern
    - [PR Comment](https://github.com/camshaft/cadenza/pull/4#discussion_r2573082852)

12. **Use smol_str for reference-counted strings**
    - Current: `Vec<String>` for reverse lookup
    - Needed: Use `smol_str` crate for reference-counted strings
    - [PR Comment](https://github.com/camshaft/cadenza/pull/4#discussion_r2573083069)

13. **Avoid allocation on intern lookup miss**
    - Current: Allocates string even when checking if key exists
    - Needed: Use hashbrown directly to get bucket for borrowed key
    - [PR Comment](https://github.com/camshaft/cadenza/pull/4#discussion_r2573081759)

14. **Intern integers and floats**
    - Current: Literals parsed on every evaluation
    - Needed: Intern map for integers/floats to avoid re-parsing
    - [PR Comment](https://github.com/camshaft/cadenza/pull/4#discussion_r2573090782)

### Testing & Ergonomics

15. **Move tests to snapshot-based test-data directory**
    - Current: Inline unit tests
    - Needed: test-data directory with snapshot tests like parser
    - [PR Comment](https://github.com/camshaft/cadenza/pull/4#discussion_r2573084407)

16. **Add builtin! macro helper**
    - Current: Verbose `BuiltinFn` struct construction
    - Needed: Ergonomic macro like `builtin!(fn inc(a: Integer) { a + 1 })`
    - [PR Comment](https://github.com/camshaft/cadenza/pull/4#discussion_r2573089374)

## Priority Suggestions

### High Priority (Architectural)
- Items 1, 6, 8: Error/value source tracking and stack traces
- Items 9, 10: BuiltinFn signature and std environment

### Medium Priority (Performance/Correctness)
- Items 11, 12, 13, 14: Interner improvements
- Items 2, 3, 4: Error handling improvements
- Item 7: Value comparison semantics

### Lower Priority (Ergonomics)
- Items 5, 15, 16: Types as values, snapshot tests, builtin! macro
