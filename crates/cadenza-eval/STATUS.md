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

1. ~~**Error should include syntax nodes and stack traces**~~ ✅
   - [x] Migrated to miette, added StackFrame with source info
   - [PR #4](https://github.com/camshaft/cadenza/pull/4#discussion_r2573079075)

2. ~~**Use InternedString instead of String in errors**~~ ✅
   - [x] Changed error variants to use InternedString
   - [PR #4](https://github.com/camshaft/cadenza/pull/4#discussion_r2573079460)

3. ~~**Store errors in compiler instead of bailing**~~ ✅
   - [x] Added diagnostics collection to Compiler, continue on error
   - [PR #4](https://github.com/camshaft/cadenza/pull/4#discussion_r2573090111)

4. ~~**Return actual parse error messages**~~ ✅
   - [x] Added ParseError variant with proper error propagation
   - [PR #4](https://github.com/camshaft/cadenza/pull/4#discussion_r2573090448)

### Types & Values

5. ~~**Types as eval-time values**~~ ✅
   - [x] Created Type enum, Value::Type variant, added type signatures to builtins
   - [x] Added List, Record, Tuple, Enum, Union types
   - [x] Removed TypeExpectation enum in favor of union types
   - [PR #4](https://github.com/camshaft/cadenza/pull/4#discussion_r2573079828)

6. **Values need syntax nodes for source tracking** (Partially Complete)
   - [x] Added SourceInfo struct and TrackedValue wrapper
   - [x] Implemented Clone for Expr to enable storing AST nodes
   - [ ] TODO: Integrate into Env and use in diagnostics
   - [PR #4](https://github.com/camshaft/cadenza/pull/4#discussion_r2573085238)

7. ~~**Value comparison should error on type mismatch**~~ ✅
   - [x] All comparison operators now require exact type match
   - [x] Type mismatches return TypeError diagnostic
   - [PR #4](https://github.com/camshaft/cadenza/pull/4#discussion_r2573087893)

### Eval Architecture

8. ~~**Make eval a trait with stack trace support**~~ ✅
   - [x] Created Eval trait, implemented for all expression types
   - [x] Created EvalContext consolidating env and compiler
   - [PR #4](https://github.com/camshaft/cadenza/pull/4#discussion_r2573081207)

9. ~~**BuiltinFn needs scope info**~~ ✅
   - [x] Updated BuiltinFn and BuiltinMacro signatures to take EvalContext
   - [PR #4](https://github.com/camshaft/cadenza/pull/4#discussion_r2573086109)

10. **Move operators to std environment**
    - Current: Operators hardcoded in `apply_operator` function
    - Needed: Load from "std" environment at startup
    - [PR #4](https://github.com/camshaft/cadenza/pull/4#discussion_r2573095772)

### Interner Improvements

11. ~~**Refactor interning to use ZST-parameterized storage**~~ ✅
   - [x] Single `Interned<S>` type with Storage trait, static OnceLock storage
   - [PR #4](https://github.com/camshaft/cadenza/pull/4#discussion_r2573082852)

12. **Investigate rowan API for zero-allocation interning**
    - Current: `SyntaxText.to_string().as_str()` allocates unnecessarily
    - [PR #4](https://github.com/camshaft/cadenza/pull/4#discussion_r2573285289)

13. **Use smol_str for reference-counted strings**
    - [PR #4](https://github.com/camshaft/cadenza/pull/4#discussion_r2573083069)

14. **Avoid allocation on intern lookup miss**
    - Use hashbrown directly for borrowed key lookups
    - [PR #4](https://github.com/camshaft/cadenza/pull/4#discussion_r2573081759)

15. ~~**Intern integers and floats**~~ ✅
   - [x] InternedInteger and InternedFloat with parse-time validation
   - [PR #4](https://github.com/camshaft/cadenza/pull/4#discussion_r2573090782)

### Testing & Ergonomics

16. ~~**Move tests to snapshot-based test-data directory**~~ ✅
   - [x] Created test-data with build script generation
   - [x] Snapshot testing via insta crate
   - [PR #4](https://github.com/camshaft/cadenza/pull/4#discussion_r2573084407)

17. **Add builtin! macro helper**
    - Current: Verbose BuiltinFn construction
    - Needed: Ergonomic macro for defining builtins
    - [PR #4](https://github.com/camshaft/cadenza/pull/4#discussion_r2573089374)

### Macros & Special Forms

18. ~~**Use Expr AST nodes instead of GreenNodes**~~ ✅
   - [x] Changed BuiltinMacro signature to use `&[Expr]`
   - [PR #21](https://github.com/camshaft/cadenza/pull/21#discussion_r2575592663)

19. ~~**Unify macros and special forms**~~ ✅
   - [x] Merged BuiltinMacro and BuiltinSpecialForm into unified type
   - [PR #21](https://github.com/camshaft/cadenza/pull/21#discussion_r2575592663)

20. ~~**Validate identifier nodes in special forms**~~ ✅
   - [x] Added validation with proper error messages
   - [PR #21](https://github.com/camshaft/cadenza/pull/21#discussion_r2575594998)

21. ~~**Simplify Apply evaluation logic**~~ ✅
   - [x] Unified with extract_identifier helper
   - [PR #21](https://github.com/camshaft/cadenza/pull/21#discussion_r2575603972)

## Priority Suggestions

### High Priority (Architectural Foundation)
- Item 10: Move operators to std environment
- Item 6: Complete source tracking integration
- **Type System**: Implement HM type inference (see Future Work below)
- **Module System**: Import/export mechanism (see Future Work below)

### Medium Priority (Performance/Correctness)
- Items 12-14: Interner performance improvements
- Item 17: builtin! macro for ergonomics

### Lower Priority (Nice-to-have)
- Advanced dimensional analysis features (temperature, SI prefixes)
- Error recovery with Error values

## Dimensional Analysis

### Completed ✅

22. **Basic unit system with dimensional analysis**
    - [x] Unit, Dimension, DerivedDimension types
    - [x] UnitRegistry in Compiler
    - [x] Value::Quantity for unit-aware values
    - [x] measure builtin macro
    - [x] Automatic dimension derivation in arithmetic
    - [x] Parser-level unit suffix detection (e.g., `25.4meter`)

### Known Issues

- **Parser emit order**: Arguments should be emitted before receiver for correct CST offsets
  - Currently emits receiver first which may cause offset issues

### Future Enhancements

- **Temperature conversions**: Support offset-based conversions (C ↔ F)
- **Integer support**: Avoid precision loss with int quantities
- **Named derived dimensions**: Register names like "velocity" for "meter/second"
- **User-space conversions**: API to convert between units
- **SI prefixes**: Built-in kilo, mega, giga with auto-formatting
- **Power-of-2 vs power-of-10**: Support mebi/mega for binary/decimal units

## Block Expressions

### Status: ✅ Completed

Block expressions have lexical scoping with proper `__block__` synthetic nodes.

**Implementation**:
- [x] Parser detects indentation-based blocks
- [x] Emits synthetic `__block__` nodes
- [x] Evaluator implements `__block__` macro with scope management

**Example**:
```cadenza
let foo =
    let bar = 1
    let baz = 2
    bar
```

**Tests**: `block-simple.cdz`, `block-scope.cdz`, `block-function-body.cdz`, `block-nested.cdz`

## Error Recovery with Error Values

### Status: Proposed Enhancement

**Current Behavior**: First error stops block evaluation, causing cascading errors.

**Proposed Enhancement**: Introduce `Value::Error` that:
- Type-checks with any type to prevent cascading errors
- Allows evaluation to continue collecting all errors
- Improves developer experience

**Priority**: Medium - Would improve DX but not blocking

---

## Future Work (From Compiler Architecture)

This section tracks longer-term features described in `/docs/COMPILER_ARCHITECTURE.md` that extend beyond the current evaluator implementation.

### Type System (Phase 2)
- [ ] **Hindley-Milner type inference**: Algorithm W with constraint generation/solving
- [ ] **Type checking after evaluation**: Validate both evaluated and unevaluated branches
- [ ] **Dimensional analysis integration**: Dimension constraints alongside type constraints
- [ ] **Type annotations**: Optional type annotations in syntax
- [ ] **Unevaluated branch handling**: Mark and type-check branches not taken at eval-time

**References**: COMPILER_ARCHITECTURE.md "Type System" section

### Module System (Phase 3)
- [ ] **Module structure**: Default exports, `_` prefix for private items
- [ ] **Import/export mechanism**: Modules as records with destructuring
- [ ] **Cross-module type checking**: Load, extract types, verify usage
- [ ] **Dependency resolution**: Module registry with cycle detection
- [ ] **@export attribute**: Name artifacts (STL, audio, etc.) for export

**References**: COMPILER_ARCHITECTURE.md "Module System" section

### Traits and Effects (Phase 4)
- [ ] **Trait system**: Type classes with implicit trait inference
- [ ] **Trait constraints**: Infer requirements from operations (e.g., `+` requires Numeric)
- [ ] **Effect system**: Computational context (DB, logging, config) as implicit parameters
- [ ] **Effect handlers**: Provide implementations at call sites with `with` blocks
- [ ] **Constraint solving**: Unify trait and effect constraints

**References**: COMPILER_ARCHITECTURE.md "Monomorphization and Trait System", "Effect System"

### Code Generation (Phase 5-6)
- [ ] **Intermediate representation**: Target-independent IR with optimization passes
- [ ] **Monomorphization**: Generate specialized functions for each type usage
- [ ] **Browser targets**: TypeScript/JavaScript (primary), WASM (optional)
- [ ] **Native targets**: Emit Rust code (primary), Cranelift/LLVM (optional)
- [ ] **Dead code elimination**: Remove unused specializations

**References**: COMPILER_ARCHITECTURE.md "Compilation Targets", "Monomorphization"

### LSP Integration (Phase 6)
- [ ] **Incremental compilation**: Use rowan's red-green tree for fast updates
- [ ] **LSP features**: Hover, completion, definition, semantic tokens, diagnostics
- [ ] **Background type checking**: Cancel outdated requests, lazy checking
- [ ] **Responsiveness**: Only re-parse/check changed regions

**References**: COMPILER_ARCHITECTURE.md "LSP Integration"

### MCP Integration (Phase 7)
- [ ] **Compiler as MCP server**: Query types, search symbols, inspect state
- [ ] **User-space MCP tools**: Introspection for writing tools in Cadenza
- [ ] **JSON schema generation**: Extract from types for tool definitions

**References**: COMPILER_ARCHITECTURE.md "MCP Integration"

---

## Implementation Roadmap

See `/docs/COMPILER_ARCHITECTURE.md` for the complete multi-phase roadmap:
- Phase 1 (Foundation) - Current: Tree-walk evaluator ✅
- Phase 2: Type System with HM inference
- Phase 3: Module System
- Phase 4: Traits and Effects  
- Phase 5: Code Generation
- Phase 6: LSP Integration
- Phase 7: Advanced Features (WASM, LLVM, MCP)

