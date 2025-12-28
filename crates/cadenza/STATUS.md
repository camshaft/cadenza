# Cadenza Compiler - Implementation Status

This document tracks the implementation status of the Cadenza compiler based on the design in `docs/design/compiler.md`.

## Overview

The Cadenza compiler implements a multi-phase pipeline that progressively annotates the AST from parse through WASM emission.

## Core Data Structures

### Object (The Central Type)

- [x] Define `Object` struct that includes a `Value` enum
- [x] Add type annotation field
- [x] Add ownership metadata field
- [x] Add source tracking (spans, file IDs)
- [x] Add contract metadata field
- [x] Add documentation field
- [x] Add monomorphization metadata field

**Status**: ✅ Core structure implemented in `src/object.rs`

The `Object` struct has been implemented with all required fields:

- `value`: The `Value` enum with all expression types
- `ty`: Optional type annotation
- `ownership`: Optional ownership metadata
- `source`: Optional source file reference
- `contract`: Optional contract metadata
- `documentation`: Optional documentation string
- `monomorphization`: Optional monomorphization metadata

The `Value` enum includes all major expression types from the design:

- Literals (Number, String, Bool, Char, Unit)
- Symbols with scope tracking
- Apply (function application)
- Collections (List, Array, Dict)
- Structs, Tuples, EnumVariants
- Functions with captures
- Control flow (Let, If, Match, While, Do)
- References (Ref, Deref)
- Type operations (Type, TypeInscription)
- Macros (Macro, Quote, Unquote)
- Attributes and Errors

### Type System

- [x] Define core `Type` enum (Integer, Float, String, Bool, etc.)
- [x] Add compound types (List, Tuple, Record, Struct, Enum)
- [x] Add function types with lifetimes
- [x] Add reference types with lifetime tracking
- [x] Add type variables for inference
- [x] Add quantified types (`forall`)
- [x] Add refined types with contracts
- [x] Add dimensional types for units
- [x] Add constrained types with traits
- [x] Add effect types

**Status**: ✅ Complete type system implemented in `src/types.rs`

The type system has been implemented with all features from the design document:

- **Base types**: Integer, Float, Rational, String, Bool, Char, Unit with optional bit widths
- **Compound types**: List, Tuple, Record (structural), Struct (nominal), Enum
- **Function types**: With parameters, return type, and lifetime tracking
- **Reference types**: Mutable and immutable borrows with lifetime annotations
- **Type variables**: For Hindley-Milner type inference
- **Quantified types**: `forall` for polymorphic functions
- **Refined types**: Types with contract predicates for design-by-contract
- **Dimensional types**: Physical dimensions for units of measure with operations
- **Constrained types**: Trait constraints for ad-hoc polymorphism
- **Effect types**: Effect requirements for computational context

The type system includes:

- `is_linear()` method to determine if a type requires memory management
- `base_type()` method to unwrap type wrappers
- `structurally_equal()` for structural type comparison
- Full `Display` implementation for readable type formatting
- Dimension operations (multiply, divide, power) for dimensional analysis
- Test suite covering linearity, dimensions, and display

### Memory Management Types

- [ ] Define `Deleter` enum (Proper, Fake, Primitive, Reference)
- [ ] Define `MemState` struct for tracking active deleters
- [ ] Define `Lifetime` enum (InsideFunction, OutsideFunction)
- [ ] Add ownership status types (Owned, Moved, Borrowed)

## Phase 1: Parse

**Status**: ✅ Delegated to `cadenza-syntax`

The parse phase is handled by the existing `cadenza-syntax` crate which provides:

- Lossless CST with rowan
- Error recovery
- Incremental re-parsing
- Full span information

**Integration**:

- [ ] Add hash-based parse cache
- [ ] Implement conversion from CST to initial `Object`

## Phase 2: Evaluate

**Purpose**: Macro expansion and module building

### Core Evaluation

- [ ] Implement tree-walk interpreter
- [ ] Add environment/scope tracking
- [ ] Implement variable binding and lookup
- [ ] Implement function application
- [ ] Track unevaluated branches for type checking

