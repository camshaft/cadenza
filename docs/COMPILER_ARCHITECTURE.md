# Cadenza Compiler Architecture

## Overview

This document describes the comprehensive architecture for the Cadenza compiler, covering type checking, multi-phase compilation, module system, code generation, LSP integration, and more. The design prioritizes incremental compilation, excellent error messages, and seamless integration between compile-time and runtime execution.

### Primary Use Cases

Cadenza is designed to make it easy to model and interact with things in the real world:

1. **3D Modeling**: Define everything in code (like OpenSCAD) to generate exportable models (STL, etc.)
2. **Algorithmic Music Composition**: Define compositions that can be listened to and exported
3. **Visual Art**: Create generative and interactive visual artworks
4. **Interactive Books**: Drive simulations and visualizations to describe concepts
5. **Quick Calculations**: Fast scratch computation with proper unit handling (e.g., "459174794 bytes/second to Gbps")

These use cases drive our focus on dimensional analysis, interactivity, and creative exploration inspired by Bret Victor's ideas.

## Table of Contents

1. [Design Principles](#design-principles)
2. [Multi-Phase Compiler Architecture](#multi-phase-compiler-architecture)
3. [Type System](#type-system)
4. [Module System](#module-system)
5. [Monomorphization and Trait System](#monomorphization-and-trait-system)
6. [Effect System](#effect-system)
7. [Compilation Targets](#compilation-targets)
8. [LSP Integration](#lsp-integration)
9. [MCP Integration](#mcp-integration)
10. [Error Handling and Source Tracking](#error-handling-and-source-tracking)

---

## Design Principles

### Core Philosophy

1. **Compile-time and Runtime Blur**: The evaluator IS the compiler's first phase. Code runs at compile-time to generate more code.
2. **Incremental Everything**: Parsing, evaluation, type checking, and code generation should all be incremental.
3. **Errors are First-Class**: Multi-error reporting, beautiful diagnostics, and precise source tracking throughout.
4. **LSP-Native**: The compiler and LSP server are the same thing—one is queryable, one is batch.
5. **Units and Dimensions**: Type system includes dimensional analysis for physical units.
6. **Homoiconic Syntax**: The language AST is simple enough to be manipulated by the language itself, enabling user macros and AST quote/splicing.
7. **Minimal Lexing/Parsing Rules**: Everything is function application (similar to Lisp). The parser doesn't specialize on keywords—identifiers get their semantic meaning at evaluation time.
8. **Real-World Modeling**: Make it easy to model real-world things with dimension analysis and interactive exploration.

### Non-Goals

- Runtime type information (RTTI) in compiled code
- Dynamic typing in the language itself
- Backward compatibility during early development

---

## Multi-Phase Compiler Architecture

The compiler operates in distinct phases, with each phase building on the previous:

```
┌─────────────────────────────────────────────────────────────────┐
│                         Source Code                              │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  Phase 1: Lexing & Parsing                                       │
│  - Tokenize source                                               │
│  - Build lossless CST (rowan)                                    │
│  - Preserve all whitespace and comments                          │
│  - Output: CST with full source tracking                         │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  Phase 2: Evaluation (Macro Expansion & Module Building)         │
│  - Tree-walk interpreter evaluates top-level expressions         │
│  - Macro expansion happens here                                  │
│  - Functions/types are accumulated in Compiler state             │
│  - Unevaluated branches are collected for later type checking    │
│  - Output: Compiler state + AST nodes for exports                │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  Phase 3: Type Checking (Hindley-Milner)                         │
│  - Collect type constraints from all code paths                  │
│  - Include unevaluated branches (guards, conditionals)           │
│  - Infer types using constraint solving                          │
│  - Check dimensional analysis for unit types                     │
│  - Validate trait requirements                                   │
│  - Validate effect contexts                                      │
│  - Output: Typed AST + Type signatures for exports               │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  Phase 4: Monomorphization                                       │
│  - Instantiate generic functions with concrete types             │
│  - Resolve trait requirements to concrete implementations        │
│  - Specialize code for each type usage                           │
│  - Output: Monomorphic IR                                        │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  Phase 5: IR Lowering & Optimization                             │
│  - Convert to target-independent IR                              │
│  - Perform optimizations (constant folding, DCE, etc.)           │
│  - Prepare for code generation                                   │
│  - Output: Optimized IR                                          │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  Phase 6: Code Generation                                        │
│  - Target-specific code generation                               │
│  - See "Compilation Targets" section                             │
│  - Output: Executable/WASM/JS/etc.                               │
└─────────────────────────────────────────────────────────────────┘
```

### Key Architectural Decisions

#### Why This Phase Ordering?

**See `/docs/ARCHITECTURE_REVIEW.md` for detailed analysis and comparison with other languages.**

The phase ordering `Parse → Evaluate (Macros) → Type Check → IR → Optimize → Codegen` is the correct approach based on:

1. **Research across multiple languages** (Rust, Julia, Zig, Lisp, etc.) shows this is the universal pattern
2. **All use cases are supported** by this architecture (verified against use case documents)
3. **No design corners** - the architecture allows future evolution without backing us into constraints

#### Evaluation Before Type Checking

The evaluator runs **before** type checking because:

1. **Macros generate code**: Macros run at compile-time and produce new AST nodes that must be type-checked.
2. **Compile-time computation**: Some types may depend on compile-time computations (e.g., array sizes, unit conversions).
3. **Module building**: The evaluator accumulates exports into the Compiler state, which the type checker validates.

**Why not do macros after IR?** 
- Macros operate on AST (high-level code structures like functions, data structures)
- IR is too low-level (SSA form, basic blocks, phi nodes)
- No language does IR → Macros (all do Macros → IR)
- Would require reverse-engineering high-level constructs from low-level IR

**Note on error handling**: If evaluation fails or produces errors, those diagnostics are recorded in the Compiler state but don't prevent type checking. The type checker operates on both successfully evaluated code and unevaluated branches, ensuring we get comprehensive type errors even when evaluation issues exist. This provides better overall error reporting than stopping at the first evaluation failure.

#### Type Checking Before IR

Type checking happens **before** IR generation (not after) because:

1. **Better error messages**: Can reference source-level constructs rather than IR
2. **Type-guided IR generation**: Type information helps generate better IR
3. **Follows established patterns**: Rust, Julia, and others check types before generating IR
4. **Enables type-directed optimizations**: Monomorphization and specialization work on typed AST

**Why not check types on IR?**
- Harder to provide good error messages (IR is far from source)
- Lose semantic information that aids type checking
- More complex to implement correctly

#### Handling Unevaluated Branches

**Problem**: If the interpreter encounters a conditional that isn't evaluated (e.g., `if false then ... else ...`), we still need to type-check the unevaluated branch.

**Solution**: During evaluation, track all code paths:

```rust
enum EvaluationResult {
    /// Code was evaluated to a value
    Evaluated(Value),
    /// Code was not evaluated but should be type-checked
    Unevaluated(Expr),
}
```

The evaluator emits both evaluated values AND unevaluated expressions to the compiler. The type checker processes both:
- Evaluated paths: Type check against the actual runtime value
- Unevaluated paths: Infer types without executing

**Example**:
```cadenza
let config = load_config "config.txt"  # Evaluated at compile-time

# Note: Syntax for conditionals is still being designed
# This is illustrative of the concept of type-checking unevaluated branches
let process =
    if config.mode == "debug"
        fn x -> debug_print x  # Might not be evaluated
    else
        fn x -> x              # Might not be evaluated
```

Both branches are type-checked even if only one is evaluated.

#### IR Generation Timing

**Implementation detail**: In the current implementation, the IR generator optionally collects information during evaluation (when `Compiler::with_ir()` is used), but the complete IR module is only finalized and retrieved **after** evaluation completes via `Compiler::build_ir_module()`.

This design:
- ✅ Keeps IR generation optional (REPL/LSP don't need it)
- ✅ Allows incremental IR building during evaluation
- ✅ Finalizes IR only after evaluation is complete
- ✅ Maintains clear phase separation

The key principle: **Macros must fully expand before any IR is finalized**. The current implementation respects this.

#### AST Representation

**Question**: Do we use the same AST for the interpreter and for exports to the compiler?

**Answer**: Yes, with annotations.

```rust
pub struct TypedExpr {
    /// The original expression AST node
    expr: Expr,
    /// The inferred type (filled in by type checker)
    ty: Type,
    /// Source location
    span: Span,
}
```

- **Interpreter**: Uses raw `Expr` nodes from the parser
- **After evaluation**: Expressions are annotated with values/types
- **Type checker**: Adds type information to create `TypedExpr`
- **Exports**: Use `TypedExpr` for cross-module type information

This allows:
- Interpreter to work with simple AST
- Type checker to annotate without rebuilding
- Exports to carry full type information

---

## Type System

### Hindley-Milner Type Inference

We use Hindley-Milner (HM) type inference with extensions for:
- Units and dimensional analysis
- Traits and type classes
- Effect types
- Row polymorphism (for records)

#### Type Representation

```rust
pub enum Type {
    /// Concrete types
    Nil,
    Bool,
    Integer,
    Float,
    String,
    
    /// Parametric types
    List(Box<Type>),
    Tuple(Vec<Type>),
    Record(Vec<(InternedString, Type)>),
    
    /// Function types: args... -> return
    Fn(Vec<Type>, Box<Type>),
    
    /// Type variables (for inference)
    Var(TypeVar),
    
    /// Quantified types (for polymorphism)
    Forall(Vec<TypeVar>, Box<Type>),
    
    /// Units and quantities
    Quantity {
        value_type: Box<Type>,  // Integer or Float
        dimension: Dimension,
    },
    
    /// Trait constraints
    Constrained {
        ty: Box<Type>,
        traits: Vec<TraitRef>,
    },
    
    /// Effect types
    Effect {
        result: Box<Type>,
        effects: Vec<EffectRef>,
    },
}
```

#### Type Inference Algorithm

We implement Algorithm W (Damas-Milner) with constraint generation:

1. **Constraint Generation**: Walk the AST and generate type constraints
   ```rust
   fn generate_constraints(expr: &Expr, env: &TypeEnv) -> (Type, Vec<Constraint>)
   ```

2. **Constraint Solving**: Use unification to solve constraints
   ```rust
   fn unify(t1: Type, t2: Type) -> Result<Substitution>
   ```

3. **Generalization**: Introduce `forall` quantifiers at let-bindings
   ```rust
   fn generalize(ty: Type, env: &TypeEnv) -> Type
   ```

4. **Instantiation**: Replace quantified variables with fresh type variables
   ```rust
   fn instantiate(ty: Type) -> Type
   ```

#### Dimensional Analysis Integration

Unit types are checked during type inference:

```cadenza
let distance = 100meter
let time = 5second
let velocity = distance / time  # Type: Quantity<Float, meter/second>

# Type error: cannot add incompatible dimensions
let invalid = distance + time  # Error: cannot add meter and second
```

The type checker maintains dimension equations and solves them alongside type constraints:

```rust
pub struct DimensionConstraint {
    left: Dimension,
    right: Dimension,
    reason: ConstraintReason,
}
```

#### Type Checking Unevaluated Code

When we encounter an unevaluated branch:

1. **Mark as speculative**: Tag the type checking context
2. **Generate constraints**: Same as evaluated code
3. **Solve constraints**: Ensure type soundness
4. **Record result**: Store inferred types but don't generate code

```rust
impl TypeChecker {
    fn check_expr(&mut self, expr: &Expr, expected: Type, mode: CheckMode) -> Result<Type> {
        match mode {
            CheckMode::Evaluated => {
                // Normal type checking + code generation
            }
            CheckMode::Unevaluated => {
                // Type checking only, no code generation
                // Still ensures type soundness
            }
        }
    }
}
```

### Type Errors

Type errors include:
- Expected vs actual type
- Source location with full context
- Suggestion for fixes when possible
- Stack trace showing call chain

---

## Module System

### Module Structure

```cadenza
# mymodule.cdz

# Private function (prefix with _ to hide)
fn _internal_helper x = x + 1

# Public function (exported by default)
fn public_fn x = x * 2

# Public type/measure
measure meter

# Public macro
defmacro my_macro args = ...
```

**Export semantics**: Functions and definitions are exported by default. Prefix names with `_` to make them private.

**Artifacts**: The `@export` attribute is used to name artifacts (models, compositions, etc.) that can be exported from a running program (e.g., STL files for 3D models, audio files for compositions).

### Import/Export Mechanism

#### Exports

Modules export all top-level definitions by default (except those prefixed with `_`). The evaluator collects these into the compiler state:

```rust
pub struct ModuleExports {
    /// Exported functions with type signatures
    pub functions: Map<InternedString, TypedFunction>,
    
    /// Exported types
    pub types: Map<InternedString, Type>,
    
    /// Exported macros (evaluated at import site)
    pub macros: Map<InternedString, Macro>,
    
    /// Exported units/measures
    pub units: Map<InternedString, Unit>,
}
```

#### Imports

Modules are treated as records, allowing destructuring and natural access patterns:

```cadenza
# Destructuring import (modules are records)
import { public_fn, meter } = mymodule

# Import entire module
import mymodule

# Qualified import with alias
import m = mymodule

# Access module members using dot notation
let x = m.public_fn 10  # Dot operator works naturally with records
```

The type checker ensures:
1. Imported names exist in the source module
2. Type signatures match at the boundary
3. No circular dependencies

#### Cross-Module Type Checking

When importing:

1. **Load module**: Parse and evaluate the imported module
2. **Extract types**: Get type signatures from exports
3. **Check usage**: Verify imported values are used correctly

```rust
pub struct ModuleRegistry {
    /// Loaded modules with their exports
    modules: Map<InternedString, ModuleExports>,
    
    /// Dependency graph for cycle detection
    dependencies: Map<InternedString, Vec<InternedString>>,
}
```

Circular dependencies are detected during evaluation phase before type checking.

---

## Monomorphization and Trait System

### Goals

1. **Generic functions**: Write functions once for many types
2. **Implicit traits**: No manual trait annotations (infer requirements)
3. **Zero-cost abstractions**: Specialized code at compile-time
4. **Type classes**: Support ad-hoc polymorphism like Haskell

### Trait Definition

```cadenza
# Define a trait (basic version)
trait Numeric =
  fn add Self Self = Self
  fn mul Self Self = Self
  fn zero = Self

# Define a trait with generics for more complex patterns
trait Numeric Rhs Output =
  fn add Self Rhs = Output
  fn mul Self Rhs = Output
  fn zero = Self

# Implement trait for a type
impl Integer = Numeric
  fn add a b = a + b
  fn mul a b = a * b
  fn zero = 0
```

**Note**: The trait system complexity can snowball quickly with generics. The exact syntax and feature set is still being designed to balance power with simplicity.

### Implicit Trait Constraints

**Key Innovation**: Don't require explicit trait bounds. Instead, **infer** them.

```cadenza
# User writes this:
let sum = fn list ->
    fold list 0 (fn acc x -> acc + x)

# Compiler infers:
let sum : forall a. [Numeric a] => List a -> a
```

The type checker:
1. Encounters `acc + x`
2. Requires both operands support `+`
3. Generates constraint: `a : Numeric`
4. Unifies with call sites

#### Trait Constraint Inference Algorithm

```rust
fn infer_traits(expr: &Expr, ty: Type) -> Vec<TraitConstraint> {
    match expr {
        // a + b requires Numeric
        Expr::Apply(op, [a, b]) if op == "+" => {
            vec![TraitConstraint::new(ty, Trait::Numeric)]
        }
        
        // Recursive inference
        Expr::Apply(f, args) => {
            let f_constraints = infer_traits(f, ...);
            let arg_constraints = args.iter().flat_map(|arg| infer_traits(arg, ...));
            f_constraints.chain(arg_constraints).collect()
        }
        
        _ => vec![]
    }
}
```

### Monomorphization

After type checking, we have:
```
fn sum : forall a. [Numeric a] => List a -> a
```

At call sites:
```cadenza
sum [1, 2, 3]        # Instantiate with a = Integer
sum [1.0, 2.0, 3.0]  # Instantiate with a = Float
```

Monomorphization generates specialized versions:

```rust
// Generated code:
fn sum_Integer(list: List<Integer>) -> Integer { ... }
fn sum_Float(list: List<Float>) -> Float { ... }
```

#### Monomorphization Algorithm

1. **Collect instantiations**: Track all call sites
2. **Generate specialized functions**: Create concrete versions
3. **Replace calls**: Update call sites to use specialized functions
4. **Dead code elimination**: Remove unused specializations

```rust
pub struct Monomorphizer {
    /// Map from generic function to instantiations
    instantiations: Map<FunctionId, Vec<Instantiation>>,
    
    /// Generated specialized functions
    specialized: Vec<MonomorphicFunction>,
}

impl Monomorphizer {
    fn instantiate(&mut self, func: &TypedFunction, types: Vec<Type>) -> FunctionId {
        // Check if already instantiated
        if let Some(id) = self.find_instantiation(func.id, &types) {
            return id;
        }
        
        // Create new specialized function
        let specialized = self.specialize(func, types.clone());
        // Use vector length as ID (index where it will be inserted)
        let id = FunctionId(self.specialized.len());
        self.specialized.push(specialized);
        self.instantiations.entry(func.id).or_default().push(Instantiation { types, id });
        id
    }
}
```

---

## Effect System

### Motivation

Effects represent computational context (like Reader monad in Haskell):
- Database connections
- Logging
- Configuration
- Permissions

**Key idea**: Effects are implicit parameters checked at compile-time.

### Effect Definition

```cadenza
# Define an effect
# Note: Exact syntax still being designed
effect Logger =
  fn log String = ()

# Function using an effect
fn process data =
    Logger.log "Processing"
    # ... do work
```

**Design considerations**:
- How to handle state updates in the effect system? Should effects act as reducers returning next state?
- Effects need to associate state (e.g., file descriptor for logging to a file)
- Balance between pure immutability and practical state management

### Implicit Effect Propagation

The compiler infers effect requirements:

```cadenza
let top_level = fn data ->
    let result = process data  # process requires Logger
    result                      # So top_level also requires Logger
```

Inferred signature:
```
top_level : forall a. [Logger] => a -> a
```

### Effect Handlers

Provide effect implementations at call sites:

```cadenza
# Define a handler
let console_logger = handler Logger
    log = fn msg -> print "LOG: " msg

# Use the handler
with console_logger
    top_level my_data
```

### Effect Type Checking

Effects extend the type system:

```rust
pub enum Type {
    // ...
    Effect {
        result: Box<Type>,
        effects: Vec<EffectRef>,
    },
}
```

Type checking ensures:
1. Required effects are provided
2. Effect handlers match effect signatures
3. Effects are propagated correctly

```rust
pub struct EffectConstraint {
    /// Required effect
    effect: EffectRef,
    /// Source expression needing the effect
    source: Expr,
    /// Where effect must be provided
    boundary: Span,
}
```

The type checker verifies all effect constraints are satisfied before the top-level boundary.

---

## Compilation Targets

### Target Requirements

1. **Browser (in-browser compilation and execution)**
   - Must compile entirely in the browser
   - No server-side processing required
   - Options: TypeScript, JavaScript, WASM

2. **Standalone Executable**
   - Native performance
   - No runtime dependencies
   - Options: LLVM, Rust, C, Zig, Cranelift

3. **Nice-to-have: Microcontrollers**
   - Embedded systems
   - Resource-constrained environments

### Browser Targets

#### Option 1: WebAssembly (WASM)

**Pros**:
- Near-native performance
- Compact binary format
- Security sandbox
- Well-supported

**Cons**:
- Requires runtime (WASM engine)
- FFI with JS can be complex
- Debugging harder than JS
- **Major challenge**: Substantial effort to efficiently lower high-level language to WASM (allocations, lifetime management, memory safety)

**Approach**: Use `wasm32-unknown-unknown` target, compile IR to WASM bytecode.

```rust
pub struct WasmBackend {
    module: wasm_encoder::Module,
}

impl WasmBackend {
    fn compile_function(&mut self, func: &MonomorphicFunction) {
        // Emit WASM instructions
    }
}
```

#### Option 2: TypeScript

**Pros**:
- Type safety
- Good tooling
- Easy debugging
- Source maps
- Less lowering work than WASM

**Cons**:
- Requires transpilation step
- Larger output than WASM
- TypeScript runtime needed

**Approach**: Emit TypeScript code from IR, use `tsc` or `esbuild` for bundling.

#### Option 3: JavaScript

**Pros**:
- No compilation needed
- Universal browser support
- Easy debugging
- Minimal lowering required

**Cons**:
- No static type checking
- Larger output
- Less efficient than WASM
- **Concern**: Lack of actual integers causes inconsistent runtime behavior across environments

**Approach**: Emit JS code directly from IR.

#### Recommendation for Browser

**Prioritize high-level targets**:
- **Primary**: Target TypeScript or JavaScript to minimize backend complexity
  - Much less lowering work required
  - Faster development iteration
  - Easier debugging
  - Good enough performance for most use cases
  
- **Optional**: WASM for performance-critical applications
  - When the substantial lowering effort is justified
  - Near-native performance needed

The compiler can target multiple backends from the same IR, allowing flexibility based on requirements.

### Standalone Targets

#### Option 1: LLVM

**Pros**:
- Industry-standard
- Excellent optimizations
- Many target architectures
- Used by Rust, Swift, etc.

**Cons**:
- Large dependency
- Complex API
- Longer compile times
- **Major challenge**: Substantial lowering work similar to WASM

**Approach**: Use `inkwell` crate for LLVM bindings.

```rust
pub struct LlvmBackend<'ctx> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
}
```

#### Option 2: Emit Rust

**Pros**:
- Leverage Rust compiler
- Excellent error messages
- Already set up in our workflow
- Good debugging
- **Minimal lowering work**: Rust is high-level, less backend complexity

**Cons**:
- Requires Rust toolchain
- Slower compilation than Cranelift
- Not truly standalone

**Approach**: Emit Rust code from IR, invoke `rustc`. Likely the least amount of backend work required.

#### Option 3: Emit C

**Pros**:
- Universal
- Highly portable
- Mature toolchain
- Works everywhere

**Cons**:
- Manual memory management
- Less type safety
- Older language

**Approach**: Emit C code from IR, invoke `gcc` or `clang`.

#### Option 4: Cranelift

**Pros**:
- Fast compilation
- Designed for JIT/AOT
- Used by Wasmtime
- Pure Rust

**Cons**:
- Less optimization than LLVM
- Fewer target architectures
- Younger project
- **Similar challenge**: Still requires substantial lowering work

**Approach**: Use Cranelift as a library for code generation.

```rust
pub struct CraneliftBackend {
    module: cranelift_module::Module<cranelift_module::ObjectBackend>,
}
```

#### Recommendation for Standalone

**Primary**: Emit Rust
- Minimal backend work (high-level target)
- Leverages existing Rust compiler infrastructure
- Good performance via rustc optimizations
- Easier to implement and maintain

**Alternative**: Cranelift
- Fast compilation (important for LSP)
- Pure Rust integration
- More control over codegen
- Requires substantial lowering effort

**Optional**: LLVM
- Best optimizations for production
- Many target architectures
- Enable with feature flag when needed
- Most complex lowering work

### Intermediate Representation (IR)

All backends consume the same IR. The IR is designed to be:
- Simple to generate from typed AST
- Easy to optimize
- Target-independent
- Translates cleanly to all backends

#### IR Design

```rust
/// Intermediate Representation for Cadenza
/// This is a typed, SSA-like IR suitable for optimization and code generation

/// Source location for tracking origins of IR nodes
/// Used to generate source maps for JavaScript and accurate stack traces
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceLocation {
    pub file: InternedString,
    pub line: u32,
    pub column: u32,
}

/// A unique identifier for values in SSA form
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ValueId(u32);

/// A unique identifier for basic blocks
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(u32);

/// A unique identifier for functions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FunctionId(u32);

/// IR instruction
pub enum IrInstr {
    /// Load a constant value
    /// %result = const <value>
    Const {
        result: ValueId,
        value: IrConst,
        source: SourceLocation,  // Track file name and line number
    },
    
    /// Binary operation
    /// %result = binop <op> %lhs %rhs
    BinOp {
        result: ValueId,
        op: BinOp,
        lhs: ValueId,
        rhs: ValueId,
        source: SourceLocation,
    },
    
    /// Unary operation
    /// %result = unop <op> %operand
    UnOp {
        result: ValueId,
        op: UnOp,
        operand: ValueId,
        source: SourceLocation,
    },
    
    /// Function call
    /// %result = call <func> (%arg1, %arg2, ...)
    Call {
        result: Option<ValueId>,  // None for void returns
        func: FunctionId,
        args: Vec<ValueId>,
        source: SourceLocation,
    },
    
    /// Create a record
    /// %result = record { field1: %val1, field2: %val2, ... }
    /// Field names stored separately from values for efficient cloning
    /// field_values[i] corresponds to field_names[i]
    Record {
        result: ValueId,
        field_names: Arc<[InternedString]>,  // Shared across all instances of this record type
        field_values: Vec<ValueId>,           // Parallel array: values[i] is value for names[i]
        source: SourceLocation,
    },
    
    /// Field access
    /// %result = field %record .field_name
    Field {
        result: ValueId,
        record: ValueId,
        field: InternedString,
        source: SourceLocation,
    },
    
    /// Create a list/tuple
    /// %result = tuple (%elem1, %elem2, ...)
    Tuple {
        result: ValueId,
        elements: Vec<ValueId>,
        source: SourceLocation,
    },
    
    /// Conditional branch
    /// br %cond, then: <then_block>, else: <else_block>
    Branch {
        cond: ValueId,
        then_block: BlockId,
        else_block: BlockId,
        source: SourceLocation,
    },
    
    /// Unconditional jump
    /// jmp <target_block>
    Jump {
        target: BlockId,
        source: SourceLocation,
    },
    
    /// Return from function
    /// ret %value
    Return {
        value: Option<ValueId>,  // None for void functions
        source: SourceLocation,
    },
    
    /// Phi node for SSA (join point for values from different blocks)
    /// %result = phi [%val1 from <block1>], [%val2 from <block2>], ...
    Phi {
        result: ValueId,
        incoming: Vec<(ValueId, BlockId)>,
        source: SourceLocation,
    },
}

/// Constants in IR
pub enum IrConst {
    Nil,
    Bool(bool),
    Integer(i64),
    Float(f64),
    String(InternedString),
    /// Quantity with dimension (e.g., 5.0 meters)
    Quantity {
        value: f64,
        dimension: Dimension,
    },
}

/// Binary operators
pub enum BinOp {
    // Arithmetic
    Add, Sub, Mul, Div, Rem,
    
    // Comparison
    Eq, Ne, Lt, Le, Gt, Ge,
    
    // Logical
    And, Or,
    
    // Bitwise (future)
    BitAnd, BitOr, BitXor, Shl, Shr,
}

/// Unary operators
pub enum UnOp {
    Neg,    // Numeric negation
    Not,    // Logical not
    BitNot, // Bitwise not (future)
}

/// Basic block - sequence of instructions
pub struct IrBlock {
    pub id: BlockId,
    pub instructions: Vec<IrInstr>,
    pub terminator: IrTerminator,
}

/// Block terminators (control flow instructions)
pub enum IrTerminator {
    Branch {
        cond: ValueId,
        then_block: BlockId,
        else_block: BlockId,
    },
    Jump {
        target: BlockId,
    },
    Return {
        value: Option<ValueId>,
    },
}

/// Function in IR
pub struct IrFunction {
    pub id: FunctionId,
    pub name: InternedString,
    pub params: Vec<IrParam>,
    pub return_ty: Type,
    pub blocks: Vec<IrBlock>,
    pub entry_block: BlockId,
}

/// Function parameter
pub struct IrParam {
    pub name: InternedString,
    pub ty: Type,
    pub value_id: ValueId,  // SSA value for this parameter
}

/// Complete IR module
pub struct IrModule {
    pub functions: Vec<IrFunction>,
    pub exports: Vec<IrExport>,
}

/// Exported items from a module
pub struct IrExport {
    pub name: InternedString,
    pub kind: IrExportKind,
}

pub enum IrExportKind {
    Function(FunctionId),
    Constant(ValueId),
}
```

#### IR Generation Strategy

The IR is generated after type checking and monomorphization:

1. **From Typed AST**: Convert type-checked expressions to IR
2. **SSA Form**: All values are assigned once (Single Static Assignment)
3. **Basic Blocks**: Group instructions into basic blocks
4. **Explicit Control Flow**: Branches and jumps are explicit

#### Example IR Generation

**Source Cadenza Code**:
```cadenza
fn add_one x = x + 1

let result = add_one 5
```

**Generated IR**:
```
; Function: add_one
function @add_one(%x: Integer) -> Integer {
  block_0 (entry):
    %0 = const 1
    %1 = binop Add %x %0
    ret %1
}

; Top-level evaluation
function @__main() -> Integer {
  block_0 (entry):
    %0 = const 5
    %1 = call @add_one(%0)
    ret %1
}
```

#### Source Tracking for Debugging

All IR nodes include `SourceLocation` to track the original source file and line number. This enables:

**JavaScript Source Maps**:
- Generate accurate source maps during JS/TS compilation
- Debuggers show original Cadenza code, not generated JavaScript
- Stack traces reference Cadenza source locations

**Rust Debugging** (future):
- May require proc macro assistance for line number mapping
- Preserve source information in generated Rust code
- Enable better error messages and debugging experience

**Stack Trace Accuracy**:
- Runtime errors show exact Cadenza source location
- Error messages reference original code, not IR or target language
- Improved developer experience across all backends

#### Optimization Pipeline

Once IR is generated, we can apply optimization passes:

```rust
pub trait IrOptimizationPass {
    fn run(&mut self, module: &mut IrModule) -> bool;  // Returns true if changed
}

/// Optimization pipeline - configurable set of passes
pub struct IrOptimizer {
    passes: Vec<Box<dyn IrOptimizationPass>>,
}

impl IrOptimizer {
    pub fn new() -> Self {
        Self { passes: vec![] }
    }
    
    pub fn add_pass(&mut self, pass: Box<dyn IrOptimizationPass>) {
        self.passes.push(pass);
    }
    
    pub fn run(&mut self, module: &mut IrModule) {
        let mut changed = true;
        let mut iterations = 0;
        const MAX_ITERATIONS: usize = 10;
        
        while changed && iterations < MAX_ITERATIONS {
            changed = false;
            for pass in &mut self.passes {
                changed |= pass.run(module);
            }
            iterations += 1;
        }
    }
}
```

#### Standard Optimization Passes

**Constant Folding**:
```rust
pub struct ConstantFolding;

impl IrOptimizationPass for ConstantFolding {
    fn run(&mut self, module: &mut IrModule) -> bool {
        let mut changed = false;
        for func in &mut module.functions {
            for block in &mut func.blocks {
                for instr in &mut block.instructions {
                    if let IrInstr::BinOp { result, op, lhs, rhs } = instr {
                        // If both operands are constants, compute at compile time
                        if is_const(*lhs) && is_const(*rhs) {
                            let folded = fold_binop(*op, get_const(*lhs), get_const(*rhs));
                            *instr = IrInstr::Const { result: *result, value: folded };
                            changed = true;
                        }
                    }
                }
            }
        }
        changed
    }
}
```

**Dead Code Elimination (DCE)**:
```rust
pub struct DeadCodeElimination;

impl IrOptimizationPass for DeadCodeElimination {
    fn run(&mut self, module: &mut IrModule) -> bool {
        let mut changed = false;
        for func in &mut module.functions {
            // Mark all used values
            let mut used = HashSet::new();
            mark_used(func, &mut used);
            
            // Remove instructions that produce unused values
            for block in &mut func.blocks {
                block.instructions.retain(|instr| {
                    if let Some(result) = instr.result_value() {
                        if !used.contains(&result) {
                            changed = true;
                            return false;
                        }
                    }
                    true
                });
            }
        }
        changed
    }
}
```

**Common Subexpression Elimination (CSE)**:
```rust
pub struct CommonSubexpressionElimination;

impl IrOptimizationPass for CommonSubexpressionElimination {
    fn run(&mut self, module: &mut IrModule) -> bool {
        // Find expressions computed multiple times
        // Replace redundant computations with references to first result
        // Requires value numbering and equivalence analysis
        todo!("CSE implementation")
    }
}
```

**Function Inlining**:
```rust
pub struct FunctionInlining {
    max_size: usize,  // Only inline small functions
}

impl IrOptimizationPass for FunctionInlining {
    fn run(&mut self, module: &mut IrModule) -> bool {
        // Inline small functions at call sites
        // Replace call instruction with function body
        // Update value IDs and blocks
        todo!("Inlining implementation")
    }
}
```

#### Creating an Optimization Pipeline

```rust
pub fn create_optimization_pipeline(level: OptLevel) -> IrOptimizer {
    let mut optimizer = IrOptimizer::new();
    
    match level {
        OptLevel::None => {
            // No optimizations
        }
        OptLevel::Basic => {
            optimizer.add_pass(Box::new(ConstantFolding));
            optimizer.add_pass(Box::new(DeadCodeElimination));
        }
        OptLevel::Aggressive => {
            optimizer.add_pass(Box::new(ConstantFolding));
            optimizer.add_pass(Box::new(CommonSubexpressionElimination));
            optimizer.add_pass(Box::new(FunctionInlining { max_size: 50 }));
            optimizer.add_pass(Box::new(DeadCodeElimination));
            // Multiple passes may discover new opportunities
        }
    }
    
    optimizer
}

pub enum OptLevel {
    None,
    Basic,
    Aggressive,
}
```

---

### IR Emission from Evaluator

The evaluator can optionally emit IR during or after evaluation:

```rust
pub struct Compiler {
    // ... existing fields ...
    
    /// IR module being built
    ir_module: Option<IrModule>,
    
    /// Whether to emit IR
    emit_ir: bool,
}

impl Compiler {
    pub fn emit_ir(&mut self, enable: bool) {
        self.emit_ir = enable;
        if enable && self.ir_module.is_none() {
            self.ir_module = Some(IrModule::new());
        }
    }
    
    pub fn take_ir(&mut self) -> Option<IrModule> {
        self.ir_module.take()
    }
}

/// Extension trait for converting typed expressions to IR
pub trait ToIr {
    fn to_ir(&self, builder: &mut IrBuilder) -> ValueId;
}

pub struct IrBuilder {
    current_function: FunctionId,
    current_block: BlockId,
    next_value_id: u32,
    next_block_id: u32,
    value_map: HashMap<InternedString, ValueId>,  // Variable name -> SSA value
}

impl IrBuilder {
    fn new_value(&mut self) -> ValueId {
        let id = ValueId(self.next_value_id);
        self.next_value_id += 1;
        id
    }
    
    fn new_block(&mut self) -> BlockId {
        let id = BlockId(self.next_block_id);
        self.next_block_id += 1;
        id
    }
    
    fn emit(&mut self, instr: IrInstr) {
        // Add instruction to current block
        todo!()
    }
}
```

---

## LSP Integration

### Core Principle

**The LSP server IS the compiler.**

Instead of having separate tools:
- Compiler: batch mode
- LSP: query mode

They share the same codebase and state.

### Architecture

```
┌────────────────────────────────────────────────────────┐
│                    LSP Server                          │
│                                                        │
│  ┌──────────────────────────────────────────────────┐ │
│  │         Incremental Compiler                     │ │
│  │  - Maintains parsed trees                        │ │
│  │  - Tracks evaluation state                       │ │
│  │  - Caches type information                       │ │
│  │  - Incrementally updates on changes              │ │
│  └──────────────────────────────────────────────────┘ │
│                        │                               │
│  ┌─────────────────────┴─────────────────────────┐    │
│  │                                               │    │
│  ▼                                               ▼    │
│  LSP Protocol Handlers              Compiler State    │
│  - textDocument/hover                - Parsed trees   │
│  - textDocument/completion           - Type info      │
│  - textDocument/definition           - Errors         │
│  - textDocument/semanticTokens       - Symbols        │
└────────────────────────────────────────────────────────┘
```

### Incremental Compilation

Use **rowan** (red-green tree) for incremental parsing:

```rust
pub struct IncrementalCompiler {
    /// Current CST (green tree)
    cst: GreenNode,
    
    /// Syntax node (red tree with parent pointers)
    syntax: SyntaxNode,
    
    /// Type information cache
    types: Map<NodeId, Type>,
    
    /// Diagnostics cache
    diagnostics: Vec<Diagnostic>,
}

impl IncrementalCompiler {
    fn update(&mut self, change: TextChange) {
        // 1. Apply change to CST (rowan handles incrementality)
        self.cst = self.cst.apply_change(change);
        
        // 2. Re-evaluate changed nodes only
        let changed_nodes = self.find_changed_nodes();
        for node in changed_nodes {
            self.reevaluate_node(node);
        }
        
        // 3. Re-type-check affected expressions
        self.recheck_types();
    }
}
```

### LSP Features

#### 1. Hover (Type Information)

```rust
fn hover(&self, position: Position) -> Option<Hover> {
    let node = self.find_node_at_position(position)?;
    let ty = self.types.get(&node.id())?;
    
    Some(Hover {
        contents: format!("Type: {}", ty),
    })
}
```

#### 2. Go to Definition

```rust
fn definition(&self, position: Position) -> Option<Location> {
    let node = self.find_node_at_position(position)?;
    
    if let Some(ident) = node.as_ident() {
        // Look up in symbol table
        let def = self.find_definition(ident)?;
        return Some(def.location);
    }
    
    None
}
```

#### 3. Completions

```rust
fn completion(&self, position: Position) -> Vec<CompletionItem> {
    let scope = self.scope_at_position(position);
    
    // Collect visible symbols
    scope.visible_symbols()
        .map(|(name, ty)| CompletionItem {
            label: name.to_string(),
            detail: Some(ty.to_string()),
            kind: CompletionItemKind::Variable,
        })
        .collect()
}
```

#### 4. Semantic Tokens

Provide syntax highlighting based on semantic information:

```rust
fn semantic_tokens(&self) -> Vec<SemanticToken> {
    let mut tokens = vec![];
    
    self.walk_ast(|node, ty| {
        let token_type = match node {
            Node::Ident if self.is_function(node) => SemanticTokenType::Function,
            Node::Ident if self.is_type(node) => SemanticTokenType::Type,
            Node::Ident => SemanticTokenType::Variable,
            Node::Literal => SemanticTokenType::Number,
            _ => return,
        };
        
        tokens.push(SemanticToken {
            delta_line: ...,
            delta_start: ...,
            length: ...,
            token_type: token_type as u32,
            token_modifiers: 0,
        });
    });
    
    tokens
}
```

#### 5. Diagnostics (As-You-Type)

```rust
fn publish_diagnostics(&self, uri: Uri) {
    let diagnostics = self.diagnostics.iter()
        .map(|d| lsp_types::Diagnostic {
            range: d.span.to_lsp_range(),
            severity: Some(d.level.to_lsp_severity()),
            message: d.message.clone(),
            source: Some("cadenza".to_string()),
            ..Default::default()
        })
        .collect();
    
    self.client.publish_diagnostics(uri, diagnostics, None);
}
```

### Responsiveness

Key to LSP performance:
1. **Incremental parsing**: Only re-parse changed regions
2. **Lazy type checking**: Only type-check visible code
3. **Background processing**: Type-check in background thread
4. **Cancellation**: Cancel outdated requests

```rust
pub struct LspServer {
    compiler: Arc<Mutex<IncrementalCompiler>>,
    background_thread: JoinHandle<()>,
    cancel_token: CancellationToken,
}

impl LspServer {
    fn on_change(&mut self, change: TextChange) {
        // Cancel any in-progress type checking
        self.cancel_token.cancel();
        
        // Quick parse + evaluate
        self.compiler.lock().update_fast(change);
        
        // Publish immediate diagnostics (parse errors)
        self.publish_diagnostics();
        
        // Start background type checking
        self.spawn_background_typecheck();
    }
}
```

---

## MCP Integration

### Model Context Protocol (MCP)

MCP is a protocol for LLM agents to interact with tools and data sources. Cadenza integrates with MCP in two ways:

#### 1. Compiler as MCP Server (Primary Focus)

The Cadenza compiler itself exposes an MCP server that allows LLMs to inspect and interact with projects:

**Capabilities**:
- Query types for expressions at specific locations
- Search for symbols across the codebase
- Look up documentation for dependencies
- Get type signatures and function definitions
- Inspect the type environment and compiler state

**Goal**: Provide a powerful query engine that gives LLMs deep insight into Cadenza programs, enabling better code understanding and assistance.

```rust
pub struct CadenzaMcpServer {
    compiler: IncrementalCompiler,
}

impl McpTools for CadenzaMcpServer {
    fn query_type(&self, file: String, line: usize, col: usize) -> Result<Type>;
    fn find_definition(&self, symbol: String) -> Result<Location>;
    fn search_symbols(&self, query: String) -> Result<Vec<Symbol>>;
    fn get_docs(&self, symbol: String) -> Result<String>;
}
```

#### 2. Writing MCP Tools in Cadenza (Ecosystem)

While not the primary focus, the language should be expressive enough to enable writing MCP tools:

**Requirements for user-space MCP tools**:
- Introspection capabilities (query types at compile-time)
- User-space macros that can access type information
- Ability to generate JSON schemas from types
- Export functions with metadata

```cadenza
# Example: User writes an MCP tool
fn calculate_area width height =
    width * height

# With sufficient introspection, ecosystem tooling could:
# - Extract type signature: Integer -> Integer -> Integer
# - Generate JSON schema for MCP
# - Package as MCP server
```

This is viewed more as an ecosystem concern, but the language needs to provide the necessary introspection primitives.

---

## Error Handling and Source Tracking

### Principles

1. **Beautiful errors**: Inspired by Rust, Elm, and Roc
2. **Multiple errors**: Don't stop at the first error
3. **Precise locations**: Exact source spans for every diagnostic
4. **Helpful suggestions**: Offer fixes when possible
5. **Full stack traces**: Show the call chain for runtime errors

### Error Representation

Using `miette` for diagnostic rendering:

```rust
use miette::{Diagnostic, SourceSpan};

#[derive(Debug, Diagnostic)]
pub enum DiagnosticKind {
    #[diagnostic(code(E001), severity(Error))]
    #[diagnostic(help("Add a type annotation or provide more context"))]
    TypeMismatch {
        #[label("expected {expected}")]
        expected_span: SourceSpan,
        expected: Type,
        
        #[label("found {actual}")]
        actual_span: SourceSpan,
        actual: Type,
    },
    
    #[diagnostic(code(E002), severity(Error))]
    UndefinedVariable {
        #[label("not found in scope")]
        span: SourceSpan,
        name: InternedString,
        
        #[help]
        suggestion: Option<String>,
    },
    
    #[diagnostic(code(E003), severity(Error))]
    DimensionMismatch {
        #[label("dimension: {left_dim}")]
        left_span: SourceSpan,
        left_dim: Dimension,
        
        #[label("dimension: {right_dim}")]
        right_span: SourceSpan,
        right_dim: Dimension,
        
        operation: String,
    },
}
```

### Example Error Output

```
error[E001]: type mismatch
  ┌─ src/main.cdz:10:5
  │
10│     let x = "hello"
  │             ------- expected Integer
11│     calculate_area x 5
  │                    ^ found String
  │
  = help: Add a type annotation or provide more context
```

### Source Tracking

Every AST node carries source information:

```rust
pub struct Expr {
    kind: ExprKind,
    span: Span,
    file: InternedString,
}

pub struct Span {
    start: u32,
    end: u32,
}
```

The `Span` maps to line/column via a `SourceMap`:

```rust
pub struct SourceMap {
    files: Map<InternedString, SourceFile>,
}

pub struct SourceFile {
    name: InternedString,
    source: String,
    line_starts: Vec<u32>,
}

impl SourceFile {
    fn position(&self, offset: u32) -> (usize, usize) {
        // Handle empty file
        if self.line_starts.is_empty() {
            return (0, 0);
        }
        
        // Handle offset before first line
        if offset < self.line_starts[0] {
            return (0, 0);
        }
        
        // Binary search to find the line containing this offset
        // Returns Ok(idx) if offset equals a line start, Err(idx) for insertion point
        let line = match self.line_starts.binary_search(&offset) {
            Ok(idx) => idx,  // Exact match - offset is at line start
            Err(0) => 0,     // Before first line (handled above, but defensive)
            Err(idx) => idx.saturating_sub(1),  // Offset is between line_starts[idx-1] and line_starts[idx]
        };
        
        // Calculate column from line start
        let line_start = self.line_starts[line];
        let column = (offset - line_start) as usize;
        (line, column)
    }
}
```

### Stack Traces

Runtime errors include full call stacks:

```rust
pub struct StackFrame {
    function: InternedString,
    file: InternedString,
    span: Span,
}

pub struct RuntimeError {
    kind: ErrorKind,
    stack: Vec<StackFrame>,
}
```

Example output:

```
error: division by zero
  ┌─ src/math.cdz:15:12
  │
15│     x / y
  │         ^ division by zero
  │
  = stack trace:
    0: divide (src/math.cdz:15:12)
    1: calculate (src/math.cdz:20:5)
    2: main (src/main.cdz:5:10)
```

### Multi-Error Reporting

The compiler collects all diagnostics:

```rust
pub struct Compiler {
    diagnostics: Vec<Diagnostic>,
    // ...
}

impl Compiler {
    fn record_diagnostic(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }
    
    fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(|d| d.is_error())
    }
    
    fn report_all(&self) {
        for diagnostic in &self.diagnostics {
            eprintln!("{:?}", miette::Report::new(diagnostic.clone()));
        }
    }
}
```

---

## Implementation Roadmap

### Phase 1: Foundation (Current)
- ✅ Lexer and parser
- ✅ Tree-walk evaluator
- ✅ Basic value types
- ✅ Macro expansion
- ✅ Unit system

### Phase 2: Type System
- [ ] Implement HM type inference
- [ ] Add type checker after evaluation
- [ ] Integrate dimensional analysis
- [ ] Handle unevaluated branches
- [ ] Add type annotations (optional)

### Phase 3: Module System
- [ ] Define module structure
- [ ] Implement import/export
- [ ] Cross-module type checking
- [ ] Module registry
- [ ] Dependency resolution

### Phase 4: Traits and Effects
- [ ] Define trait system
- [ ] Implement trait inference
- [ ] Add effect types
- [ ] Effect handlers
- [ ] Constraint solving

### Phase 5: Code Generation
- [ ] Design IR
- [ ] Implement monomorphization
- [ ] IR optimization passes
- [ ] JavaScript backend (for browser)
- [ ] Cranelift backend (for native)

### Phase 6: LSP
- [ ] LSP server skeleton
- [ ] Incremental compilation
- [ ] Hover/go-to-definition
- [ ] Completions
- [ ] Semantic tokens
- [ ] As-you-type diagnostics

### Phase 7: Advanced Features
- [ ] WASM backend
- [ ] LLVM backend (optional)
- [ ] MCP integration
- [ ] Advanced optimizations
- [ ] Debugger support

---

## Open Questions

1. **AST cloning**: Currently `Expr` doesn't implement `Clone`. Do we need to clone for macro expansion, or can we work with references?

2. **Type annotation syntax**: What syntax for optional type annotations?
   ```cadenza
   let x : Integer = 42
   let f : Integer -> Integer = fn x -> x + 1
   ```

3. **Effect syntax**: How do users define and use effects?
   ```cadenza
   @effect
   let Database = effect
       query : String -> Result
   ```

4. **Trait syntax**: How verbose should trait definitions be?

5. **Code size**: With monomorphization, code size can explode. When do we use dynamic dispatch?

6. **FFI**: How do we call into Rust/JS/C from Cadenza?

7. **Package manager**: How do we distribute and install libraries?
   - **Consideration**: Reuse npm for browser compatibility (unpkg, jsDelivr can load dependencies)
   - Cargo has better interface but harder to load in browser
   - Need to support both browser and native use cases

---

## Conclusion

This architecture provides:

✅ **Type safety** through HM inference  
✅ **Correctness** by checking all code paths  
✅ **Performance** via monomorphization and multiple backends  
✅ **Developer experience** with LSP integration  
✅ **Extensibility** through MCP support  
✅ **Beautiful errors** with precise source tracking  

The multi-phase design allows each stage to focus on its concerns while maintaining a clean separation. The evaluator bootstraps the language, the type checker ensures soundness, and code generation produces efficient artifacts.

Next steps: Begin implementing Phase 2 (Type System) starting with basic HM inference.
