# Cadenza Evaluator - Status Document

This document tracks the current state of the `cadenza-eval` crate and remaining work items based on code review feedback.

## Non-Goals

**Backwards Compatibility**: The API is in-flight and may change at any time. Focus is on getting the design right rather than maintaining API stability.

## Current State

The evaluator implements a minimal tree-walk interpreter for Cadenza. It can:

- Parse and evaluate literals (integers, floats, strings)
- Evaluate arithmetic and comparison operators
- Look up variables in scoped environments
- Apply builtin functions and macros
- Handle macro expansion

### Completed Tasks

- [x] Create new crate `cadenza-eval` with proper Cargo.toml
- [x] Implement `InternedString` with static `OnceLock` storage and `Deref`
- [x] Implement `InternedInteger` and `InternedFloat` for literal interning
- [x] Implement `Value` enum with Display/Debug
- [x] Implement `Env` with scoped `Map<InternedString, Value>`
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

2. ~~**Use InternedString instead of String in errors**~~
   - [x] COMPLETED: Changed `UndefinedVariable(String)` to `UndefinedVariable(InternedString)`
   - [x] Updated `Diagnostic::undefined_variable` to take `InternedString`
   - [x] Updated `display_with_interned_string` to resolve via `Deref`
   - [x] Updated all call sites in eval.rs
   - [PR Comment](https://github.com/camshaft/cadenza/pull/4#discussion_r2573079460)

3. ~~**Store errors in compiler instead of bailing**~~
   - [x] COMPLETED: Added `diagnostics` field to `Compiler` struct
   - [x] Added methods: `record_diagnostic`, `diagnostics`, `take_diagnostics`, `num_diagnostics`, `has_errors`, `clear_diagnostics`
   - [x] Modified `eval` function to continue evaluation on error, recording diagnostics in compiler
   - [x] Returns `Vec<Value>` instead of `Result<Vec<Value>>`, callers check `compiler.has_errors()`
   - [PR Comment](https://github.com/camshaft/cadenza/pull/4#discussion_r2573090111)

4. ~~**Return actual parse error messages**~~
   - [x] COMPLETED: Added `ParseError` variant to `DiagnosticKind`
   - [x] Added `Diagnostic::parse_error` constructor with span
   - [x] Added `From<cadenza_syntax::parse::ParseError>` impl for `Diagnostic`
   - [x] Updated test helpers to return actual parse error messages instead of generic format
   - [PR Comment](https://github.com/camshaft/cadenza/pull/4#discussion_r2573090448)

### Types & Values

5. ~~**Types as eval-time values**~~
   - [x] COMPLETED: Created `Type` enum to represent runtime types
   - [x] Added `Value::Type(Type)` variant for types as first-class values
   - [x] Added `type_of() -> Type` method to Value for getting runtime type
   - [x] Updated `type_name()` to use `type_of().as_str()`
   - [x] Updated `TypeError` in `DiagnosticKind` to use `Type` instead of `String`
   - [x] Added `TypeExpectation` enum for flexible type expectations (single, multiple, description)
   - [x] Updated `NotCallable` in `DiagnosticKind` to use `Type`
   - [x] Exported `Type` and `TypeExpectation` from crate root
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

11. ~~**Refactor interning to use ZST-parameterized storage**~~
    - [x] COMPLETED: Single `Interned<S>` type with `Storage` trait
    - [x] `Interned<S>` implements `Deref` for direct value access
    - [x] `Storage` trait with `insert(&str) -> Index` and `resolve(Index) -> &'static Value`
    - [x] Static `OnceLock` storage instead of thread-local
    - [x] Storage types are `Send + Sync` for thread-safe usage
    - [x] `Interned::new(v: &str)` and `From<&str>` trait for easy creation
    - Original: https://github.com/camshaft/cadenza/pull/4#discussion_r2573082852

12. **Investigate rowan API for zero-allocation interning**
    - Current: `SyntaxText.to_string().as_str()` allocates a String just to intern
    - Needed: Find rowan API to get `&str` directly from `SyntaxText`
    - This defeats some of the purpose of interning (avoiding allocations)
    - [PR Comment](https://github.com/camshaft/cadenza/pull/4#discussion_r2573285289)

13. **Use smol_str for reference-counted strings**
    - Current: `Vec<String>` for reverse lookup
    - Needed: Use `smol_str` crate for reference-counted strings
    - [PR Comment](https://github.com/camshaft/cadenza/pull/4#discussion_r2573083069)

14. **Avoid allocation on intern lookup miss**
    - Current: Allocates string even when checking if key exists
    - Needed: Use hashbrown directly to get bucket for borrowed key
    - [PR Comment](https://github.com/camshaft/cadenza/pull/4#discussion_r2573081759)

15. ~~**Intern integers and floats**~~
    - [x] COMPLETED: `InternedInteger` and `InternedFloat` types
    - [x] Parse literal strings during interning, store `Result<T, ParseError>`
    - [x] Handles underscores in numeric literals (e.g., `1_000_000`)
    - [PR Comment](https://github.com/camshaft/cadenza/pull/4#discussion_r2573090782)

### Testing & Ergonomics

16. **Move tests to snapshot-based test-data directory**
    - Current: Inline unit tests
    - Needed: test-data directory with snapshot tests like parser
    - [PR Comment](https://github.com/camshaft/cadenza/pull/4#discussion_r2573084407)

17. **Add builtin! macro helper**
    - Current: Verbose `BuiltinFn` struct construction
    - Needed: Ergonomic macro like `builtin!(fn inc(a: Integer) { a + 1 })`
    - [PR Comment](https://github.com/camshaft/cadenza/pull/4#discussion_r2573089374)

## Priority Suggestions

### High Priority (Architectural)
- Items 1, 6, 8: Error/value source tracking and stack traces
- Items 9, 10: BuiltinFn signature and std environment

### Medium Priority (Performance/Correctness)
- Items 11, 12, 13, 14, 15: Interner improvements
- Items 2, 3, 4: Error handling improvements
- Item 7: Value comparison semantics

### Lower Priority (Ergonomics)
- Items 5, 15, 16: Types as values, snapshot tests, builtin! macro