### Macro System

- [ ] Implement macro expansion
- [ ] Add quote/unquote support
- [ ] Track macro definitions in environment
- [ ] Recursive macro expansion until fixpoint

### Module Building

- [ ] Collect function definitions
- [ ] Collect type definitions
- [ ] Collect macro definitions
- [ ] Track public vs private exports (underscore prefix)
- [ ] Build module export list

### Special Forms

Special forms are implemented in Rust and define interactions with the compiler:

- [ ] `def` - Define functions
- [ ] `deftype` - Define types
- [ ] `defmacro` - Define macros
- [ ] `let` - Local bindings
- [ ] `if` - Conditionals
- [ ] `match` - Pattern matching
- [ ] `while` - Loops
- [ ] `quote` - Prevent evaluation
- [ ] `unquote` - Evaluate in quote context

### Project.cdz Support

- [ ] Implement `package` builtin
- [ ] Implement `dependency` builtin
- [ ] Implement `artifact` builtin
- [ ] Implement `profile` builtin
- [ ] Collect configuration in compiler state
- [ ] Validate configuration after evaluation

### Test Discovery

- [ ] Scan for `@test` attributes during evaluation
- [ ] Scan for `@property` attributes
- [ ] Collect test functions in compiler state
- [ ] Support `@tag` for test organization

## Phase 3: Type Check

**Purpose**: Infer and validate all types

### Constraint Generation

- [ ] Implement Algorithm W constraint generation
- [ ] Generate type equations from expressions
- [ ] Track constraints in constraint set
- [ ] Support recursive/mutual definitions

### Constraint Solving

- [ ] Implement unification algorithm
- [ ] Implement occurs check
- [ ] Find substitutions that satisfy constraints
- [ ] Report unification failures as type errors

### Type Inference

- [ ] Implement Hindley-Milner inference
- [ ] Support let-polymorphism
- [ ] Implement generalization at let-bindings
- [ ] Implement instantiation at use sites
- [ ] Iterative inference for mutual dependencies

### Dimensional Analysis

- [ ] Parse `measure` definitions
- [ ] Track base dimensions (meter, second, kilogram, etc.)
- [ ] Compute derived dimensions from operations
- [ ] Type check dimensional consistency
- [ ] Report dimension mismatch errors

### Trait System

- [ ] Infer trait constraints from operations
- [ ] Track trait requirements on type variables
- [ ] Support built-in traits (Numeric, Comparable, etc.)
- [ ] Validate trait implementations

### Effect System

- [ ] Track effects as implicit parameters
- [ ] Propagate effects through call graph
- [ ] Require effect handlers at component boundaries
- [ ] Report missing effect handlers

### Contract System

- [ ] Parse contract annotations (`@requires`, `@ensures`, `@invariant`)
- [ ] Validate contract predicates are well-typed
- [ ] Track contracts on types and functions
- [ ] Support constraint variable substitution (`$0`, `$1`, etc.)

### Rational Number Support

- [ ] Add `Rational` type to type system
- [ ] Automatic promotion: Integer division → Rational
- [ ] Numeric tower: Integer < Rational < Float
- [ ] Coercion rules between numeric types

## Phase 4: Ownership Analysis

**Purpose**: Track linear types and ensure memory safety

### Memory State Tracking

- [ ] Implement `MemState` with active deleter set
- [ ] Track type dependencies (types needing delete)
- [ ] Track lifetime mappings for references

### Ownership Operations

- [ ] Implement manage (add to owned set)
- [ ] Implement unmanage (remove from owned set)
- [ ] Implement transfer (unmanage + manage)
- [ ] Detect use-after-move errors
- [ ] Detect use of unowned values

### Lifetime Validation

- [ ] Check references don't outlive targets
- [ ] Track inside-function vs outside-function lifetimes
- [ ] Validate reference usage against target liveness

### Deleter Annotation

- [ ] Walk AST maintaining memory state
- [ ] Annotate expressions with deleters
- [ ] Calculate scope boundary cleanup
- [ ] Handle conditional branches (merge states)
- [ ] Handle loops (simulate twice)

