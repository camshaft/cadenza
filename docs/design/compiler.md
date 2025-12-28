# Cadenza Compiler

This document describes the architecture for the Cadenza compiler, which supports static typing, linear memory management, WASM component output, and tooling support. The design takes inspiration from Rust, OCaml, Lisp, Carp, F#, Ada, and Elixir, while adapting them for a WASM-first development and interactive environment for creative programming.

## Document Structure

This document is organized into the following sections:

1. **Overview** Vision, goals, and context
2. **Core Concepts** - Fundamental ideas and terminology
3. **Type System** - Types, inference, dimensions, traits, effects, contracts
4. **Memory Management** - Linear types, ownership, lifetimes
5. **Compilation Pipeline** - The complete compilation process
6. **Module System** - WIT integration, artifacts, imports/exports
7. **Project Configuration** - Project.cdz design
8. **Testing Framework** - Built-in tests and property-based testing
9. **Code Generation** - WASM emission strategy
10. **LSP Integration** - Incremental compilation and editor support
11. **Package Management** - Component registry and dependency resolution
12. **Implementation Roadmap** - Priorities and timeline

## Overview

### Vision

Cadenza is a statically-typed Lisp+ML that targets WASM components. It combines:

- **Functional programming** with immutable-by-default semantics
- **Linear type system** for deterministic memory management without garbage collection
- **Dimensional analysis** for physical units and measurements
- **Contract programming** for correctness guarantees
- **First-class tooling** with LSP support throughout
- **Component-based architecture** using WIT for interoperability with other languages
- **Standardized build system** with integrated package manager

### Design Goals

1. **Type Safety**: Catch errors at compile time through Hindley-Milner type inference with extensions for dimensions, contracts, and effects

2. **Predictable Performance**: Linear type system ensures deterministic memory behavior - no garbage collection pauses, explicit allocation/deallocation

3. **Interoperability**: Every module compiles to a WASM component with WIT interface, enabling integration with any language

4. **Developer Experience**: LSP support for all files (including configuration), readable error messages, interactive REPL, and integrated testing

5. **Progressive Rigor**: Start simple, add contracts and property tests as needed, optionally prove correctness with SMT solvers

6. **Real-World Modeling**: Built-in dimensional analysis for physical units, making it natural to work with real-world quantities

### Primary Use Cases

Cadenza targets specific domains where its unique features shine:

1. **3D Modeling**: Parametric design with dimensional analysis (millimeters, inches, etc.)
2. **Music Composition**: Algorithmic composition with time/pitch units
3. **Scientific Computing**: Calculations with proper unit tracking
4. **Interactive Visualizations**: Generative art and data visualization
5. **Embedded Systems**: Deterministic memory for resource-constrained devices

### Non-Goals

- **Garbage collection**: Use linear types for deterministic memory management
- **Dynamic typing**: Static types ensure correctness
- **Runtime type information**: Types erased after compilation
- **Backwards compatibility**: API will evolve during development

## Core Concepts

This section introduces fundamental concepts that underpin the Cadenza compiler architecture. Understanding these concepts is essential for understanding how the various systems interact.

### The Object

**Inspired by**: Carp's XObj (eXpression with eXtras)

The central data structure throughout the compilation pipeline is the **Object** - a value which represents compile-time values. Each phase in the Cadenza compiler adds additional information to the object.

**What it contains:**

- **Original syntax** (`Expr` from cadenza-syntax) - The actual source code structure
- **Type information** - Inferred types, trait constraints, effect requirements
- **Memory metadata** - Ownership status, deleters, lifetime information
- **Source tracking** - File location, spans, unique identifiers
- **Contracts** - Preconditions, postconditions, invariants
- **Monomorphization** - Specialized names for instantiated generics

**Why this approach:**

- **Simplicity**: One representation, not multiple IRs
- **Source fidelity**: Easy to trace back to original code
- **Incremental**: Can annotate partially and continue
- **Proven**: Carp successfully uses this pattern to emit C code

### Linear Types and Ownership

**Inspired by**: Carp's memory management system, Rust's ownership model

Linear types are types where each value has exactly one owner at any point in time. This property enables automatic memory management without garbage collection.

**Key properties:**

- **Single owner**: Only one binding owns a value's memory
- **Explicit moves**: Ownership transfers are explicit in the code
- **References for borrowing**: Temporary read access without ownership transfer
- **Deterministic cleanup**: Compiler inserts delete calls when values go out of scope

**Linear vs Non-linear types:**

- **Linear**: String, List, Record, Struct, Function closures - need memory management
- **Non-linear**: Integer, Float, Bool, references - just copy the bits

**The "use once" rule**: Each linear value can only be moved once from a binding in a given scope. References allow temporary borrowing for read access without consuming the value.

### Lifetimes

**Inspired by**: Carp's lifetime system, Rust's lifetime checking

Lifetimes track how long values and references remain valid. Every reference has an associated lifetime that the compiler uses to ensure references don't outlive the values they point to.

**Two lifetime modes:**

- **Inside function**: Reference depends on a local variable in current scope
- **Outside function**: Reference depends on something beyond current function (e.g., a global or parameter)

**Validation**: The compiler checks that references are only used when their target value is still alive (has a deleter in the active scope).

### Deleters

**Inspired by**: Carp's deleter system

Deleters are markers that track which values need cleanup and where. As the compiler analyzes code, it maintains a set of active deleters representing currently-owned values.

**Four deleter types:**

- **Proper deleter**: Call actual delete function for managed types
- **Fake deleter**: Marks ownership without actual cleanup (for tracking)
- **Primitive deleter**: Non-managed types (just documentation)
- **Reference deleter**: Reference ownership (no cleanup, just tracking)

**How they work**: When ownership transfers (via move), the deleter moves from one expression's annotation to another. At scope boundaries, remaining deleters trigger delete calls.

### Monomorphization

**Inspired by**: Carp's concretization, Rust's monomorphization

Monomorphization is the process of generating specialized versions of generic functions for each concrete type they're used with.

**Example:**

```
Generic: fn identity<T>(x: T) -> T
Used with: Integer, String, Float
Generates: identity__Int, identity__String, identity__Float
```

**Why needed**: WASM doesn't have native generics. All generic code must be specialized to concrete types before emission.

**Polymorphic suffix**: The naming scheme that distinguishes specializations (e.g., `__Int__String` for a function specialized with Integer and String).

### Constraints

**Inspired by**: Hindley-Milner type inference, Ada's constrained types

Constraints are equations that must be satisfied for the program to type-check. The compiler generates constraints while analyzing code, then solves them via unification.

**Three kinds of constraints:**

- **Type constraints**: Equations between types (e.g., `t1 = Integer`)
- **Dimension constraints**: Equations between physical dimensions
- **Contract constraints**: Predicates that must hold (e.g., `x > 0`)

**Solving**: Unification finds substitutions that satisfy all constraints, or reports conflicts.

### WIT Interfaces

**WASM Component Model standard**

WIT (WebAssembly Interface Types) is a language for describing component interfaces. Every Cadenza package is able to compile to a WASM component with a WIT interface describing its exports.

**Key properties:**

