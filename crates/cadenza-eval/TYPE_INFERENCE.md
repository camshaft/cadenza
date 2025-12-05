# Type Inference in Cadenza

This document describes the Hindley-Milner type inference system implemented in the `cadenza-eval` crate.

## Overview

The type inference system is based on Algorithm W (Damas-Milner) and provides:

- **Lazy type checking**: Type inference is on-demand, not automatic during evaluation
- **Polymorphism**: Support for parametric polymorphism with quantified types
- **Metaprogramming**: Macros can query expression types for code generation
- **LSP integration**: Designed for responsive IDE features (hover, diagnostics)

## Architecture

### Core Components

#### `InferType` - Types During Inference

```rust
pub enum InferType {
    Concrete(Type),              // Runtime types
    Var(TypeVar),                // Type variables (α, β, γ, ...)
    Fn(Vec<InferType>, Box<InferType>),  // Function types
    Forall(Vec<TypeVar>, Box<InferType>), // Polymorphic types (∀α. α → α)
    // ... other variants
}
```

`InferType` is separate from the runtime `Type` enum to keep inference-specific details (type variables, quantifiers) isolated.

#### `TypeVar` - Type Variables

Type variables represent unknown types during inference. They are placeholders that get unified with concrete types:

```rust
let α = inferencer.fresh_var();  // Creates a new type variable
```

#### `Substitution` - Type Variable Mappings

A substitution is a mapping from type variables to types:

```rust
{ α ↦ Integer, β ↦ String }
```

Substitutions are applied to types to replace variables with their inferred types.

#### `TypeEnv` - Type Environment

The type environment tracks the types of variables in scope:

```rust
let mut env = TypeEnv::new();
env.insert(x_name, InferType::Concrete(Type::Integer));
```

#### `TypeInferencer` - The Inference Engine

The main interface for type inference:

```rust
let mut inferencer = compiler.type_inferencer_mut();
let inferred_type = inferencer.infer_expr(&expr, &env)?;
```

## Algorithm

### 1. Constraint Generation

When inferring the type of an expression, we generate constraints between types:

```cadenza
let f = fn x -> x + 1
```

Generates constraints:
- `typeof(x) = α` (fresh type variable)
- `typeof(+) = (Integer, Integer) -> Integer`
- `α = Integer` (from the constraint that x must be Integer for +)
- `typeof(f) = α -> Integer`

### 2. Unification

Unification finds a substitution that makes two types equal:

```
unify(α, Integer) = { α ↦ Integer }
unify(α -> β, Integer -> String) = { α ↦ Integer, β ↦ String }
```

The unification algorithm includes an **occurs check** to prevent infinite types:

```
unify(α, List[α])  // ERROR: occurs check failed
```

### 3. Generalization

Generalization introduces quantifiers at let-bindings for polymorphism:

```cadenza
let id = fn x -> x  // Type: ∀α. α -> α
id 42               // Instantiates to: Integer -> Integer
id "hello"          // Instantiates to: String -> String
```

Variables that are free in the type but not in the environment are quantified:

```rust
fn generalize(ty: &InferType, env: &TypeEnv) -> InferType {
    let free_in_ty = ty.free_vars();
    let free_in_env = env.free_vars();
    let to_quantify = free_in_ty - free_in_env;
    if to_quantify.is_empty() {
        ty
    } else {
        Forall(to_quantify, ty)
    }
}
```

### 4. Instantiation

Instantiation replaces quantified variables with fresh variables:

```rust
∀α. α -> α  ===instantiate===>  β -> β  (where β is fresh)
```

This allows polymorphic functions to be used with different types.

## Usage

### Basic Type Inference

```rust
use cadenza_eval::{Compiler, typeinfer::TypeEnv};

let mut compiler = Compiler::new();
let mut env = TypeEnv::new();

// Parse an expression
let parsed = parse("42");
let root = parsed.ast();
let expr = &root.items().collect::<Vec<_>>()[0];

// Infer its type
let inferred = compiler.type_inferencer_mut().infer_expr(expr, &env)?;
println!("Type: {}", inferred);  // Type: integer
```

### With Environment

```rust
// Add a variable to the environment
let x: InternedString = "x".into();
env.insert(x, InferType::Concrete(Type::Integer));

// Parse an expression using that variable
let parsed = parse("x + 1");
let expr = &root.items().collect::<Vec<_>>()[0];

// Infer its type
let inferred = compiler.type_inferencer_mut().infer_expr(expr, &env)?;
println!("Type: {}", inferred);  // Type: integer
```