### Reference and Copy

- [ ] Implement `&` operator for borrowing
- [ ] Implement `*` operator for copying
- [ ] Track reference creation
- [ ] Validate reference lifetimes

## Phase 5: Monomorphize

**Purpose**: Specialize generics for concrete types

### Generic Instantiation

- [ ] Find all generic function call sites
- [ ] Generate specialized versions for each type combination
- [ ] Apply polymorphic suffix naming (e.g., `identity__Int`)
- [ ] Recursively instantiate dependencies

### Trait Resolution

- [ ] Find trait implementations for concrete types
- [ ] Resolve trait method calls to implementations
- [ ] Validate trait constraints are satisfied

### Dead Code Elimination

- [ ] For reactor/command artifacts, eliminate unused functions
- [ ] Whole-program analysis to find reachable code
- [ ] Keep only functions transitively called from exports

### Library vs Component

- [ ] Skip monomorphization for library artifacts
- [ ] Preserve generic functions in library output
- [ ] Monomorphize only exports for reactor/command

### Monomorphization Cache

- [ ] Hash-based cache: (function_id + types) → specialized
- [ ] Reuse cached specializations across compilation units

## Phase 6: Lambda Lift

**Purpose**: Convert closures to top-level functions

### Closure Analysis

- [ ] Identify captured variables (free variables)
- [ ] Generate environment struct type
- [ ] Generate copy function for environment
- [ ] Generate delete function for environment

### Lifting Transformation

- [ ] Create top-level function with environment parameter
- [ ] Replace lambda with struct + function pointer
- [ ] Update call sites to pass environment

## Phase 7: Contract Instrumentation

**Purpose**: Enforce contracts at runtime or prove them

### Dynamic Mode

- [ ] Insert precondition checks at function entry
- [ ] Insert postcondition checks at function exit
- [ ] Insert invariant checks at type construction
- [ ] Generate panic on contract violation

### Static Mode (Future)

- [ ] Translate contracts to SMT-LIB
- [ ] Query SMT solver (Z3, CVC5)
- [ ] Remove checks for proven contracts
- [ ] Fall back to dynamic for unproven

### Hybrid Mode (Future)

- [ ] Try static verification first
- [ ] Fall back to dynamic checks if can't prove
- [ ] Optimize away proven contracts in release builds

## Phase 8: Generate WIT

**Purpose**: Generate component interface for exports

### Export Discovery

- [ ] Find public functions (not prefixed with `_`)
- [ ] Filter to concrete (non-generic) functions
- [ ] Warn on generic exports without `@t` annotation

### Type Mapping

- [ ] Map `Integer` → `s64`
- [ ] Map `Float` → `float64`
- [ ] Map `Rational` → `record { numerator: s64, denominator: s64 }`
- [ ] Map `String` → `string`
- [ ] Map `List T` → `list<T>`
- [ ] Map `Record` → `record { ... }`
- [ ] Handle unsupported types (e.g., functions)

### WIT Generation

- [ ] Generate package declaration
- [ ] Generate version
- [ ] Generate interface with function signatures
- [ ] Write WIT to `.wit` file

## Phase 9: Emit WASM

**Purpose**: Generate executable WASM component

### WASM Structure

- [ ] Generate import section (allocator, dependencies)
- [ ] Generate export section (from WIT)
- [ ] Generate memory section
- [ ] Generate type section (function signatures)

### Expression Emission

- [ ] Emit literals (i64.const, f64.const)
- [ ] Emit variable loads (local.get, memory.load)
- [ ] Emit function calls (call, call_indirect)
- [ ] Emit let bindings (local.set)
- [ ] Emit conditionals (if/else/end blocks)
- [ ] Emit loops (loop/br)
- [ ] Emit deleter calls (call $free)

### Memory Management

- [ ] Generate malloc calls for allocations
- [ ] Generate free calls at deleter positions
- [ ] Calculate struct layouts (offsets, alignment, padding)
- [ ] Implement string representation (ptr + len + cap)
- [ ] Implement list representation (ptr + len + cap)