- **Concrete types only**: WIT doesn't support generics
- **Cross-language**: Any language can import a WIT interface
- **Versioned**: WIT includes versioning for compatibility
- **Standardized**: Part of the WASM Component Model specification

**Cadenza integration**: The compiler automatically generates WIT from exported functions, mapping Cadenza types to WIT types.

### Artifacts

**Inspired by**: Rust's library/binary distinction

Artifacts are the different kinds of outputs the compiler can produce:

- **Library (`lib`)**: Source + type metadata for use by other Cadenza projects, keeps generics
- **Reactor (`reactor`)**: WASM component with WIT interface for cross-language use, concrete only
- **Command (`command`)**: WASM component with WIT interface which exports a single `_start` function

**Selection**: Project.cdz specifies which artifacts to build. A single codebase can produce multiple artifacts.

### Incremental Compilation

**Strategy**: Hash-based caching

To support interactive development and LSP responsiveness, the compiler caches results at each phase:

**Cache keys:**

- **Parse cache**: Keyed by source file content hash
- **Type cache**: Keyed by expression hash + type environment hash
- **Ownership cache**: Keyed by typed expression hash
- **Monomorphization cache**: Keyed by function ID + type arguments

**Invalidation**: When source changes, hash changes, cache misses trigger recomputation. Caches shared between LSP and batch compilation.

### Homoiconicity

**Lisp tradition**

The language's AST can be represented as language values, enabling powerful metaprogramming:

- **Quote**: Prevents evaluation, returns AST as a value
- **Unquote**: Evaluates an AST value back to result
- **AST manipulation**: Macros receive and return AST structures

**In practice**: Macros receive `Expr` nodes (the AST type), can inspect and construct new `Expr` nodes, enabling arbitrary source transformations at compile time.

## Type System

The Cadenza type system combines Hindley-Milner type inference with extensions for physical dimensions, contracts, traits, and effects. The system is designed to catch errors early while requiring minimal type annotations from users.

### Type Universe

**Core principle**: Types are values that can be inspected at compile time.

Cadenza's types form a hierarchy:

- **Base types**: Integer, Float, String, Bool, Nil
- **Compound types**: List, Tuple, Record, Struct (nominal Record), Enum
- **Function types**: With lifetimes for memory safety
- **Reference types**: Borrowed access with lifetime tracking
- **Type variables**: For polymorphism during inference
- **Quantified types**: `forall` for generics
- **Refined types**: Base types with contract predicates
- **Dimensional types**: Numbers with physical units
- **Constrained types**: Types with trait requirements
- **Effect types**: Functions with effect requirements

**Type as value**: The type `Type` itself is a value, enabling introspection and metaprogramming. Macros can query and inspect types to generate specialized code.

### Hindley-Milner Type Inference

