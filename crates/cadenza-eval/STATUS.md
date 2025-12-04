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
- Declare variables with `let` and assign with `=`

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
- [x] Implement `let` and `=` as special forms for variable declaration and assignment

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
   - [x] COMPLETED: Created `Type` enum with full type system support
   - [x] Added `Value::Type(Type)` variant for types as first-class values
   - [x] Added `type_of() -> Type` method to Value for getting runtime type
   - [x] Updated `type_name()` to use `type_of().as_str()`
   - [x] Replaced `BuiltinFn` and `BuiltinMacro` type markers with `Fn(Vec<Type>)` - unified function type
   - [x] Added `List(Box<Type>)` - list type with element type parameter
   - [x] Added `Record(Vec<(InternedString, Type)>)` - record type with field names and types
   - [x] Added `Tuple(Vec<Type>)` - tuple type with element types
   - [x] Added `Enum(Vec<(InternedString, Type)>)` - enum type with variant names and types
   - [x] Added `Union(Vec<Type>)` - union type for expressing "one of" (replaces TypeExpectation::OneOf)
   - [x] Added type signatures to `BuiltinFn` and `BuiltinMacro` structs
   - [x] Updated `TypeError` to use `Type` for both expected and actual
   - [x] Removed `TypeExpectation` enum - union types now express type alternatives
   - [x] Updated `NotCallable` in `DiagnosticKind` to use `Type`
   - [x] Exported `Type` from crate root
   - [PR Comment](https://github.com/camshaft/cadenza/pull/4#discussion_r2573079828)

6. **Values need syntax nodes for source tracking** (Partially Complete)
   - [x] Added `SourceInfo` struct with file and span tracking
   - [x] Added `TrackedValue` wrapper type to pair values with optional source information
   - [x] Added helper methods to `Value` for creating tracked values
   - [x] Added `from_expr` method to extract source info from expressions
   - [x] Implemented `Clone` for `Expr` to enable storing AST nodes when needed
   - [ ] TODO: Update `Env` to store `TrackedValue` instead of `Value` for variable definitions
   - [ ] TODO: Track source locations for function and variable definitions
   - [ ] TODO: Use source information in error diagnostics (e.g., "variable `foo` defined at file:line")
   - Note: Infrastructure is in place, but needs integration into evaluator and diagnostics
   - [PR Comment](https://github.com/camshaft/cadenza/pull/4#discussion_r2573085238)

7. ~~**Value comparison should error on type mismatch**~~
   - [x] COMPLETED: Updated `==` and `!=` operators to check type compatibility before comparison
   - [x] Updated `<`, `<=`, `>`, `>=` operators to require exact type match (strongly typed)
   - [x] Type mismatches now return `TypeError` diagnostic instead of silently returning false
   - [x] Added comprehensive test coverage for type mismatch errors
   - [x] Created snapshot tests for all comparison operators with type mismatches
   - [x] Uses `cmp()` and `partial_cmp()` for comparisons instead of float casting
   - Note: All comparison operators now require exact type match - no implicit conversions
   - [PR Comment](https://github.com/camshaft/cadenza/pull/4#discussion_r2573087893)

### Eval Architecture

8. ~~**Make eval a trait with stack trace support**~~
   - [x] COMPLETED: Created `Eval` trait with `eval(&self, ctx: &mut EvalContext) -> Result<Value>` method
   - [x] Implemented `Eval` trait for `Expr`, `Literal`, `Ident`, `Apply`, `Attr`, and `Synthetic`
   - [x] Created `EvalContext` struct that consolidates env and compiler into a single struct
   - [ ] Stack trace maintenance (future work - tracked separately)
   - [PR Comment](https://github.com/camshaft/cadenza/pull/4#discussion_r2573081207)

9. ~~**BuiltinFn needs scope info**~~
   - [x] COMPLETED: Updated `BuiltinFn` signature from `fn(&[Value]) -> Result<Value>` to `fn(&[Value], &mut EvalContext<'_>) -> Result<Value>`
   - [x] Updated `BuiltinMacro` signature from `fn(&[rowan::GreenNode]) -> Result<rowan::GreenNode>` to `fn(&[rowan::GreenNode], &mut EvalContext<'_>) -> Result<rowan::GreenNode>`
   - [x] Native functions now have access to env and compiler via `EvalContext`
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

16. ~~**Move tests to snapshot-based test-data directory**~~
    - [x] COMPLETED: Created test-data directory with `.cdz` files
    - [x] Added build script that generates tests from test-data files
    - [x] Added `testing.rs` module with `eval_all` helper that returns values and diagnostics
    - [x] Added snapshot testing via `insta` crate
    - [x] Snapshots capture both evaluated values and diagnostics for verification
    - [PR Comment](https://github.com/camshaft/cadenza/pull/4#discussion_r2573084407)
    - [PR Comment](https://github.com/camshaft/cadenza/pull/21#discussion_r2575622101)

17. **Add builtin! macro helper**
    - Current: Verbose `BuiltinFn` struct construction
    - Needed: Ergonomic macro like `builtin!(fn inc(a: Integer) { a + 1 })`
    - [PR Comment](https://github.com/camshaft/cadenza/pull/4#discussion_r2573089374)

### Macros & Special Forms

18. ~~**Use Expr AST nodes instead of GreenNodes**~~
    - [x] COMPLETED: Changed `BuiltinMacro` signature from `&[rowan::GreenNode]` to `&[Expr]`
    - [x] Updated `builtin_let` and `builtin_assign` to work directly with Expr arguments
    - [x] Removed intermediate GreenNode parsing and conversion steps
    - [x] Improved type safety and code clarity
    - [PR Comment](https://github.com/camshaft/cadenza/pull/21#discussion_r2575592663)

19. ~~**Unify macros and special forms into a single type**~~
    - [x] COMPLETED: Merged `BuiltinMacro` and `BuiltinSpecialForm` into a unified `BuiltinMacro` type
    - [x] Both now use `&[Expr]` arguments and return `Value` directly
    - [x] Simplified `Apply::eval` to handle macros uniformly
    - [x] Removed duplicate code paths for macro expansion and special form application
    - Note: Decided against `Value::Expr(Expr)` variant as `Expr` doesn't implement `Clone`. The unified type returning `Value` directly is simpler and achieves the same goal.
    - [PR Comment](https://github.com/camshaft/cadenza/pull/21#discussion_r2575592663)

20. ~~**Validate identifier nodes in special forms**~~
    - [x] COMPLETED: Updated `builtin_let` to validate that argument is an identifier
    - [x] Cast GreenNode to SyntaxNode, then to Expr
    - [x] Match on `Expr::Ident` and return syntax error for non-identifier expressions
    - [x] Added test case `error-let-invalid.cdz` to verify error message
    - [PR Comment](https://github.com/camshaft/cadenza/pull/21#discussion_r2575594998)

21. ~~**Simplify Apply evaluation logic**~~
    - [x] COMPLETED: Replaced duplicate Ident and Op checks with unified `extract_identifier` helper function
    - [x] Eliminated special-case checks for Ident vs Op in receiver handling
    - [x] Reduced code duplication and improved maintainability
    - [PR Comment](https://github.com/camshaft/cadenza/pull/21#discussion_r2575603972)

## Priority Suggestions

### High Priority (Architectural)
- ~~Items 1, 6, 8~~: Error/value source tracking and stack traces (Items 1 and 6 COMPLETED, Item 8 partially completed)
- Items 9, 10: BuiltinFn signature and std environment (Item 9 COMPLETED)
- ~~Items 18, 19~~: Macro/special form unification and Apply simplification (COMPLETED)
- ~~Item 21~~: Apply simplification (COMPLETED)

### Medium Priority (Performance/Correctness)
- Items 11, 12, 13, 14, 15: Interner improvements
- Items 2, 3, 4: Error handling improvements
- Item 7: Value comparison semantics
- ~~Item 20~~: Identifier validation in special forms (COMPLETED)

### Lower Priority (Ergonomics)
- Items 5, 15, 17: Types as values, builtin! macro
- ~~Item 16~~: Snapshot tests (COMPLETED)

## Dimensional Analysis

### Completed

22. **Basic unit system with dimensional analysis** (PR #XX)
    - [x] Added `Unit`, `Dimension`, `DerivedDimension` types
    - [x] Added `UnitRegistry` to Compiler for tracking units
    - [x] Added `Value::Quantity` for unit-aware numeric values
    - [x] Added `Value::UnitConstructor` for creating quantities
    - [x] Implemented `measure` builtin macro for defining units
    - [x] Added automatic dimension derivation in arithmetic (e.g., distance/time = velocity)
    - [x] Implemented parser-level unit suffix detection (e.g., `25.4meter`)
    - [x] Parser creates reversed Apply nodes for unit suffixes: `25.4meter` → `Apply(meter, [25.4])`

### Known Issues

- **measure macro with `=` syntax**: ~~(RESOLVED in a0f85c7)~~
  - Parser precedence causes `measure foot = inch 12` to parse as `[=, [measure, foot], [inch, 12]]`
  - Fixed by adding special handling in `=` operator for 'measure' callee (same pattern as 'fn')
  - Both `measure` and `fn` now require special handling in `=` due to juxtaposition having higher precedence than assignment

### Future Enhancements (from PR comments)

- **Temperature conversions**: Support offset-based conversions (C ↔ F) requiring addition/subtraction
- **Integer support**: Avoid precision loss by supporting both int and float in quantities
- **Named derived dimensions**: Register names like "velocity" for "meter/second" and display them
- **User-space conversions**: API to convert quantities between units (e.g., inches to mm, or inches/minute to meter/second)
- **SI prefixes**: Built-in knowledge of kilo, mega, giga, etc. with automatic display formatting
- **Power-of-2 vs power-of-10**: Support both mebi/mega for binary and decimal units (e.g., MiB vs MB)

### CST Offset Issue

- **Parser emit order**: Arguments should be emitted before receiver to maintain correct CST offsets
  - Currently emits receiver first, which may cause offset issues
  - AST doesn't care about order, but CST requires correct source positions
  - Needs fix in parse_literal() reversed Apply logic

## Block Expressions

### Current Status: ✅ Completed

Block expression support has been successfully implemented in both the parser and evaluator.

**Implementation Summary (PR #XX)**
- Parser detects when multiple expressions are at the same indentation level following an operator
- Emits synthetic `__block__` nodes instead of creating nested Apply chains
- Evaluator implements `__block__` builtin macro with proper scope management
- Block expressions have lexical scoping - variables defined inside blocks are isolated from outer scope

**Example**:
```cadenza
let foo =
    let bar = 1
    let baz = 2
    bar
```

**Parser Output**: `[=, [let, foo], [__block__, [=, [let, bar], 1], [=, [let, baz], 2], bar]]`

### What Was Completed

#### Parser Changes (`cadenza-syntax/src/parse.rs`)
- [x] Detect block contexts when indentation increases from parent during infix operator argument parsing
- [x] Use `child_marker.should_continue()` combined with indentation checks for proper continuation logic
- [x] Collect all expressions at the same indentation level into a synthetic `__block__` node
- [x] Emit `Apply(__block__, expr1, expr2, ...)` structure

#### Evaluator Changes (`cadenza-eval/src/eval.rs`)
- [x] Implement `builtin_block()` macro that:
  - Pushes a new scope for the block
  - Evaluates each expression in sequence
  - Returns the last expression's value
  - Pops the scope when exiting
- [x] Update `extract_identifier()` to handle Synthetic nodes for macro lookup
- [x] Register `__block__` in synthetic node whitelist

#### Environment (`cadenza-eval/src/env.rs`)
- [x] Register `__block__` macro in standard builtins

#### Tests
- [x] `block-simple.cdz`: Basic multi-statement block
- [x] `block-scope.cdz`: Verifies proper variable scoping
- [x] `block-function-body.cdz`: Function definitions with block bodies
- [x] `block-nested.cdz`: Deeply nested blocks

### Previous Implementation Attempts

**What Was Attempted (Earlier PR)**
- Tried to implement block expressions using a heuristic approach in the evaluator
- Detected blocks by checking if an Apply node's receiver was a statement (like `let` or `=`)
- This approach was **reverted** due to being too brittle and specific

**Why the Heuristic Approach Failed**
1. **Too Specific**: Only worked for `let` and `=` statements, couldn't handle arbitrary expressions
2. **Structural Limitations**: Failed on 3+ statement blocks due to complex nested Apply structures
3. **Wrong Layer**: Trying to detect blocks in the evaluator is a workaround; blocks should be a parser concern
4. **Not Extensible**: Couldn't handle function definitions, arbitrary function calls, or other expressions in blocks


### Known Challenges (Resolved)

1. ~~**Parser Complexity**~~: Successfully added block detection without breaking existing parsing
2. ~~**Indentation Edge Cases**~~: Handled through `WhitespaceMarker` and `should_continue()` logic
3. ~~**Scope Management**~~: Blocks properly push/pop scopes while allowing access to outer variables
4. **Error Recovery**: Currently stops on first error in a block (see Error Recovery section below)
5. ~~**CST Preservation**~~: Synthetic nodes correctly preserve span information

### Future Enhancements

See the "Error Recovery with Error Values" section below for planned improvements to error handling in blocks.

## Error Recovery with Error Values

### Current Status: Proposed Enhancement

**Current Behavior**
- When an expression in a block fails to evaluate, the `?` operator causes the entire block to stop evaluation
- Only the first error is reported, subsequent expressions are not evaluated
- This leads to cascading errors and incomplete error reporting

**Proposed Enhancement**
The evaluator should introduce a special `Error` value type that:
- Type-checks with any type to prevent cascading type errors
- Allows evaluation to continue past errors
- Collects all errors for the entire module upfront rather than one at a time

**Benefits**
1. **Better Error Messages**: See all errors in a module, not just the first one
2. **Reduced Cascading Errors**: An error in one expression doesn't prevent type-checking later expressions
3. **Improved Developer Experience**: Fix multiple errors in one iteration instead of one at a time

**Implementation Approach**

```rust
// Add to Value enum
pub enum Value {
    // ... existing variants
    Error(Box<Diagnostic>),
}

// In builtin_block:
for expr in args {
    match expr.eval(ctx) {
        Ok(value) => result = value,
        Err(diagnostic) => {
            // Record the error
            ctx.compiler.record_diagnostic(diagnostic);
            // Continue with an Error value
            result = Value::Error(Box::new(diagnostic));
        }
    }
}
```

**Type Checking Considerations**
- `Error` values should be considered compatible with any expected type
- When an `Error` value is used in an operation, it propagates but doesn't cause a new error
- This prevents cascading "undefined variable" errors when a previous error made a variable unavailable

**Example**
```cadenza
let foo =
    let bar = undefined_var  # Error 1: undefined variable
    let baz = bar * 2        # Would normally error, but continues with Error value
    baz + 10                 # Would normally error, but continues with Error value

# Both errors reported:
# - undefined_var is not defined
# (no cascading errors about bar or baz)
```

**Related Issues**
- Diagnostic collection infrastructure already exists (STATUS.md item #3 - completed)
- Need to update type system to handle Error values
- Need to update all builtin operations to propagate Error values

**Priority**: Medium - Would significantly improve developer experience but not blocking for current functionality

## References

- Parser code: `crates/cadenza-syntax/src/parse.rs` 
- WhitespaceMarker: `parse.rs` lines 717-737
- Block implementation: `crates/cadenza-eval/src/eval.rs` lines 1004-1030
- Existing synthetic nodes: `__list__` and `__record__` as examples
- Token generation: `crates/cadenza-syntax/build/token.rs` lines 647-663