### Allocator Integration

- [ ] Link with pre-compiled allocator.wasm
- [ ] Import malloc/free/realloc functions
- [ ] Share memory region with allocator

### Component Assembly

- [ ] Use wasm-encoder to build component
- [ ] Follow WASM component model conventions
- [ ] Write component to `.wasm` file

## Supporting Systems

### Error Handling

- [ ] Define `CompilerError` enum for all phases
- [ ] Implement `miette` integration for nice diagnostics
- [ ] Multi-error reporting (collect all errors)
- [ ] Source location tracking in errors
- [ ] Recovery strategies for partial compilation

### Incremental Compilation

- [ ] Hash-based parse cache
- [ ] Hash-based type cache
- [ ] Hash-based ownership cache
- [ ] Hash-based monomorphization cache
- [ ] Cache invalidation on source changes
- [ ] Shared cache between LSP and batch compiler

### Compiler State

- [ ] Track collected function definitions
- [ ] Track collected type definitions
- [ ] Track collected macro definitions
- [ ] Track collected test functions
- [ ] Track project configuration
- [ ] Track type environment
- [ ] Track memory state

## Module System

### Import Handling

- [ ] Parse WIT files for WASM component imports
- [ ] Load Cadenza library packages (`.cadenza-pkg`)
- [ ] Convert WIT types to Cadenza types
- [ ] Register imported functions in environment
- [ ] Link WASM components at runtime

### Export Handling

- [ ] Collect public definitions during evaluation
- [ ] Qualify exports with module name
- [ ] Generate export list for modules

### Artifact Types

- [ ] Support library artifacts (source + metadata, no WASM)
- [ ] Support reactor artifacts (WASM + WIT)
- [ ] Support command artifacts (WASM with `_start`)
- [ ] Generate test runner artifact from `@tests` path

## Testing Infrastructure

### Test Framework Integration

- [ ] Generate test runner from collected tests
- [ ] Execute test functions
- [ ] Catch panics and record results
- [ ] Report pass/fail counts
- [ ] Support test filtering by tag
- [ ] Support test name pattern matching

### Property-Based Testing

- [ ] Implement random value generation for types
- [ ] Respect type constraints in generation
- [ ] Generate test cases (default 100)
- [ ] Implement shrinking on failure
- [ ] Report minimal failing input

## Package Management

### Dependency Resolution

- [ ] Parse dependency declarations from Project.cdz
- [ ] Support registry dependencies
- [ ] Support git dependencies
- [ ] Support path dependencies
- [ ] Resolve transitive dependencies
- [ ] Generate lockfile with exact versions

### Package Format

- [ ] Bundle Cadenza source files
- [ ] Include type metadata
- [ ] Include package manifest
- [ ] Support pre-compiled WASM (optional)

## LSP Integration

### Core LSP Features

- [ ] Hover: Show type on hover
- [ ] Completion: Autocomplete identifiers
- [ ] Go to definition: Jump to source
- [ ] Diagnostics: Show errors as-you-type
- [ ] Semantic tokens: Semantic highlighting

### LSP-Specific Concerns

- [ ] Share compiler pipeline with batch mode
- [ ] Incremental re-compilation on file changes
- [ ] Background type checking
- [ ] Cancellation support
- [ ] Cache sharing between LSP and builds

## Next Steps

Based on the design document, the recommended implementation order is:

1. **Object and Type System** - Core data structures
2. **Phase 2: Evaluate** - Get basic evaluation working
3. **Phase 3: Type Check** - Implement type inference
4. **Phase 4: Ownership** - Add memory safety
5. **Phase 9: Emit WASM** - Get end-to-end compilation working (skip intermediate phases initially)
6. **Phase 5: Monomorphize** - Add generics support
7. **Phase 6-8**: Lambda lifting, contracts, WIT generation
8. **LSP and Package Management** - Tooling support

The strategy is to get a minimal end-to-end pipeline working first, then incrementally add the more sophisticated features.