### Polymorphic Functions

```rust
// Create a polymorphic identity function: ∀α. α -> α
let type_var = inferencer.fresh_var();
let id_type = InferType::Forall(
    vec![type_var],
    Box::new(InferType::Fn(
        vec![InferType::Var(type_var)],
        Box::new(InferType::Var(type_var)),
    )),
);

env.insert("id".into(), id_type);

// Use it with different types
let parsed = parse("id 42");
let inferred = compiler.type_inferencer_mut().infer_expr(&expr, &env)?;
// Result: integer (type variable was unified with Integer)

let parsed = parse("id \"hello\"");
let inferred = compiler.type_inferencer_mut().infer_expr(&expr, &env)?;
// Result: string (type variable was unified with String)
```

### In Macros

Macros can access the type inferencer for metaprogramming:

```rust
fn my_macro(args: &[Expr], ctx: &mut EvalContext) -> Result<Value> {
    let inferencer = ctx.compiler.type_inferencer_mut();
    
    // Build a type environment from current scope
    let mut env = TypeEnv::new();
    // ... populate from ctx.env ...
    
    // Infer type of first argument
    let arg_type = inferencer.infer_expr(&args[0], &env)?;
    
    // Generate code based on the type
    match arg_type {
        InferType::Concrete(Type::Integer) => {
            // Generate integer-specific code
        }
        InferType::Concrete(Type::String) => {
            // Generate string-specific code
        }
        _ => {
            // Handle other types
        }
    }
}
```

## Design Rationale

### Why Lazy Type Checking?

Type checking is **not automatic** during evaluation for several reasons:

1. **Performance**: The evaluator can run at full speed without type checking overhead
2. **LSP Responsiveness**: IDE features can be implemented without blocking
3. **Incremental Compilation**: Only changed code needs to be re-type-checked
4. **Cancellation**: Long-running type checks can be cancelled if the user makes changes

### Why Separate InferType from Type?

Runtime `Type` is used for:
- Runtime type checking (e.g., comparing types of values)
- Displaying types to users
- Storing type information in values

`InferType` is used for:
- Type inference with type variables
- Polymorphism with quantified types
- Constraint solving during inference

Keeping them separate:
- Avoids polluting the runtime type system with inference-specific details
- Makes the type inference system easier to understand and maintain
- Allows the runtime type system to evolve independently

### Why Not Type Check Unevaluated Branches Automatically?

The current implementation doesn't automatically track and type-check unevaluated branches (e.g., in conditionals). This is a deliberate choice for Phase 1:

1. **Simplicity**: Easier to implement and understand
2. **Performance**: No overhead for tracking unevaluated code
3. **Flexibility**: Can be added later when needed

Future work will add support for:
- Tracking unevaluated branches during evaluation
- Type-checking them in the background
- Reporting type errors even in unexecuted code

## Future Work

### Type Annotations

Add syntax for optional type annotations:

```cadenza
fn add (x: Integer) (y: Integer) -> Integer =
    x + y
```

Type annotations would:
- Provide better error messages
- Enable earlier error detection
- Document code intent
- Allow partial type inference

### Dimensional Analysis Integration

Integrate type inference with the unit system:

```cadenza
let distance = 100meter
let time = 5second
let velocity = distance / time  // Type: Quantity[Float, meter/second]
```

This requires:
- Extending `InferType` with dimension information
- Adding dimension constraints to unification
- Solving dimension equations alongside type equations

### Unevaluated Branch Tracking

Track and type-check code paths not taken at evaluation time:

```cadenza
let x = if config.debug then
    fn a -> debug_log a  // Not evaluated if debug=false
else
    fn a -> a           // Not evaluated if debug=true
```

Both branches should be type-checked even if only one is executed.

### Background Type Checking

For LSP integration, implement background type checking with:
- Cancellation support (if user makes changes)
- Prioritization (check visible code first)
- Incremental updates (only re-check changed code)
- Caching (avoid redundant type checks)

### Effect System

Extend type inference to track computational effects:

```cadenza
fn read_file (path: String) -> String !IO
```

This requires:
- Extending `InferType` with effect information
- Adding effect constraints to unification
- Inferring effects from operations