**Inspired by**: ML family (OCaml, Haskell, F#), Carp's constraint solver

The compiler uses Algorithm W for automatic type inference:

**Process:**

1. **Constraint generation**: Walk AST, generate type equations
2. **Constraint solving**: Use unification to solve equations
3. **Generalization**: Introduce `forall` quantifiers at let-bindings
4. **Instantiation**: Replace quantified variables with fresh variables at use sites

**Key benefit**: Users rarely need to write type annotations. The compiler infers them automatically and checks for consistency.

**Iterative solving**: The compiler may iterate the inference process multiple times to handle mutual dependencies and resolve ambiguities.

### Type Variables and Unification

**Type variables**: Placeholders during inference, written as lowercase letters (`a`, `b`, `t`, etc.)

**Unification**: The process of finding substitutions that make two types equal. For example:

- Unifying `a` with `Integer` produces the substitution `a := Integer`
- Unifying `List a` with `List Integer` produces `a := Integer`
- Unifying `a -> b` with `Integer -> String` produces `a := Integer, b := String`

**Occurs check**: Prevents infinite types by detecting when a type variable appears inside its own definition (e.g., `a = List a`).

### Polymorphism

**Parametric polymorphism**: Functions can work with any type:

```
fn identity x = x
Type: forall a. a -> a
```

**Type schemes**: Generic functions have quantified types. When called, type variables are instantiated with concrete types.

**Let-polymorphism**: Variables bound in `let` can have polymorphic types, but function parameters cannot (no rank-N types initially).

### Dimensional Analysis

**Inspired by**: F#'s units of measure, physics and engineering practice

Physical quantities have dimensions (length, time, mass, etc.) tracked by the type system:

**Base dimensions**: Defined with `measure` keyword:

```
measure meter    # Length
measure second   # Time
measure kilogram # Mass
```

**Derived dimensions**: Computed from operations:

```
distance / time  => velocity (meter/second)
force * distance => energy (kilogram·meter²/second²)
```

**Type checking**: The compiler ensures dimensional consistency:

```
100meter + 50meter      # OK: same dimension
100meter + 5second      # Error: incompatible dimensions
distance / time         # OK: produces velocity dimension
```

**Representation**: Dimensions are tracked as part of the type, but erased in generated code (compile-time only).

### Trait System

**Inspired by**: Haskell's type classes, Rust's traits

Traits define interfaces that types can implement, enabling ad-hoc polymorphism.

**Implicit constraints**: The compiler infers trait requirements from usage:

```
fn sum list =
  fold list 0 (fn acc x -> acc + x)

# Inferred: forall a. [Numeric a] => List a -> a
# The use of + implies a must implement Numeric
```

**No manual annotations**: Unlike Rust, users don't write trait bounds explicitly. The compiler infers them from operations.

**Resolution during monomorphization**: When specializing a generic function, the compiler finds the appropriate trait implementation for the concrete type.

### Effect System

**Inspired by**: Algebraic effects, Koka, Haskell's effect systems

Effects represent computational context (IO, logging, configuration, etc.) as implicit parameters:

**Implicit propagation**: Functions that call effectful functions inherit those effects:

```
fn process_data data =
  Logger.log "Processing"  # Requires Logger effect
  transform data

# Inferred: forall a. [Logger] => a -> a
```

**Effect handlers**: Provide implementations at call sites, similar to dependency injection:

```
with console_logger
  process_data my_data
```

**Compile-time checking**: All required effects must be provided before reaching component boundaries. Missing effects are compile errors.

### Contract Programming

**Inspired by**: Ada's constrained types, Eiffel's design by contract

Contracts are predicates attached to types and functions:

**Constrained types**: Subtypes with invariants:

```
@invariant $0 >= 0
type Natural = Integer

@invariant $0 >= 0.0 && $0 <= 100.0
type Percentage = Float
```

**Function contracts**: Pre and postconditions:

```
@requires b != 0 "divisor cannot be zero"
@ensures $0 * b == a "result * divisor = dividend"
fn divide a b = a / b
```

**Verification modes**:

- **Dynamic**: Insert runtime checks (default)
- **Static**: Prove with SMT solver (Z3, CVC5)
- **Hybrid**: Static where possible, dynamic fallback
- **None**: Documentation only (fastest)

**Progressive adoption**: Start without contracts, add them incrementally as code matures.

### Type and Dimension Interaction

Dimensional analysis integrates with the type system:

**Constrained dimensional types**:

```
@invariant $0 >= 0.0 "speed cannot be negative"
type Speed = Quantity meter/second

@invariant $0 >= 0.0 "temperature cannot be below absolute zero"
type Temperature = Quantity kelvin
```

**Type inference with dimensions**: Both type and dimension are inferred simultaneously:

```
let d = 100meter      # Type: Quantity meter
let t = 5second       # Type: Quantity second
let v = d / t         # Type: Quantity meter/second (inferred!)
```

**Compile-time checking**: Dimension errors caught during type checking, not at runtime.

### Rational Numbers

**Inspired by**: Haskell's Rational, Scheme's exact arithmetic

Rational numbers provide exact arithmetic without floating-point precision loss or integer truncation. This is essential for financial calculations, unit conversions, and precise mathematical operations.

**Automatic promotion**: Integer division produces rationals, not truncated integers:

```
let half = 1 / 2         # Type: Rational (not 0!)
let result = half * 2    # Type: Rational (equals 1 exactly)
```

**Integration with dimensions**: Rationals work seamlessly with dimensional analysis:

```
measure inch
measure centimeter

# 1 inch = 2.54 cm (exact rational conversion)
let conversion_factor = 254 / 100  # Rational, not 2.54 float
let length = 10inch
let in_cm = length * conversion_factor centimeter
# Result is exact, no floating-point error accumulation
```

**Numeric tower**: The type system includes an implicit numeric hierarchy:

```
Integer < Rational < Float
```

**Coercion rules**:

- Integer operations stay Integer when possible
- Division always produces Rational (preserves exactness)
- Any operation with Float produces Float (precision lost)
- Rationals automatically simplify (6/4 becomes 3/2)

**Why essential**: Prevents subtle bugs from precision loss, especially important in dimensional calculations where compounding errors can lead to incorrect physical results.

**Performance consideration**: Rationals are more expensive than floats, but correctness often matters more. Users can explicitly perform integer or floating point arithmetic when performance is critical.

## Memory Management

Cadenza uses a linear type system for automatic, deterministic memory management without garbage collection. This system is directly inspired by Carp's memory management approach, adapted for WASM's linear memory model.

### The Golden Rule

**Each linear value can only be used once.**

This single rule enables all the safety guarantees of the memory management system. A "use" occurs when a value's ownership transfers to a different binding or scope.

### Linear vs Non-Linear Types

**Linear types** require memory management:

- String - dynamically allocated UTF-8
- List - dynamically sized array
- Function closures - includes captured environment

**Non-linear types** are just copied:

- Integer - 64-bit value
- Float - 64-bit value
- Rational - numerator and denominator (may become linear)
- Bool - single byte
- References - pointer value (borrowed, not owned)

**Unspecified** types are only linear if their subtypes are:

- Record - a structural type with fields
- Struct - user-defined nominal types
- Enum - a tagged union of optionally-named subtypes

**Determination**: A type is linear if it implements the `delete` interface. The compiler auto-generates `delete` for all user-defined types.

### Three Core Operations

The memory management system revolves around three operations:

#### 1. Moving (Transfer of Ownership)

Moving transfers ownership from one binding to another:

```
let string = "hello"
let other = string      # Move: ownership transfers to 'other'
let invalid = string    # Error: string no longer owns the value
```

**Across scopes**: Returning or passing a value moves it:

```
fn process text =
  reverse text   # Move: ownership transfers to parameter

let result = process my_string  # my_string moved into function
# Can't use my_string here anymore
```

**One move per binding**: Each binding can move its value once in a scope.

#### 2. Borrowing (Temporary Access)

References provide temporary read access without transferring ownership:

```
let string = "hello"
let reversed = reverse &string   # Borrow: & creates reference
let concatenated = concat string reversed  # OK: string still owned
```

**No ownership transfer**: The `&` operator creates a reference without moving the original value.

**Reference types**: Have their own type `Ref<T, lifetime>` separate from `T`.

#### 3. Copying (Explicit Duplication)

The `*` operator duplicates a referenced value:

```
fn duplicate text_ref =
  *text_ref   # Copy: creates new independent value
```

**When needed**: If you need to use a value multiple times but can only move it once, copy via reference:

```
let text = "hello"
let copy1 = *text
let copy2 = *text
# Now have three independent strings
```

**Cost**: Copying allocates new memory. Made explicit with `*` to show cost.

### The Memory State (MemState)

**Inspired by**: Carp's Memory.hs

As the compiler analyzes code, it maintains a **memory state** tracking:

**Active deleters**: Set of values currently owned and needing cleanup

```
{string: ProperDeleter, list: ProperDeleter}
```

**Type dependencies**: Types needing delete functions

```
{String, List Integer}
```

**Lifetime mappings**: References and their validity

```
{lt_0: InsideFunction("string"), lt_1: OutsideFunction}
```

### Manage and Unmanage Operations

**Manage**: Add a value to the set of owned values:

```
let x = "hello"
# Manage: x added to deleters set
```

**Unmanage**: Remove a value from owned set (ownership transferred):

```
let y = x
# Unmanage x: removed from deleters
# Manage y: added to deleters
```

**Transfer**: Combines unmanage + manage:

```
transfer_ownership(from: x, to: y)
```

**At scope end**: Remaining deleters trigger cleanup:

```
(
  let x = "hello"
  let y = "world"
  # Scope ends
  # Emit: delete(x); delete(y);
)
```

### Lifetime Validation

**Goal**: Ensure references don't outlive their targets.

**Check performed**: When using a reference, verify its target value is still alive (has a deleter in the current memory state).

**Invalid reference**:

```
fn bad_ref =
  let x = "temporary"
  &x  # Error: x deleted at function end, reference outlives it
```

**Valid reference**:

```
let global = "permanent"

fn good_ref =
  &global  # OK: global outlives the reference
```

**Two lifetime modes**:

- **Inside function**: Reference depends on local variable (must check liveness)
- **Outside function**: Reference depends on parameter or global (always valid)

### Ownership Analysis Algorithm

**Inspired by**: Carp's `manageMemory` function in Memory.hs

The ownership analyzer walks the AST in a single pass, maintaining memory state:

**For each expression**:

1. **Visit children** recursively
2. **Check ownership rules** (can we use this value?)
3. **Update memory state** (manage/unmanage as needed)
4. **Annotate with deleters** (what needs cleanup here?)

**At scope boundaries**:

- **Calculate diff**: What's owned now vs. at scope start?
- **Attach deleters**: Add cleanup markers to scope-ending expressions
- **Restore state**: Pop scope, return to parent state

**For conditionals** (if/match):

- **Branch divergence**: Track state in each branch independently
- **Convergence**: At merge point, only values deleted in ALL branches are deleted
- **Attach deleters**: Each branch gets deleters for its unique cleanups

**For loops** (while):

- **Simulate twice**: Visit body twice to catch use-after-move in subsequent iterations
- **Attach deleters**: Cleanup at loop end for values used in loop body

### Deleter Types

**Four flavors**, each serving a specific purpose:

**Proper deleter**: Calls actual delete function:

```
ProperDeleter {
  path: "String_delete",
  variable: "my_string",
}
```

**Fake deleter**: Tracks ownership without actual cleanup (for analysis):

```
FakeDeleter {
  variable: "captured_var",
}
```

**Primitive deleter**: Non-managed type (no cleanup, just documentation):

```
PrimDeleter {
  variable: "my_int",
}
```

**Reference deleter**: Borrowed value (no cleanup):

```
RefDeleter {
  variable: "text_ref",
}
```

### Memory Management and Type System Integration

**Managed types**: Types requiring memory management are identified during type checking.

**Delete function discovery**: For generic types, the compiler finds the appropriate delete function during monomorphization using the type dependency information.

**Example**:

```
fn process value =
  # Do something
  value  # Moves value out

# When specialized for String:
# Compiler finds String_delete and generates call if value isn't returned
```

**Copy function discovery**: Similar process for when values need to be copied.

### WASM Memory Model

**Linear memory**: WASM provides a single linear memory space (array of bytes).

**Allocator interface**: Pre-compiled WASM module providing:

- `malloc(size: u32, alignment: u32) -> u32` - Allocate bytes, return pointer
- `free(ptr: u32)` - Deallocate bytes at pointer
- `realloc(ptr: u32, size: u32) -> u32` - Resize allocation
- Single exported memory region

**Integration**: Cadenza's delete calls map to `free()` calls in WASM. The ownership analysis determines exactly where to insert these calls.

**No GC needed**: Because ownership is tracked statically, we know precisely when to free memory. No runtime tracking, no GC pauses.

### Escape Hatches

**When needed**: Sometimes the ownership rules are too restrictive.

**Unsafe operations** (future):

- Manual memory management when needed
- Explicit delete calls
- Custom allocators for specific types

**Philosophy**: Make the safe path the default, provide escape hatches when required.

## Compilation Pipeline

The Cadenza compiler operates in distinct phases, each building on the previous. Unlike traditional compilers with multiple IR transformations, Cadenza follows Carp's pattern of progressively annotating the same Object structure through each phase.

### Pipeline Overview

```
Source Files + Project.cdz
         ↓
Phase 1: Parse (cadenza-syntax)
         ↓
Phase 2: Evaluate (macro expansion, module building)
         ↓
Phase 3: Type Check (infer types, traits, effects, dimensions, contracts)
         ↓
Phase 4: Ownership Analysis (track linear types, insert deleters)
         ↓
Phase 5: Monomorphize (specialize generics, resolve traits)
         ↓
Phase 6: Lambda Lift (convert closures to top-level functions)
         ↓
Phase 7: Contract Instrumentation (insert or prove contracts)
         ↓
Phase 8: Generate WIT (export interface for component)
         ↓
Phase 9: Emit WASM (walk annotated AST, generate code)
         ↓
Linked WASM Component
```

### Phase 1: Parse

**Input**: Source text  
**Output**: Concrete Syntax Tree (CST)  
**Responsibility**: Convert text to structured syntax tree

Uses cadenza-syntax crate (already implemented) with rowan for lossless parsing:

- Preserves all whitespace and comments
- Indentation-aware for blocks
- Error recovery for partial parses
- Incremental re-parsing on edits

**Caching**: Parse results keyed by source file content hash.

### Phase 2: Evaluate

**Input**: CST  
**Output**: Object with evaluation metadata  
**Responsibility**: Macro expansion, module structure building

**Inspired by**: Carp's Eval.hs

This phase runs the tree-walk interpreter to:

- Expand macros (receive `Expr`, return `Expr`)
- Execute compile-time code
- Build module structure (collect exports)
- Hoist function definitions
- Track unevaluated branches (for later type checking)

**Key insight**: The evaluator IS the first compiler phase. Code runs at compile time to generate more code.

**Macro expansion**: Macros receive unevaluated AST nodes and return new AST nodes. The evaluator recursively expands until no more macros remain.

**Accumulation**: Results accumulate in Compiler state through special forms. These are implemented in Rust and define interactions with each phase.

- Function definitions registered
- Type definitions registered
- Macros registered
- Project configuration collected (from Project.cdz)
- Test functions collected (@test, @property)

**Unevaluated branches**: When a conditional branch isn't taken at compile time, track it for later type checking (ensures all code paths type check).

**Compile-time dependency execution**: Imported WASM components and Cadenza modules can be called during evaluation. This enables:

- Generate code based on dependency computation results
- Process data at compile time using external tools
- Serialize results into the output artifact

**Requirements for compile-time calls**:

- All arguments must be concrete values (no references)
- Return types must be serializable (primitives, records, lists)
- Component must be available during compilation

**Example use case**: A 3D modeling library could export mesh generation functions that are called at compile time to pre-compute geometry, with the results baked into the final component.

**Limitation**: Cannot call functions returning references or closures (not serializable). These can only be called at runtime.

### Phase 3: Type Check

**Input**: Object from evaluation  
**Output**: Object + Type annotations  
**Responsibility**: Infer and validate all types

**Inspired by**: Carp's Infer.hs, Constraints.hs

**Sub-phases**:

1. **Initial types**: Assign preliminary types (literals, known functions)
2. **Constraint generation**: Walk AST, generate type equations
3. **Constraint solving**: Unify types, find substitutions
4. **Dimensional checking**: Validate physical dimensions
5. **Trait inference**: Determine required trait constraints
6. **Effect inference**: Determine required effects
7. **Contract validation**: Check contract predicates are well-typed

**Iterative**: May repeat until convergence to handle mutual dependencies.

**Unevaluated code**: Also type-checks branches that weren't evaluated, ensuring complete coverage.

**Output**: Each Object now has:

- Concrete type (or polymorphic type scheme)
- Trait requirements
- Effect requirements
- Validated contracts

### Phase 4: Ownership Analysis

**Input**: Type-annotated Objects  
**Output**: Objects + Ownership metadata + Deleters  
**Responsibility**: Track linear types, ensure memory safety

**Inspired by**: Carp's Memory.hs

This is the phase that enables automatic memory management.

**Algorithm**:

- Walk AST with Memory State (set of active deleters)
- For each expression:
  - Check if value can be used (still owned)
  - Manage/unmanage based on ownership transfers
  - Validate reference lifetimes
  - Annotate with deleters for this expression
- At scope boundaries, calculate cleanup needed
- At conditionals, merge branch states

**Output**: Each Object annotated with:

- Ownership status (owned, moved, borrowed)
- Deleters to call at this point
- Lifetime validity information

**Memory safety errors** caught here:

- Use after move
- Use of unowned value
- Reference outliving target
- Double free (prevented by one-use rule)

### Phase 5: Monomorphize

**Input**: Ownership-analyzed Objects  
**Output**: Specialized concrete Objects  
**Responsibility**: Generate versions for each type usage

**Inspired by**: Carp's Concretize.hs

For each generic function:

1. **Find all call sites** with concrete types
2. **Generate specialized version** for each unique type combination
3. **Apply polymorphic suffix** (e.g., `identity__Int`)
4. **Resolve trait implementations** for concrete types
5. **Recursively instantiate dependencies** (specialized functions calling other specialized functions)

**Trait resolution**: For each trait constraint, find the implementation for the concrete type.

**Effect resolution**: Effects are monomorphic (no effect polymorphism), already resolved.

**Contract specialization**: Contracts are also specialized - predicates may simplify with concrete types.

**Artifact mode matters**:

- **Library**: Skip monomorphization, keep generics
- **Reactor/Command**: Monomorphize only exported functions, whole-program monomorphization, dead code elimination

### Phase 6: Lambda Lift

**Input**: Monomorphized Objects  
**Output**: Objects with no closures (all top-level functions)  
**Responsibility**: Convert closures to callable functions

**Inspired by**: Carp's lambda lifting in Concretize.hs

For each lambda:

1. **Identify captured variables** (free variables in body)
2. **Generate environment struct** type for captures
3. **Generate copy/delete functions** for environment
4. **Create top-level function** with environment parameter
5. **Replace lambda** with struct + function pointer

**Example transformation**:

```
Before: fn outer x = fn y -> x + y

After:
  struct Lambda_outer_0_env { x: Integer }
  fn Lambda_outer_0(env: Lambda_outer_0_env, y: Integer) -> Integer
  # Body of outer creates Lambda_outer_0_env{x} and returns function pointer
```

**Ownership**: Environment struct is linear, needs copy/delete like any struct.

### Phase 7: Contract Instrumentation

**Input**: Lambda-lifted Objects  
**Output**: Objects with contract checks inserted (or proven)  
**Responsibility**: Enforce contracts at runtime or prove them away

**Three strategies** based on verification mode:

**Dynamic mode**: Insert runtime checks:

- Preconditions: Check at function entry, panic if violated
- Postconditions: Check at function exit
- Invariants: Check at type construction and mutation

**Static mode**: Attempt to prove with SMT solver:

- Translate contracts to SMT-LIB
- Query solver (Z3, CVC5)
- If proven, remove checks
- If not proven, fall back to dynamic

**Hybrid mode**: Try static first, fall back to dynamic if can't prove.

**Optimization**: In release builds with hybrid/static mode, proven contracts have zero runtime cost.

### Phase 8: Generate WIT

**Input**: Instrumented Objects  
**Output**: WIT interface definition  
**Responsibility**: Generate component interface

**For reactor/command artifacts only** (libraries don't generate WIT).

**Process**:

1. **Find exported functions** (public, not prefixed with `_`)
2. **Check if generic** - emit warning if function monomorphizes to multiple types
3. **Map types to WIT** - Cadenza type → WIT type
4. **Generate interface** - package, version, function signatures

**Type mapping**:

- `Integer` → `s64`
- `Float` → `float64`
- `Rational` → `record { numerator: s64, denominator: s64 }`
- `String` → `string`
- `List T` → `list<T>`
- `Record` → `record { fields... }`
- `Fn` → Not directly representable (use resources)

**Generic handling**: Only concrete functions appear in WIT. Generic functions generate warnings.

### Phase 9: Emit WASM

**Input**: Fully annotated Objects + WIT  
**Output**: WASM component binary  
**Responsibility**: Generate executable code

**Inspired by**: Carp's Emit.hs

**Strategy**: Walk the annotated AST (like Carp walks XObj) and emit WASM:

**For each expression type**:

- **Literals**: Emit const instructions
- **Variables**: Load from locals or memory
- **Function calls**: Emit call instruction
- **Let bindings**: Allocate locals, emit assignments
- **Conditionals**: Emit if/else blocks with proper control flow
- **Loops**: Emit loop blocks with break conditions
- **Deleters**: Emit free() calls at marked positions

**Memory operations**:

- Allocations: Call `malloc` from allocator module
- Deallocations: Call `free` at deleter positions (determined by ownership analysis)
- Struct layout: Calculate offsets, generate field access

**Component structure**:

- Import allocator functions
- Export functions from WIT
- Include memory for data
- Type section with function signatures

**Output format**: WASM component (not core module), following component model conventions.

### Phase Interactions

**Type checking needs evaluation**: Macros must expand before types can be checked.

**Ownership needs types**: Must know which types are linear (requires type information).

**Monomorphization needs ownership**: Specialized functions need their ownership annotations preserved.

**WIT needs monomorphization**: Can only export concrete types.

**Emission needs everything**: Final phase uses all accumulated annotations.

### Error Handling Across Phases

**Progressive diagnostics**: Each phase can emit diagnostics without stopping compilation.

**Multi-error reporting**: Collect all errors, show them together at the end.

**Error quality**: Earlier phases provide better error messages (type errors are clearer than WASM emission errors).

**Recovery**: Later phases skip malformed items but continue with well-formed ones.

### Incremental Compilation

**Hash-based caching** at each phase:

**Parse cache**: `hash(source) → CST`  
**Type cache**: `hash(object + type_env) → typed object`  
**Ownership cache**: `hash(typed_object) → owned object`  
**Mono cache**: `hash(function_id + types) → specialized`

**Invalidation cascades**: Changing source invalidates all downstream caches for that file.

**Shared state**: LSP and batch compiler share the same cache, avoiding duplicate work.

## Module System

Cadenza's module system is built around WASM components and WIT interfaces. Every module can compile to a component, and components can be imported as modules. This tight integration enables both Cadenza-to-Cadenza and cross-language interoperability.

### Module Structure

**Export by default**: All top-level definitions are public unless prefixed with `_` or `@private` attribute.

**Example module**:

```
# math.cdz

# Public function (exported)
fn add x y = x + y

# Private helper (not exported)
fn _internal_helper x = x + 1

# Public type
struct Point { x = Float, y = Float }

# Public unit
measure meter
```

**Collected exports**: During evaluation (Phase 2), the compiler collects all public definitions into a module export list.

### Artifact Types and Their Outputs

**Three artifact types** serve different purposes:

#### Library Artifact

**Purpose**: Share generic code between Cadenza projects.

**Output**: `.cadenza-pkg` package containing:

- Source files (for generic instantiation)
- Type metadata (export signatures)
- No compiled WASM (instantiated at import site)

**Use case**: Math libraries, utility functions, any generic code.

**Generic preservation**: Functions keep their polymorphic types. No monomorphization.

**Import experience**: Other Cadenza projects import and instantiate generics as needed.

#### Reactor Artifact

**Purpose**: Reusable component for any language.

**Output**: `.wasm` + `.wit` files.

**Compilation**: Full monomorphization of exports. Generic functions must have explicit type annotations via the `@t` attribute:

```
@t Integer -> Integer
fn square x = x * x
```

**WIT generation**: Produces concrete interface:

```wit
interface math {
  square: func(x: s64) -> s64
}
```

**Warnings**: Generic functions without `@t` emit warnings (not included in WIT).

**Use case**: Libraries for JavaScript, Python, any language via component model.

#### Command Artifact

**Purpose**: Standalone executable with entry point.

**Output**: `.wasm` with `_start` function.

**Compilation**: Whole-program monomorphization, dead code elimination, fully linked.

**Entry point**: Exports `_start` that WASM runtime calls.

**Use case**: CLI tools, standalone applications, scripts.

### Import Mechanism

**Modules as records**: Imports bind to record values with exported names as fields.

**Three import styles**:

**Destructuring import**:

```
import { add, multiply } = math
let result = add 1 2
```

**Whole module**:

```
import math
let result = math.add 1 2
```

**Aliased import**:

```
import m = math
let result = m.add 1 2
```

**Dot notation**: Works naturally since modules are records. The `.` operator for field access applies to module access as well.

### WIT Import

**Loading**: When importing a WASM component, the compiler:

1. **Parse WIT file**: Extract interface definition
2. **Convert types**: WIT types → Cadenza types
3. **Register bindings**: Add functions to environment
4. **Link at runtime**: WASM component linked during execution

**Type checking**: Imports are type-checked against their WIT signatures. Calling an imported function validates argument types match.

**Cross-language**: Can import components written in any language (Rust, JavaScript, C++, etc.) as long as they provide WIT.

### Cadenza Library Import

**Loading**: When importing a Cadenza library (`.cadenza-pkg`):

1. **Load source**: Extract `.cdz` files from package
2. **Parse and evaluate**: Run through compiler phases
3. **Extract generics**: Get polymorphic function definitions
4. **Register**: Add to environment as instantiable functions

**Instantiation**: When calling a generic imported function:

- Check if concrete version exists in cache
- If not, monomorphize at import site
- Generate specialized version in importing module
- Use specialized version

**Benefit**: True generics within Cadenza ecosystem while maintaining component interop.

### Export Handling

**Automatic discovery**: During evaluation, compiler identifies exports:

- Public functions (not prefixed with `_`)
- Public types
- Public macros (expand at import site)
- Public units (measurements)

**Qualification**: Exports are qualified with module name to avoid collisions.

### Module Compilation Flow

**For library artifact**:

1. Parse source
2. Type check (but don't monomorphize)
3. Extract type signatures
4. Bundle source + metadata
5. No WASM emission

**For reactor artifact**:

1. Parse source
2. Evaluate and type check
3. Analyze ownership
4. Monomorphize exports (guided by @export)
5. Lambda lift
6. Generate WIT
7. Emit WASM
8. Link with allocator

**For command artifact**:

1. Same as reactor through lambda lift
2. Whole-program dead code elimination
3. No WIT generation (only `_start` export)
4. Emit WASM with entry point
5. Link with allocator and dependencies

### Dependency Resolution

**At compile time**: Project.cdz lists dependencies, package manager resolves:

1. **Download**: Fetch from registry or git
2. **Verify**: Check signatures, versions
3. **Cache**: Store locally for reuse
4. **Make available**: Provide to compiler for import

**Transitive dependencies**: Automatically resolved and linked.

**Version constraints**: SemVer-based compatibility checking.

### Cross-Module Type Checking

**At import boundary**: Type signatures validated:

1. **Import claims type**: Based on WIT or source metadata
2. **Usage type-checked**: Calls must match claimed signatures
3. **Dimension checking**: Units must be consistent across modules
4. **Trait requirements**: Must be satisfied by imports

**Error location**: Import errors reported at call site with reference to definition.

## Project Configuration

Project configuration in Cadenza is written as Cadenza code itself (`Project.cdz`), providing type checking and LSP support for the build system.

### Project.cdz Design

**Configuration as code**: Rather than TOML or JSON, project configuration is a Cadenza source file.

**Type-checked**: All configuration is validated by the type system.

**LSP support**: Get autocomplete, hover info, and errors inline while editing configuration.

**Imperative style**: Call builtin functions to configure the project, results accumulate in compiler state.

### Example Project.cdz

```
# Project.cdz

# Package metadata
package {
  name = "physics-sim",
  version = "2.1.0",
  authors = ["Physics Team <team@physics.org>"],
  description = "Physics simulation library",
  license = "Apache-2.0",
}

# Dependencies from various sources
dependency {
  name = "math",
  registry = "cadenza",
  version = "1.0",
}

dependency {
  name = "geometry",
  git = "https://github.com/user/cadenza-geo",
  tag = "v3.0.0",
}

# Can depend on non-Cadenza WASM components
dependency {
  name = "external-lib",
  registry = "wasm",
  version = "2.0",
}

# Artifact definitions
artifact {
  type = Lib,
  name = "physics",
  path = "src/lib.cdz",
}

artifact {
  type = Reactor,
  name = "physics-wasm",
  path = "src/lib.cdz",
}

artifact {
  type = Command,
  name = "simulator",
  path = "src/main.cdz",
}

# Special: test runner artifact
artifact {
  type = Command,
  name = "tests",
  path = "@tests",  # Compiler generates test harness
}

# Build configuration
profile "release" {
  verification = Hybrid,  # Contract checking mode
  optimization = Release,
  target = "wasm32-component",
}
```

### Configuration Schema

The compiler provides builtin functions with typed schemas:

**package builtin**:

- Type: `Record -> Nil`
- Required fields: `name`, `version`
- Optional fields: `authors`, `description`, `license`
- Accumulates: Package metadata in compiler state

**dependency builtin**:

- Type: `Record -> Nil`
- Required fields: `name`, `version` or `git`
- Source variants: `registry`, `git`, `path`
- Accumulates: Dependency list in compiler state

**artifact builtin**:

- Type: `Record -> Nil`
- Required fields: `type`, `name`, `path`
- Type values: `Lib`, `Reactor`, `Command`
- Accumulates: Artifact specifications in compiler state

**configure builtin**:

- Type: `Record -> Nil`
- Optional fields: `verification`, `optimization`, `target`
- Sets: Build configuration options

### Type Checking Project.cdz

**Special environment**: When evaluating Project.cdz, the compiler loads an environment with project-specific builtins:

1. Load standard builtins (for general computation)
2. Add project builtins (`package`, `dependency`, `artifact`, `profile`)
3. Evaluate Project.cdz with this environment
4. Collect accumulated configuration from compiler state

**Validation**: Type system ensures:

- Required fields are present
- Field types match schema
- Version strings are valid format
- Paths point to existing files

**Errors**: Configuration errors reported with full type information and source locations.

### LSP Support for Project.cdz

**Full IDE integration**:

**Autocomplete**: Field names in records, dependency names from registry, artifact types

**Hover**: Show field documentation, dependency descriptions, type information

**Diagnostics**: Show errors inline (missing fields, invalid versions, wrong types)

**Go-to-definition**: Jump to source file paths, follow dependency links

**Example LSP interaction**:

```
package {
  name = "my-app",
  vers  # ← LSP suggests: "version"
}

dependency {
  name = "unknown-pkg",  # ← Warning: "Package not found in registry"
  version = "1.0.0",
}

artifact {
  type = "libb",  # ← Error: "Unknown artifact type. Did you mean 'lib'?"
}
```

### Evaluation Order Independence

**Order doesn't matter**: Can call configuration functions in any order:

```
# These are equivalent:
artifact { ... }
package { ... }

# And:
package { ... }
artifact { ... }
```

**Multiple calls allowed**: Can define multiple artifacts, dependencies, etc.

**Final validation**: After evaluation completes, compiler validates complete configuration (e.g., must have at least one artifact).

### Compile-Time Computation in Project.cdz

**Full language access**: Project.cdz can use all Cadenza features:

```
# Compute values
let is_release = environment_var "RELEASE" == "true"

# Conditional configuration
profile "default" {
  verification = if is_release then Optimized else Dynamic,
  optimization = if is_release then Release else Debug,
}

# Generate artifact names programmatically
let app_name = "my-app"
artifact {
  type = Command,
  name = app_name,
  path = "src/main.cdz",
}
```

### Configuration and Caching

**Project.cdz is cached**: Like any source file, keyed by content hash.

**Invalidation**: Changing Project.cdz invalidates entire project compilation (dependencies might have changed).

**Optimization**: If only source files change (not Project.cdz), dependency resolution and WIT loading can be skipped.

## Testing Framework

Cadenza includes an integrated testing framework inspired by Rust's `#[test]` and property-based testing. Tests are written in Cadenza itself and discovered automatically.

### Test Declaration

**Mark with @test attribute**: Any function can become a test:

```
@test
fn test_addition =
  assert 1 + 1 == 2
  assert 2 + 2 == 4

@test "edge case: zero"
fn test_zero =
  assert 0 + 0 == 0

@test "negative numbers"
fn test_negatives =
  assert -1 + 1 == 0
  assert -5 + -3 == -8
```

**Test discovery**: Compiler scans for `@test` attributes during evaluation (Phase 2).

**Test collection**: Accumulated in compiler state along with function definitions.

### Test Organization

**File placement**: Tests can be inline with code or in separate test files:

**Inline tests**:

```
# math.cdz

fn add a b = a + b

@test
fn test_add = assert add 2 3 == 5
```

**Separate test files** (`.test.cdz` convention):

```
# math.test.cdz

@test
fn test_add_comprehensive =
  assert add 0 5 == 5
  assert add -2 3 == 1
```

**Tag-based organization**: Use `@tag` for filtering:

```
@test
@tag "unit"
@tag "arithmetic"
fn test_addition = assert 1 + 1 == 2

@test
@tag "integration"
@tag "database"
fn test_db_connection = assert connect_db "localhost"
```

### Test Runner Generation

**Special artifact**: The `@tests` path generates a test runner:

```
artifact {
  type = Command,
  name = "tests",
  path = "@tests",
}
```

**Generated runner**: Compiler creates a main function that:

1. Discovers all `@test` functions
2. Executes each test
3. Reports results (pass/fail count)
4. Exits with appropriate status code

**Execution**:

```bash
cadenza test              # Run all tests
cadenza test --tag unit   # Run only unit tests
cadenza test addition     # Run tests matching pattern
```

### Property-Based Testing

**Inspired by**: QuickCheck, Hypothesis, Rust's bolero

Property tests declare relationships that should hold for all inputs:

```
@property
fn prop_addition_commutative (a: Integer) (b: Integer) =
  assert a + b == b + a

@property
fn prop_reverse_twice (list: List Integer) =
  assert reverse (reverse list) == list
```

**Automatic generation**: Compiler generates arbitrary values for each parameter type.

**Configuration**: Control test parameters:

```
@property { cases = 1000, max_size = 100 }
fn prop_sort_idempotent (list: List Integer) =
  let sorted = sort list
  assert sort sorted == sorted
```

**Type-driven generation**: Uses type information to generate valid values:

- `Integer`: Random integers
- `Natural`: Only non-negative (respects constraints!)
- `List Integer`: Lists of random length with random integers
- `Record`: Records with random field values
- `Constrained types`: Only values satisfying constraints

### Shrinking

**When tests fail**: Automatically find minimal failing case.

**Strategy**: Try smaller/simpler inputs that still fail:

```
# Fails with: list = [1, 2, -5, 10, 3]
# Shrinks to: list = [-5]

@property
fn prop_all_positive (list: List Integer) =
  for x in list
    assert x > 0
```

**Shrinking strategies**:

- **Integers**: Binary search toward zero
- **Lists**: Remove elements, try subli sts
- **Strings**: Remove characters
- **Records**: Try simpler field values

### Integration with Contracts

**Synergy**: Contracts and tests work together:

**Constrained types guide generation**:

```
@invariant $0 >= 0
type Natural = Integer

@property
fn prop_natural_add (a: Natural) (b: Natural) =
  let result = a + b
  assert result >= 0  # Always true!
```

**Generator respects constraints**: Only generates valid `Natural` values (non-negative).

**Contract postconditions as test oracles**:

```
@ensures $0 >= 0
fn absolute_value x =
  if x < 0 then -x else x

# Property test verifies postcondition
@property
fn prop_absolute_value (x: Integer) =
  let result = absolute_value x
  assert result >= 0  # Checks postcondition holds
```

### Expected Failures

**Test functions that should panic**:

```
@test
@should_panic "division by zero"
fn test_divide_by_zero =
  let x = 1 / 0
  x  # Should panic before here
```

**Validation**: Test passes only if it panics with matching message.

### Test Execution Flow

**At compile time**:

1. Discover tests during evaluation
2. Collect in compiler state
3. Generate test runner artifact

**At runtime**:

1. Test runner calls each test function
2. Catches panics, records results
3. Reports pass/fail counts
4. Exits 0 if all pass, non-zero if any fail

**Property tests**:

1. Generate N random inputs (default 100)
2. Run property for each input
3. On failure, shrink to minimal case
4. Report minimal failing input

### Test Output

**Human-readable format**:

```
running 15 tests
test test_addition ... ok
test test_subtraction ... ok
test prop_reverse_twice ... ok (100 cases)
test prop_all_positive ... FAILED
  Minimal failing input: list = [-5]

test result: FAILED. 14 passed; 1 failed
```

**Machine-readable**: Can output JUnit XML or other formats for CI integration.

### Stateful Property Testing

**Future feature**: Test state machines with command sequences:

```
@property { stateful = true }
fn prop_bank_account =
  let model = BankAccount { balance = 100.0 }

  @command
  fn deposit (amount: PositiveFloat) =
    model.balance = model.balance + amount

  @command
  fn withdraw (amount: PositiveFloat) =
    if amount <= model.balance
      model.balance = model.balance - amount
```

**Framework generates**: Random command sequences, checks invariants hold.

## Code Generation

Code generation is the final phase where fully-analyzed Objects are converted to executable WASM. The process follows Carp's approach of walking annotated AST rather than transforming to a separate IR.

### Emission Strategy

**Walk annotated AST**: Like Carp's Emit.hs walking XObj, emit WASM by traversing Objects:

**For each Object**: Inspect annotations (type, ownership, deleters) and emit appropriate WASM instructions.

**Stateless emitter**: Code generation is a pure function from annotated AST to WASM bytes.

### WASM Component Structure

**Component model**: Output follows WASM component model, not core modules:

**Imports**:

- Allocator functions (`malloc`, `free`, `realloc`)
- Dependency functions (from imported WITs)

**Exports**:

- Functions from generated WIT interface
- For command artifacts: `_start` function

**Memory**:

- Single linear memory region
- Shared with allocator
- Grows as needed

**Type section**: Function signatures, struct layouts

### Memory Layout

**Struct layout**: Calculate field offsets based on type sizes and alignment:

- Integers: 8 bytes, 8-byte aligned
- Floats: 8 bytes, 8-byte aligned
- Rationals: 16 bytes (two 8-byte integers), 8-byte aligned
- References: 4 bytes (pointer), 4-byte aligned
- Strings: Pointer + length (8 bytes total)
- Lists: Pointer + length + capacity (16 bytes)

**Alignment**: Ensure fields are properly aligned for efficient access.

**Padding**: Insert padding bytes as needed for alignment.

### Expression Emission

**Literals**: Load constants directly (i64.const, f64.const)

**Variables**: Load from locals or memory based on scope

**Function calls**:

- Load arguments (respecting calling convention)
- Call instruction (direct or indirect for closures)
- Store result in local

**Let bindings**: Allocate locals, evaluate initializers, store values

**Conditionals**: Use structured control flow (if/else/end blocks)

**Loops**: Use loop/br instructions

**Deleters**: Emit free() calls at positions marked by ownership analysis

### Allocator Integration

**Pre-compiled allocator.wasm**: Provides memory management functions

**Linking**: User component imports from allocator:

```wat
(import "allocator" "malloc" (func $malloc (param i32 i32) (result i32)))
(import "allocator" "free" (func $free (param i32)))
(import "allocator" "realloc" (func $realloc (param i32 i32) (result i32)))
```

**Struct allocation**:

```wat
# Allocate struct Point { x: Float, y: Float }
(call $malloc (i32.const 16) (i32.const 8))  # 16 bytes, 8-byte aligned
```

**Deallocation**: At deleter positions, emit:

```wat
(call $free (local.get $ptr))
```

### Closure Representation

**After lambda lifting**: Closures become struct + function pointer:

**Struct**: Contains captured variables

**Function pointer**: Points to lifted function

**Calling convention**: Lifted function takes environment as first parameter

### String and List Representation

**Strings**: Pointer to UTF-8 bytes + length

```wat
struct String {
  ptr: i32,      # Pointer to bytes
  len: i32,      # Length in bytes
  cap: i32,      # Capacity (for growth)
}
```

**Lists**: Pointer to elements + length + capacity

```wat
struct List<T> {
  ptr: i32,      # Pointer to array
  len: i32,      # Number of elements
  cap: i32,      # Allocated capacity
}
```

### Optimization Opportunities

**Compile-time evaluation**: Constants computed at compile time, not runtime

**Dead code elimination**: Unused functions not emitted (from monomorphization)

**Inline small functions**: Can inline trivial functions

**Contract optimization**: Proven contracts removed (zero runtime cost)

**Future**: More sophisticated optimizations (CSE, loop optimization, etc.)

## LSP Integration

The LSP (Language Server Protocol) support provides IDE features through the same compiler used for batch compilation.

### Architecture

**Shared compiler**: LSP and batch mode use identical compilation pipeline.

**Hash-based caching**: Results cached and reused between LSP requests and builds.

**Incremental updates**: Only recompile changed files.

**Background checking**: Type checking happens in background, doesn't block editing.

### Core LSP Features

#### Hover

**Show type on hover**: Position cursor over identifier, see inferred type.

**Process**:

1. Find Object at cursor position
2. Extract type annotation from Object
3. Format type for display
4. Return hover text

**Example**: Hover over `x` in `let x = 1 + 1` shows `Type: Integer`.

#### Completion

**Autocomplete identifiers**: Type partial name, see suggestions.

**Sources**:

- Variables in current scope
- Functions from current module
- Imports from dependencies
- Keywords and builtins

**Type-aware**: Show function signatures, parameter hints.

**Context-sensitive**: Inside record, suggest field names.

#### Go to Definition

**Jump to source**: Click identifier, jump to definition.

**Process**:

1. Find Object at cursor
2. Look up binding in environment
3. Extract source location from Object
4. Return file path + position

**Cross-module**: Can jump to definitions in dependencies (if source available).

#### Diagnostics

**As-you-type errors**: Show errors while editing, no need to compile.

**Process**:

1. On file change, re-parse (fast, incremental)
2. Re-evaluate affected expressions
3. Re-type-check changed code
4. Update diagnostic list
5. Send to editor

**Multi-error**: Show all errors, not just first.

**Categories**: Parse errors, type errors, ownership errors, contract violations.

#### Semantic Tokens

**Syntax highlighting based on semantics**: Color code based on identifier meaning/type.

**Process**: Walk AST, emit token type for each identifier based on binding info.

### Caching Strategy

**Three-level cache**:

**Parse cache**: Content hash → CST

**Type cache**: (Object hash + type env hash) → typed Object

**Ownership cache**: Typed Object hash → owned Object

**Invalidation**: Content change invalidates parse cache, cascades to downstream caches.

**Sharing**: LSP and batch compiler share cache directory.

### Performance Targets

**Responsiveness goals**:

- Parse: < 50ms for typical files
- Type check: < 200ms for typical files
- Hover: < 50ms
- Completion: < 100ms

**Background work**: Heavy operations (monomorphization, WASM emission) happen in background, don't block editing.

**Cancellation**: Can cancel in-progress type checking if file changes again.

## Package Management

The package manager handles downloading, caching, and linking WASM components and Cadenza libraries.

### Package Registry

**Central registry**: Uses `npm` for registry, which is accessible via unpkg and jsDelivr in the browser.

**Versioning**: SemVer for compatibility guarantees.

**Metadata**: Package description, authors, license, keywords, WIT interface.

### Package Format

**Cadenza packages**: Tarball containing:

- Source files (`*.cdz`)
- Type metadata (`types.json`)
- Package manifest (`package.json`)
- Optional: Pre-compiled WASM for common platforms

**WASM components**: Standard WASM component binaries with WIT.

### Dependency Resolution

**SemVer resolution**: Find compatible versions automatically.

**Lockfile**: Records exact versions used (like `package-lock.json`).

**Transitive dependencies**: Automatically resolve entire dependency tree.

**Conflict resolution**: Multiple versions of same package allowed (different major versions).

### Publishing

**Prepare package**: Bundle source, metadata, optionally pre-compile WASM.

**Validation**: Ensure package compiles, tests pass, WIT is valid.

**Upload**: Push to registry with authentication.

**Versioning**: Cannot republish same version (immutable).

## Conclusion

This design document describes a complete compiler architecture for Cadenza, drawing on proven patterns from many successful languages while adding modern features like WIT integration, contracts, and LSP support.

### Key Architectural Decisions

**From Carp**:

- Linear type system with automatic memory management
- Constraint-based type inference
- Annotated AST (not separate IR)
- Monomorphization for generics
- Lambda lifting for closures

**New for Cadenza**:

- WASM components with WIT interfaces
- Project.cdz type-checked configuration
- Integrated property-based testing
- Hash-based incremental compilation
- LSP support throughout
- Rational numbers with dimensional analysis

### What Makes This Design Work

**Simplicity**: Single Object representation throughout pipeline

**Incrementality**: Hash-based caching at each phase

**Safety**: Linear types + ownership analysis prevent memory errors

**Interoperability**: WIT enables cross-language composition

**Tooling**: LSP shares compiler, no duplicate implementation

**Testing**: Built-in framework with property tests

**Correctness**: Contracts with optional formal verification
