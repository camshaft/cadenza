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
   - [x] Attach source spans to diagnostics from AST nodes
   - [x] UndefinedVariable errors now include span of identifier
   - [x] Field access errors now include span of field name
   - [x] Added span() method to all AST nodes in syntax crate
   - [ ] TODO: Track source file information in diagnostics
   - [ ] TODO: Store "defined at" location for variables/functions
   - [ ] TODO: Show "defined at" messages in diagnostics (e.g., "variable `foo` defined at module.cdz:4")
   - [PR #4](https://github.com/camshaft/cadenza/pull/4#discussion_r2573085238)
   
   **Note**: Current implementation provides span information for error locations within
   a single source file. Full source tracking requires storing file info and definition
   locations with values, which is a larger feature requiring changes to how values are
   stored in the environment.

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

10. ~~**Move operators to std environment**~~ ✅
    - [x] Created BuiltinFn implementations for all operators (+, -, *, /, ==, !=, <, <=, >, >=)
    - [x] Registered operators in Env::register_standard_builtins()
    - [x] Removed hardcoded apply_operator function
    - [x] Operators are now first-class values in the environment
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

22. **Refactor Built-in Macros to Special Forms** (In Progress)
   - [x] Create BuiltinSpecialForm struct with eval_fn and ir_fn
   - [x] Add Value::SpecialForm variant using &'static BuiltinSpecialForm
   - [x] Organize special forms as submodules (one module per form)
   - [x] Update to use Result<ValueId> for IR generation (consistent with eval)
   - [x] Implement let_form as example with OnceLock pattern
   - [x] Migrate 11 builtins to special forms (evaluation only):
     - [x] `=` - Assignment operator
     - [x] `fn` - Function definition
     - [x] `__block__` - Block expressions
     - [x] `__list__` - List literals
     - [x] `__record__` - Record literals
     - [x] `match` - Pattern matching
     - [x] `assert` - Runtime assertions
     - [x] `typeof` - Type queries
     - [x] `measure` - Unit definitions
     - [x] `|>` - Pipeline operator
     - [x] `.` - Field access (implemented as `__index__` special form)
   - [x] Remove deprecated `builtin_*` BuiltinMacro functions from eval.rs
   - [ ] **Wire up special form IR generation to IR generator**
     - [ ] Modify IrGenerator::gen_apply to detect special forms and call their ir_fn
     - [ ] Remove hardcoded __block__ and __list__ handling in favor of special form dispatch
     - [ ] Test IR generation for all special forms
   - [ ] **Migrate builtin operators to special forms**
     - [ ] `+` - Addition operator
     - [ ] `-` - Subtraction operator
     - [ ] `*` - Multiplication operator
     - [ ] `/` - Division operator
     - [ ] `==` - Equality operator
     - [ ] `!=` - Inequality operator
     - [ ] `<` - Less than operator
     - [ ] `<=` - Less than or equal operator
     - [ ] `>` - Greater than operator
     - [ ] `>=` - Greater than or equal operator
   
   **Migration Pattern Used:**
   1. Create `special_form/<name>.rs` module
   2. Implement `get() -> &'static BuiltinSpecialForm` using OnceLock
   3. Implement `eval_fn` and `ir_fn` functions
   4. Update `Env::register_standard_builtins()` to use SpecialForm
   5. Update IR generator for IR generation support
   6. Move tests from old location to new module


## Priority Suggestions

### High Priority (Architectural Foundation)
- ~~**IR Integration**: Wire up IR generator to evaluator for end-to-end compilation~~ ✅
  - [x] Integrate IrGenerator with Compiler to generate IR during/after evaluation
  - [x] Add API to expose generated IR modules
  - [x] Validate that builder API supports evaluator needs
  - [x] Enable testing of IR generation with real-world code
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

## Language Features to Implement

This section tracks planned language features for the Cadenza evaluator that extend the core functionality.

### Assertions

~~Support for runtime assertions with rich diagnostic output that inspects and displays values.~~

**Status**: ✅ **Completed**

**Syntax**:
```cadenza
let v = 1
assert v == 1

assert v == 1 "expected v to be one"
```

**Requirements**:
- [x] Implement `assert` as a builtin macro
- [x] Basic assertion support (condition only)
- [x] Optional custom error message parameter
- [x] Rich diagnostic reporting with actual values
- [x] Integration with existing diagnostic system (miette)

**Implementation Notes**: 
- Assertions are implemented as a builtin macro that receives unevaluated expressions
- When an assertion fails, it reports the condition expression text in the error message
- Custom messages can be provided as a second argument
- Error messages include source location spans for precise error reporting
- Fixed bug in macro argument handling where `arguments()` was used instead of `all_arguments()`

### String Interpolation

Support for embedding expressions inside string literals using `${}` syntax.

**Syntax**:
```cadenza
let v = "world"
let s = "hello ${v}"
```

**Requirements**:
- [ ] Extend lexer to recognize `${}` within string literals
- [ ] Parser support for interpolated string expressions
- [ ] AST representation for interpolated strings
- [ ] Evaluator support to evaluate embedded expressions and concatenate results
- [ ] Handle nested expressions and escaping of `$` and `{}`
- [ ] Proper source tracking for interpolated parts

**Notes**: Common in many modern languages (JavaScript, Kotlin, Swift, etc.). Should convert all expressions to strings automatically. **Dependency**: Requires trait system to support automatic conversion of values to strings.

### Rational Numbers

Support for exact rational number arithmetic to avoid integer truncation and floating-point precision issues.

**Syntax**:
```cadenza
let v = 1 / 2  # does not immediately divide and truncate the integer
let a = v * 2
assert a == 1 "expected exact rational arithmetic: 1/2 * 2 should equal 1"
```

**Requirements**:
- [ ] Implement `Rational` type (likely using a numerator/denominator pair)
- [ ] Add `Value::Rational` variant
- [ ] Update division operator to return rationals when dividing integers
- [ ] Implement rational arithmetic (+, -, *, /)
- [ ] Implement comparison operators for rationals
- [ ] Auto-simplification of rationals (reduce to lowest terms)
- [ ] Conversion between integers, floats, and rationals
- [ ] Display formatting for rationals

**Notes**: Essential for exact arithmetic. Consider using existing Rust crates like `num-rational`. Should integrate with type system and unit system. Rationals should support measurement units for proper dimensional analysis.

### Quote and Unquote

Support for quoting expressions to create AST values and unquoting to evaluate them.

**Syntax**:
```cadenza
let foo = 1
let ast = quote foo
unquote ast  # returns 1
```

**Requirements**:
- [ ] Implement `quote` macro that prevents evaluation and returns AST
- [ ] Add `Value::Ast` or similar variant to hold unevaluated expressions
- [ ] Implement `unquote` macro to evaluate AST values
- [ ] Proper environment capture and hygiene
- [ ] Handle nested quote/unquote correctly

**Notes**: Foundation for metaprogramming and macros. Similar to Lisp's quote/unquote or Rust's `quote!` macro.

### Quote with Splicing (Unquote within Quote)

Support for embedding evaluated expressions within quoted code using unquote.

**Syntax**:
```cadenza
let foo = 1
let ast = quote foo
let ast2 = quote
    (unquote ast) + (unquote ast)
unquote ast2  # returns 2
```

**Requirements**:
- [ ] Extend quote macro to detect and handle `unquote` within quoted expressions
- [ ] Implement splicing mechanism to substitute evaluated expressions into AST
- [ ] Handle nested quote/unquote combinations
- [ ] Maintain proper source location information through splicing
- [ ] Test complex splicing scenarios

**Notes**: Enables template-like code generation. Must handle hygiene correctly to avoid variable capture issues.

### User-Defined Macros

Support for user-defined compile-time macros with proper hygiene and environment capture.

**Syntax**:
```cadenza
macro add_one expr =
    # Macros are functions that take unevaluated AST arguments and return an AST
    let one = quote 1
    quote
        (unquote expr) + (unquote one)

add_one 5  # expands to 5 + 1, returns 6
```

**Requirements**:
- [ ] Implement `macro` as a builtin macro (no keyword needed - identifiers handled by environment)
- [ ] Parser support for macro definitions
- [ ] AST representation for macro definitions
- [ ] Implement macro storage in environment (similar to functions)
- [ ] Macro expansion during evaluation (before evaluating macro application)
- [ ] Parameter binding for macro arguments (unevaluated)
- [ ] Hygiene: Ensure macros don't capture variables from call site unintentionally
- [ ] Proper error messages for macro expansion failures
- [ ] Recursive macro expansion support

**Notes**: Macros are compile-time functions that transform AST. Must ensure proper hygiene (gensym or similar). Dependencies: quote/unquote must be implemented first.

### Record Construction

Support for creating record (struct-like) values with named fields.

**Syntax**:
```cadenza
let record = { a = 1, b = 2 }

# Shorthand syntax
let a = 1
let b = 2
let record2 = { a, b }  # equivalent to { a = a, b = b }
```

**Requirements**:
- [x] Extend lexer/parser for record literal syntax `{ field = value, ... }` (Merged in #43)
- [x] Parser support for shorthand syntax `{ field, ... }` (No parser changes needed - macro handles it)
- [x] Add AST nodes for record literals (Merged in #43)
- [x] Implement `Value::Record` type with field name to value mapping
- [x] Evaluator support for record construction
- [x] Evaluator support for shorthand syntax
- [ ] Type checking for record literals (all fields present)
- [x] Implement record display/debug formatting

**Notes**: Records have structural typing initially. Fields are known at compile time. Consider using `InternedString` for field names.

**Future Performance Enhancements**:
- Consider using `Arc<[(InternedString, Type)]>` for sorted field names to enable binary search during field lookup
- Use `Box<[Value]>` for values, where index corresponds to field name position
- This avoids cloning field name lists for every record instantiation
- Down the line, consider a type interner where complex types have IDs for cheap comparison

### Record Merging

Support for merging multiple records into a new record using spread syntax.

**Syntax**:
```cadenza
let record_a = { a = 1 }
let record_b = { b = 2 }
let merged = { ...record_a, ...record_b }  # creates { a = 1, b = 2 }

# Later values override earlier ones if types match
let record_c = { a = 10 }
let overridden = { ...record_a, ...record_c }  # OK: { a = 10 }, same field type
```

**Requirements**:
- [ ] Add `...` (spread/rest) token to lexer
- [ ] Parser support for spread syntax in record literals
- [ ] AST representation for record spread
- [ ] Evaluator support for merging records
- [ ] Type checking: ensure overlapping fields have matching types
- [ ] Error messages for type conflicts in overlapping fields
- [ ] Preserve field order or define merge semantics

**Notes**: Spread operator is common in JavaScript/TypeScript. Later fields should override earlier ones if types match.

### Record Field Access

Support for accessing and assigning record fields using dot notation.

**Syntax**:
```cadenza
let record = { a = 1 }
let a = record.a

record.a = a + 2

assert record.a == 3
```

**Requirements**:
- [x] Parser already supports field access syntax (appears to be implemented in lexer/parser based on test files)
- [x] Implement field access evaluation in evaluator
- [x] Support field access chaining (e.g., `record.a.b`)
- [x] Support field access on arbitrary expressions (e.g., `(make_rec 1).x`)
- [x] Implement field assignment (mutation)
- [x] Type checking for field assignment (new value must match field type)
- [x] Type checking for field access (field must exist)
- [x] Error messages for accessing non-existent fields
- [x] Handle field access on non-record types with clear errors

**Notes**: 
- Field access (`.` operator) is implemented as a builtin macro and supports arbitrary expressions for the record
- Field assignment is handled by the `=` operator detecting field access on the LHS
- Field assignment includes type checking to ensure the new value matches the field's existing type
- Field assignment requires the record to be a variable (not an arbitrary expression) since ephemeral values cannot be mutated

### Destructuring / Pattern Matching on Records

Support for destructuring records in `let` bindings.

**Syntax**:
```cadenza
let record = { a = 1, b = 2 }
let { a, b } = record

# With renaming
let { a = a_renamed, b = b_renamed } = record

# With rest pattern
let { a, ...without_a } = record  # without_a is { b = 2 }
```

**Requirements**:
- [ ] Extend parser to support destructuring patterns in `let`
- [ ] AST nodes for destructuring patterns
- [ ] Implement record destructuring in evaluator
- [ ] Support field renaming syntax
- [ ] Support rest patterns (`...rest`) to capture remaining fields
- [ ] Type checking: ensure all destructured fields exist
- [ ] Error messages for missing fields or type mismatches
- [ ] Consider supporting nested destructuring

**Notes**: Start with simple cases, add renaming, then rest patterns. Foundation for more general pattern matching.

### Match Expressions

Support for pattern matching on values with multiple branches.

**Syntax**:
```cadenza
fn len a =
  match a
    [] ->
      0
    [_, ...tail] ->
      1 + len tail

assert (len [1, 2, 3]) == 3
```

**Requirements**:
- [ ] Add `match` keyword to lexer
- [ ] Parser support for match expressions with pattern arms
- [ ] AST representation for match expressions and patterns
- [ ] Pattern types: literals, variables, list patterns, record patterns, wildcards
- [ ] Implement pattern matching algorithm in evaluator
- [ ] Support list destructuring patterns (`[head, ...tail]`)
- [ ] Support record destructuring patterns in match arms
- [ ] Exhaustiveness checking (warn if not all cases covered)
- [ ] Error messages for non-exhaustive matches
- [ ] Proper scoping for pattern-bound variables

**Notes**: Complex feature, may want to start with simple patterns. Consider how this interacts with type system. The `_` wildcard and `...` rest patterns are essential.

### Struct (Nominal Record Types)

Support for defining nominally-typed records (structs) with type constructors.

**Syntax**:
```cadenza
struct Foo {
  a = Integer,
  b = Float,
}

let foo = Foo { a = 1, b = 2.0 }

assert foo.a == 1
assert foo.b == 2.0
```

**Requirements**:
- [ ] Implement `struct` as a builtin macro (no keyword needed - identifiers handled by environment)
- [ ] Parser support for struct definitions
- [ ] AST representation for struct definitions
- [ ] Type system support for nominal types
- [ ] Store struct definitions in environment/compiler
- [ ] Generate constructor functions for structs
- [ ] Struct type checking: ensure fields match definition
- [ ] Module-scoped nominal typing (structs from different modules are different)
- [ ] Field access on struct values
- [ ] Display/debug formatting for struct instances

**Notes**: Structs provide nominal typing vs structural typing of records. Same field structure but different names = different types. Important for type safety and API design.

### Enum Types

Support for algebraic data types (tagged unions) with named variants.

**Syntax**:
```cadenza
enum Foo {
  A = {
    a = Integer,
  },
  B = {
    b = Float,
  },
}

let value_a = Foo.A { a = 42 }
let value_b = Foo.B { b = 3.14 }

match value_a
  Foo.A { a } -> a
  Foo.B { b } -> 0  # would need to convert float to int or use common type
```

**Requirements**:
- [ ] Implement `enum` as a builtin macro (no keyword needed - identifiers handled by environment)
- [ ] Parser support for enum definitions with variants
- [ ] AST representation for enum definitions
- [ ] Type system support for sum types / tagged unions
- [ ] Store enum definitions in environment/compiler
- [ ] Generate constructor functions for enum variants
- [ ] Runtime tagging mechanism to identify variants
- [ ] Pattern matching integration for enums
- [ ] Type checking for enum construction and matching
- [ ] Exhaustiveness checking in match expressions for enums
- [ ] Display/debug formatting for enum values

**Notes**: Algebraic data types are essential for robust code. Common in functional languages (Rust, OCaml, Haskell). Should integrate with match expressions for destructuring. Consider how variants are constructed (e.g., `Foo.A { ... }` or `Foo::A { ... }`).

---

## Future Work (From Compiler Architecture)

This section tracks longer-term features described in `/docs/COMPILER_ARCHITECTURE.md` that extend beyond the current evaluator implementation.

### Type System (Phase 2)
- [x] **Hindley-Milner type inference**: Algorithm W with constraint generation/solving
  - [x] Type variables and unification
  - [x] Generalization and instantiation
  - [x] Occurs check
  - [x] Type environment for polymorphism
  - [x] Basic expression type inference (literals, identifiers, application)
  - [x] Full expression coverage (all language constructs: Op, Attr, Synthetic, Error)
- [ ] **Lazy type checking**: On-demand type inference for LSP responsiveness
  - [x] Type inferencer integrated with Compiler
  - [ ] API for LSP to request types
  - [ ] Background type checking
  - [ ] Cancellation support
- [x] **Macro metaprogramming with types**: Allow macros to query expression types
  - [x] Type inferencer accessible from compiler
  - [x] Macro API for type queries (TypeEnv::from_context)
  - [x] Example macros using type information (typeof builtin)
- [ ] **Type checking after evaluation**: Validate both evaluated and unevaluated branches
  - [ ] Track unevaluated branches
  - [ ] Type check unevaluated code paths
- [ ] **Dimensional analysis integration**: Dimension constraints alongside type constraints
- [ ] **Type annotations**: Optional type annotations in syntax
- [ ] **Unevaluated branch handling**: Mark and type-check branches not taken at eval-time

#### Performance and Architecture Enhancements

The following enhancements were identified during code review and are planned for future optimization:

- [ ] **Efficient data structures for InferType** (PR review comment 2592518863)
  - Consider using slot map architecture similar to the intern map
  - Store types as indices into a Vec for cheap cloning and comparison
  - Reduces memory allocation overhead during type inference
  
- [ ] **Caching for to_concrete()** (PR review comment 2592523329)
  - Add memoization for recursive to_concrete() conversions
  - Cache results to avoid redundant computation
  - Particularly beneficial for deeply nested types
  
- [ ] **Visitor pattern for type traversal** (PR review comment 2592529917)
  - Implement visitor pattern to avoid repeating traversal logic
  - Single implementation for common operations (free_vars, apply, etc.)
  - Makes adding new traversal operations easier and less error-prone
  
- [ ] **Lazy type environment with deferred unification** (PR review comment 2592573297)
  - Allow environment to register Expr nodes without immediate type inference
  - Defer unification until types are actually queried
  - Cache inferred types to avoid re-running queries
  - Reduces overhead for code that's never type-checked

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

### Code Generation (Phase 5)
- [x] **Intermediate representation**: Target-independent IR with optimization passes (basic implementation complete)
  - [x] IR types and builder API
  - [x] Basic IR generation from AST (literals, identifiers, binary operators, functions)
  - [x] ~~**Wire up IR generator to evaluator**~~ ✅ (Completed - IR generated during function definition)
  - [x] ~~**Typed operators in IR**~~: ✅ Each operator and instruction includes type information to preserve type inference results
  - [ ] Type-driven IR generation (integrate TypeInferencer results)
  - [ ] Support for function calls, conditionals, records, lists
- [ ] **Monomorphization**: Generate specialized functions for each type usage
- [ ] **Browser targets**: TypeScript/JavaScript (primary), WASM (optional)
- [ ] **Native targets**: Emit Rust code (primary), Cranelift/LLVM (optional)
- [ ] **Dead code elimination**: Remove unused specializations

#### WASM Code Generation

**Status**: Initial implementation merged with basic WAT output generation. Critical next step is proper value management.

**Current State**:
- [x] Basic WASM codegen infrastructure with wasm-encoder
- [x] WAT snapshot testing for all test-data files
- [x] Function type signatures and declarations
- [x] Simple constant generation (integers, floats, booleans)
- [x] Binary operations (arithmetic, comparison) - instruction generation only
- [x] Module structure with proper section ordering
- [x] ~~Value location tracking with local management~~ ✅ (Completed - all 89 WAT tests pass)

**Critical Issues** (blocking correct WASM output):
1. ~~**Value Location Tracking**~~: ✅ **FIXED** - WASM codegen now properly tracks SSA values as WASM locals and generates correct `local.get`/`local.set` instructions.

**High Priority Tasks** (in order):
- [x] ~~**Value location tracking and local management**~~  ✅
  - [x] Design data structure to map ValueId → LocalIndex or StackPosition
  - [x] Track function parameters as locals 0..N
  - [x] Allocate locals for SSA values (one local per ValueId that needs storage)
  - [x] Implement strategy: parameters as locals, results pushed to stack
  - [x] Generate `local.get` instructions to load operands before operations
  - [x] Generate `local.set` instructions when values need to be stored
  - [x] Handle the case where a value is used multiple times (must store in local)
- [x] ~~**Function calls**~~  ✅
  - [x] Load arguments in order using `local.get` or stack values
  - [x] Generate `call` instruction with correct function index
  - [x] Handle return values on stack
  - [x] Test with recursive functions
  - [x] Tail call optimization using `return_call` instruction (important for functional languages)
- [ ] **Control flow** 🔨 **IN PROGRESS**
  - [x] Add `if` special form to the language (evaluator level)
  - [x] Add test file with if expressions (if-simple.cdz)
  - [x] Add WASM test with Branch/Jump terminators (test_generate_function_with_branch)
  - [x] Implement structured control flow codegen for WASM (simple if-else pattern)
    - [x] Generate WASM `if-else-end` instructions for conditional branches
    - [x] Handle blocks that return directly from each branch (no phi nodes)
    - [ ] Handle phi nodes by ensuring correct values on stack
    - [ ] Map arbitrary basic block graphs to structured control flow (harder - may need block restructuring)
  - [ ] Implement IR generation for `if` expressions
    - [ ] Refactor IR generator to work with multiple blocks
    - [ ] Generate Branch terminators from `if` macro applications
    - [ ] Create then/else/merge blocks with proper phi nodes
  - [ ] Implement unconditional jump (br) for loops
  - [ ] Generate proper WASM blocks and loops for complex control flow
- [x] **Unary operations**
  - [x] Fix negation to properly load operand first (uses 0 - operand pattern)
  - [x] Fix logical not with proper type conversions
  - [ ] Implement bitwise not

**Architectural Notes on Control Flow**:

WebAssembly uses structured control flow (if/else/block/loop) rather than arbitrary jumps.
To map IR's basic block graph to WASM's structured control flow:

1. **Simple patterns** (if-then-else-merge with phi):
   - Recognize the pattern: entry → branch → (then, else) → merge (phi) → return
   - Generate: `if (result_type) ... else ... end`
   - Handle phi by ensuring correct value is on stack from each branch

2. **General case** (arbitrary control flow graph):
   - May need to restructure the control flow graph (Relooper algorithm or similar)
   - Use WASM's `block` and `br` (break) instructions for forward jumps
   - Use `loop` instruction for backward jumps
   - Each phi node needs special handling to ensure values flow correctly

3. **Implementation approach**:
   - Start with simple if-then-else patterns (most common case)
   - Add support for loops and more complex patterns as needed
   - Consider using a separate CFG analysis pass before codegen

**Medium Priority** (after basic codegen works):
- [ ] **String constants**
  - [ ] Use WASM GC arrays for string representation
  - [ ] Import/export string handling functions
  - [ ] Use UTF-8 encoding (requires validation but better compatibility)
- [ ] **Quantity/measurement values**
  - [ ] Compile-time eliminate units and use raw numbers
  - [ ] Dimensional analysis happens at compile time
  - [ ] Any conversions made explicit by IR passes calling conversion functions
- [ ] **Records**
  - [ ] Use WASM GC struct types
  - [ ] Generate field access instructions (struct.get)
  - [ ] Generate struct construction (struct.new)
  - [ ] Note: Tuples should be eliminated to records by IR codegen time
- [ ] **Lists**
  - [ ] Use WASM GC array types (investigate if growable arrays are possible)
  - [ ] Generate array operations (array.new, array.get, array.set)
  - [ ] Consider implications for dynamic sizing

**Lower Priority** (optimizations and advanced features):
- [x] **Optimization passes** ✅
  - [x] Dead code elimination
  - [x] Constant folding
  - [x] Common subexpression elimination
  - [ ] Inlining small functions
  - [x] Configurable optimization pipeline with OptimizationPass trait
- [ ] **Export generation and linking model**
  - [ ] Determine linking strategy: single WASM binary per package vs per-module
  - [ ] Consider parallelization benefits of small modules
  - [ ] Export functions (investigate if mangling needed with component model)
  - [ ] Generate getter functions for constant exports
  - [ ] Handle module-level exports
  - [ ] Plan for hot reloading support
- [ ] **Better type handling**
  - [ ] Use WASM GC reference types for complex types
  - [ ] Implement runtime type tags for union types
  - [ ] Generate type checking code for dynamic operations
- [ ] **Debugging support** (should be planned early, difficult to add later)
  - [ ] Generate DWARF debug info
  - [ ] Source maps for WAT output
  - [ ] Function name section for better stack traces
  - [ ] Track derivation of computed const values for better error reporting
- [ ] **Memory management**
  - [ ] Integrate with WASM GC proposal
  - [ ] Handle reference counting for shared values
  - [ ] Optimize struct layout for cache efficiency

**Testing Strategy**:
- [ ] Add focused unit tests for value location tracking
- [ ] Validate WAT output with wasmparser
- [ ] Test function calls with varying argument counts
- [ ] Test nested function calls and recursion
- [ ] Add performance benchmarks
- [ ] **Differential fuzz testing**: Generate Cadenza programs, compare interpreter vs WASM codegen output
  - [ ] Add operation limit to interpreter to prevent infinite loops
  - [ ] Essential for compile-time execution safety

**References**: 
- WASM spec: https://webassembly.github.io/spec/
- WASM GC proposal: https://github.com/WebAssembly/gc
- wasm-encoder docs: https://docs.rs/wasm-encoder/

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

