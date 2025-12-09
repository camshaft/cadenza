# Traits and Effects System Design

This document details the implementation of the trait and effect system for Cadenza, based on the high-level design in `/docs/COMPILER_ARCHITECTURE.md`.

## Table of Contents

1. [Overview](#overview)
2. [Trait System](#trait-system)
3. [Effect System](#effect-system)
4. [Implementation Phases](#implementation-phases)
5. [Open Questions](#open-questions)

## Overview

### Design Principles

1. **Implicit trait bounds**: Users don't write trait bounds; they're inferred from usage
2. **Implicit effect propagation**: Effects propagate to callers automatically
3. **Duck typing for records**: Functions accept any record with required fields
4. **Extensibility**: Field access and operators can be trait-based
5. **Zero-cost abstractions**: Monomorphization eliminates runtime overhead

### Relationship to Existing Features

- **Type System**: Traits and effects extend the existing Hindley-Milner type inference
- **Records**: Duck typing builds on structural record types
- **Operators**: Operators will become traits (Add, Sub, Mul, etc.)
- **Field Access**: Can optionally be implemented as a trait

## Trait System

### What are Traits?

Traits are similar to type classes in Haskell or traits in Rust. They define a set of methods that types can implement. The key innovation in Cadenza is **implicit trait inference** - you don't need to write trait bounds; the compiler infers them from usage.

### Trait Definition Syntax

**Option 1: Simple trait with methods**
```cadenza
trait Numeric =
  fn add Self Self = Self
  fn mul Self Self = Self
  fn zero = Self
```

**Option 2: Trait with type parameters**
```cadenza
trait Numeric Rhs Output =
  fn add Self Rhs = Output
  fn mul Self Rhs = Output
  fn zero = Self
```

**Decision**: Start with Option 1 (simpler), add generics later if needed.

### Trait Implementation Syntax

```cadenza
impl Integer = Numeric
  fn add a b = a + b
  fn mul a b = a * b
  fn zero = 0

impl Float = Numeric
  fn add a b = a + b
  fn mul a b = a * b
  fn zero = 0.0
```

### Implicit Trait Inference

**User writes:**
```cadenza
fn sum list =
    fold list 0 (fn acc x -> acc + x)
```

**Compiler infers:**
```cadenza
fn sum : forall a. [Numeric a] => List a -> a
```

**How it works:**
1. Type inference encounters `acc + x`
2. Looks up `+` operator → requires `Numeric` trait
3. Generates constraint: `typeof(acc) : Numeric`
4. Unifies with `typeof(x)` (both must be same type and support `+`)
5. Adds trait constraint to function signature

### Trait Constraint Representation

**In Type System:**
```rust
pub enum Type {
    // ... existing variants ...
    
    /// Type with trait constraints
    /// Example: `[Numeric a] => a` is Constrained { ty: Var(a), traits: [Numeric] }
    Constrained {
        ty: Box<Type>,
        traits: Vec<TraitRef>,
    },
}

pub struct TraitRef {
    /// Name of the trait
    name: InternedString,
    /// Type parameters to the trait (if any)
    params: Vec<Type>,
}
```

**In Type Inference:**
```rust
pub enum InferType {
    // ... existing variants ...
    
    /// Type variable with trait constraints
    /// Example: `α where α : Numeric`
    ConstrainedVar {
        var: TypeVar,
        traits: Vec<TraitRef>,
    },
}
```

### Trait Registry

```rust
pub struct TraitRegistry {
    /// All defined traits
    traits: Map<InternedString, TraitDef>,
    
    /// Implementations: (Type, Trait) -> Implementation
    implementations: Map<(Type, InternedString), TraitImpl>,
}

pub struct TraitDef {
    name: InternedString,
    methods: Vec<MethodSignature>,
    type_params: Vec<InternedString>,
}

pub struct MethodSignature {
    name: InternedString,
    params: Vec<Type>,
    return_ty: Type,
}

pub struct TraitImpl {
    trait_name: InternedString,
    for_type: Type,
    methods: Map<InternedString, Value>,  // Method implementations
}
```

### Built-in Traits

Define standard traits for operators:

```cadenza
# Numeric operations
trait Add =
  fn add Self Self = Self

trait Sub =
  fn sub Self Self = Self

trait Mul =
  fn mul Self Self = Self

trait Div =
  fn div Self Self = Self

# Comparisons
trait Eq =
  fn eq Self Self = Bool

trait Ord =
  fn lt Self Self = Bool
  fn le Self Self = Bool
  fn gt Self Self = Bool
  fn ge Self Self = Bool

# Conversions
trait ToString =
  fn to_string Self = String

# Field access (optional)
trait FieldAccess Field =
  fn get_field Self Field = Result
```

### Trait Constraint Inference Algorithm

```rust
impl TypeInferencer {
    fn infer_with_traits(&mut self, expr: &Expr, env: &TypeEnv) -> Result<InferType> {
        match expr {
            Expr::Apply(op, args) => {
                // Infer types of operator and arguments
                let op_type = self.infer_expr(op, env)?;
                let arg_types: Vec<_> = args.iter()
                    .map(|arg| self.infer_expr(arg, env))
                    .collect::<Result<_>>()?;
                
                // Check if operator requires traits
                if let Some(trait_name) = self.get_operator_trait(op) {
                    // Generate trait constraint
                    let constraint = TraitConstraint {
                        ty: arg_types[0].clone(),
                        trait_ref: TraitRef {
                            name: trait_name,
                            params: vec![],
                        },
                    };
                    self.add_trait_constraint(constraint);
                }
                
                // Continue with normal type inference
                self.infer_apply(op_type, arg_types)
            }
            // ... other cases
        }
    }
}
```

### Monomorphization with Traits

After type inference, we have:
```
fn sum : forall a. [Numeric a] => List a -> a
```

At call sites:
```cadenza
sum [1, 2, 3]        # a = Integer, Numeric Integer
sum [1.0, 2.0, 3.0]  # a = Float, Numeric Float
```

Monomorphization:
1. Collect all instantiations with their trait constraints
2. For each instantiation, lookup trait implementation
3. Generate specialized function with direct method calls
4. Replace generic calls with specialized calls

## Effect System

### What are Effects?

Effects are similar to traits but represent computational context rather than type capabilities. They're like the Reader monad in Haskell - implicit parameters that propagate through the call stack.

### Key Differences from Traits

| Aspect | Traits | Effects |
|--------|--------|---------|
| Receiver | Explicit (`self.method()`) | Implicit (from scope) |
| Purpose | Type capabilities | Computational context |
| Example | `Numeric.add(a, b)` | `Logger.log("msg")` |
| Propagation | Via type parameters | Via effect context |

### Effect Definition Syntax

```cadenza
effect Logger =
  fn log String = ()

effect Database =
  fn query String = Result

effect Config =
  fn get String = String
```

### Effect Usage

**Without explicit context:**
```cadenza
fn process data =
    Logger.log "Processing data"
    let result = Database.query "SELECT * FROM items"
    result
```

**Compiler infers:**
```cadenza
fn process : [Logger, Database] => Data -> Result
```

### Effect Handlers

Provide implementations at call sites:

```cadenza
# Define a handler
let console_logger = handler Logger
    log = fn msg -> print "LOG: ${msg}"

let file_logger = handler Logger
    log = fn msg -> write_file "log.txt" msg

# Use a handler
with console_logger
    process my_data

# Chain handlers
with console_logger, my_database
    process my_data
```

### Effect State Management

**Question**: How do effects maintain state?

**Option 1: Pure functions with explicit state**
```cadenza
# Effect handler returns next state
let stateful_logger = handler Logger state
    log = fn msg state -> 
        let new_state = { ...state, count = state.count + 1 }
        ((), new_state)  # Return unit value and new state
```

**Option 2: Mutable cells/boxes**
```cadenza
# Effect handler has mutable state
let stateful_logger = handler Logger
    let count = cell 0
    log = fn msg ->
        count := count.get + 1
        print "LOG ${count.get}: ${msg}"
```

**Option 3: Coroutine-style resumption**
```cadenza
# Effect can pause function and resume with a value
let interactive_logger = handler Logger
    log = fn msg ->
        # Pause and ask user for confirmation
        let should_log = prompt "Log '${msg}'? (y/n)"
        if should_log == "y"
            print msg
```

**Decision**: Start with Option 1 (pure state passing), add Option 3 (coroutines) later as it's more powerful.

### Effect Representation

**In Type System:**
```rust
pub enum Type {
    // ... existing variants ...
    
    /// Function type with effects
    /// Example: `[Logger] => Integer -> Integer`
    EffectFn {
        params: Vec<Type>,
        return_ty: Box<Type>,
        effects: Vec<EffectRef>,
    },
}

pub struct EffectRef {
    name: InternedString,
}
```

**In Value System:**
```rust
pub enum Value {
    // ... existing variants ...
    
    /// Effect handler
    EffectHandler {
        effect: InternedString,
        methods: Map<InternedString, Value>,
        state: Option<Box<Value>>,
    },
}
```

### Effect Registry

```rust
pub struct EffectRegistry {
    /// All defined effects
    effects: Map<InternedString, EffectDef>,
    
    /// Active handlers in current scope
    active_handlers: Vec<(InternedString, Value)>,
}

pub struct EffectDef {
    name: InternedString,
    methods: Vec<MethodSignature>,
}
```

### Effect Propagation Algorithm

```rust
impl TypeInferencer {
    fn infer_with_effects(&mut self, expr: &Expr, env: &TypeEnv) -> Result<InferType> {
        match expr {
            Expr::Apply(receiver, method, args) if is_effect_call(receiver) => {
                // Detect effect call: EffectName.method(...)
                let effect_name = extract_effect_name(receiver)?;
                
                // Add effect to current context
                self.add_effect_requirement(effect_name);
                
                // Infer method return type
                let method_type = self.lookup_effect_method(effect_name, method)?;
                Ok(method_type)
            }
            Expr::Function(params, body) => {
                // Infer function body with fresh effect context
                let body_type = self.infer_expr(body, env)?;
                let effects = self.take_effect_requirements();
                
                // Function type includes effect requirements
                Ok(InferType::EffectFn {
                    params: params.iter().map(|_| self.fresh_var()).collect(),
                    return_ty: Box::new(body_type),
                    effects,
                })
            }
            // ... other cases
        }
    }
}
```

### Effect Handler Evaluation

```rust
impl EvalContext {
    pub fn with_handler(&mut self, handler: Value, body: impl FnOnce(&mut Self) -> Result<Value>) -> Result<Value> {
        // Push handler onto effect stack
        self.push_effect_handler(handler)?;
        
        // Evaluate body with handler active
        let result = body(self);
        
        // Pop handler
        self.pop_effect_handler();
        
        result
    }
    
    fn call_effect_method(&mut self, effect: InternedString, method: InternedString, args: Vec<Value>) -> Result<Value> {
        // Look up active handler for this effect
        let handler = self.find_effect_handler(effect)?;
        
        // Extract method implementation from handler
        let method_impl = handler.get_method(method)?;
        
        // Call method
        self.apply_function(method_impl, args)
    }
}
```

## Duck Typing for Fields

### Current State

Records already have structural typing:
```cadenza
let point = { x = 1, y = 2 }
let point3d = { x = 1, y = 2, z = 3 }
```

### Goal

Functions should accept any record with required fields:

```cadenza
fn get_x record = record.x

# Works with any record that has an 'x' field
get_x { x = 1, y = 2 }       # OK
get_x { x = 5, z = 10 }      # OK
get_x { y = 2 }              # Error: missing field 'x'
```

### Implementation via Row Polymorphism

```rust
pub enum InferType {
    // ... existing variants ...
    
    /// Record type with row polymorphism
    /// { x: Integer | r } represents a record with at least field 'x: Integer'
    /// and possibly other fields captured by row variable 'r'
    RecordRow {
        fields: Vec<(InternedString, InferType)>,
        rest: Option<RowVar>,  // Row variable for additional fields
    },
}

pub struct RowVar(u32);
```

**Example:**
```cadenza
fn get_x record = record.x
```

Inferred type:
```
get_x : forall r. { x: a | r } -> a
```

Meaning: accepts any record with field `x` of type `a`, plus any other fields.

### Type Inference with Rows

```rust
impl TypeInferencer {
    fn infer_field_access(&mut self, record: &Expr, field: InternedString, env: &TypeEnv) -> Result<InferType> {
        let record_type = self.infer_expr(record, env)?;
        
        match record_type {
            InferType::Record(fields) => {
                // Known record type - lookup field
                fields.iter()
                    .find(|(name, _)| *name == field)
                    .map(|(_, ty)| ty.clone())
                    .ok_or_else(|| Error::FieldNotFound(field))
            }
            InferType::Var(v) => {
                // Unknown type - constrain it to have this field
                let field_type = self.fresh_var();
                let rest = self.fresh_row_var();
                
                // Unify: v = { field: field_type | rest }
                self.unify(
                    InferType::Var(v),
                    InferType::RecordRow {
                        fields: vec![(field, InferType::Var(field_type))],
                        rest: Some(rest),
                    }
                )?;
                
                Ok(InferType::Var(field_type))
            }
            _ => Err(Error::NotARecord(record_type))
        }
    }
}
```

## Field Access as a Trait (Optional)

### Motivation

Making field access a trait allows:
1. Custom types to implement field access
2. Computed properties
3. Virtual fields
4. Lazy evaluation of fields

### Trait Definition

```cadenza
trait FieldAccess Field Result =
  fn get_field Self Field = Result
  fn set_field Self Field Result = Self
```

### Implementation for Records

```cadenza
# Automatically implemented for all records
impl { ... } = FieldAccess String a
  fn get_field self field =
    # Builtin field lookup
    __builtin_field_get self field
  
  fn set_field self field value =
    # Builtin field update
    __builtin_field_set self field value
```

### Custom Implementations

```cadenza
struct Point { x = Float, y = Float }

# Allow accessing 'r' and 'theta' as virtual polar coordinates
impl Point = FieldAccess "r" Float
  fn get_field self field =
    sqrt (self.x * self.x + self.y * self.y)

impl Point = FieldAccess "theta" Float
  fn get_field self field =
    atan2 self.y self.x
```

**Decision**: Defer this to Phase 7. It's cool but not essential for MVP.

## Implementation Phases

### Phase 1: Trait System Basics (Current Priority)

**Goal**: Basic trait definitions and implementations without inference.

1. Add `trait` and `impl` as special forms
2. Extend `Type` with `Trait` and `Constrained` variants
3. Implement `TraitRegistry` in `Compiler`
4. Parse trait definitions and store in registry
5. Parse trait implementations and store in registry
6. Add tests with explicit trait bounds (inference comes later)

**Deliverables**:
- `trait_form.rs` - Special form for defining traits
- `impl_form.rs` - Special form for implementing traits
- Updated `Type` enum with trait support
- `TraitRegistry` in `Compiler`
- Test files demonstrating trait definition and implementation

### Phase 2: Implicit Trait Inference

**Goal**: Automatically infer trait bounds from usage.

1. Extend type inferencer to track trait constraints
2. Add trait constraint generation for operators
3. Implement trait constraint solving
4. Add trait bounds to inferred function types
5. Validate trait implementations match signatures

**Deliverables**:
- Updated `TypeInferencer` with trait constraint tracking
- Trait constraint generation in `infer_expr`
- Tests showing automatic trait bound inference

### Phase 3: Effect System Basics

**Goal**: Basic effect definitions and handlers without propagation.

1. Add `effect` and `handler` as special forms
2. Extend `Type` with `EffectFn` variant
3. Implement `EffectRegistry` in `Compiler`
4. Parse effect definitions and store in registry
5. Parse handlers and implement effect method dispatch
6. Add tests with explicit effect contexts

**Deliverables**:
- `effect_form.rs` - Special form for defining effects
- `handler_form.rs` - Special form for creating handlers
- Updated `Type` enum with effect support
- `EffectRegistry` in `Compiler`
- Test files demonstrating effects and handlers

### Phase 4: Implicit Effect Propagation

**Goal**: Automatically propagate effect requirements to callers.

1. Extend type inferencer to track effect requirements
2. Add effect requirement generation for effect calls
3. Propagate effects through function calls
4. Add effect requirements to inferred function types
5. Validate handlers match effect signatures

**Deliverables**:
- Updated `TypeInferencer` with effect tracking
- Effect propagation in `infer_expr`
- Tests showing automatic effect propagation

### Phase 5: Duck Typing for Records

**Goal**: Support structural typing with row polymorphism.

1. Add row variables to type system
2. Implement row polymorphism in type inference
3. Update field access to generate row constraints
4. Implement row unification
5. Add tests for duck-typed record functions

**Deliverables**:
- Row polymorphism support in type system
- Updated field access inference
- Tests for duck-typed functions

### Phase 6: Monomorphization

**Goal**: Generate specialized code for each trait/type combination.

1. Implement monomorphization pass after type inference
2. Collect trait instantiations at call sites
3. Generate specialized functions with direct calls
4. Replace generic calls with specialized calls
5. Add dead code elimination for unused specializations

**Deliverables**:
- Monomorphization pass in IR generation
- Specialized function generation
- Tests showing code specialization

### Phase 7: Operators as Traits

**Goal**: Make all operators use trait dispatch.

1. Define standard operator traits (Add, Sub, Mul, etc.)
2. Implement operator traits for built-in types
3. Update operator special forms to use trait dispatch
4. Allow user-defined operator implementations
5. Update IR generation for trait-based operators

**Deliverables**:
- Operator trait definitions
- Trait-based operator dispatch
- Tests for user-defined operators

## Open Questions

### 1. Effect State Management

**Question**: How should effects maintain state across calls?

**Options**:
- A) Pure state passing (handler returns next state)
- B) Mutable cells/boxes
- C) Coroutine-style with pause/resume

**Recommendation**: Start with A, implement C later (most powerful).

### 2. Trait Syntax Complexity

**Question**: Do we support generic traits with type parameters?

**Options**:
- A) Simple traits only: `trait Numeric = fn add Self Self = Self`
- B) Generic traits: `trait Numeric Rhs Output = fn add Self Rhs = Output`

**Recommendation**: Start with A, add B if needed (avoid complexity snowball).

### 3. Field Access Trait

**Question**: Should field access use trait dispatch?

**Options**:
- A) Keep field access as built-in operation
- B) Make field access a trait (allows virtual fields, computed properties)

**Recommendation**: Start with A, B is optional future enhancement.

### 4. Effect Handler Scoping

**Question**: How do nested handlers interact?

**Example**:
```cadenza
with logger1
    with logger2
        Logger.log "test"  # Which handler is used?
```

**Options**:
- A) Inner handler shadows outer (default)
- B) Error on conflicting handlers
- C) Compose handlers (both run)

**Recommendation**: Start with A (simplest), consider C later.

### 5. Trait Coherence

**Question**: Can the same trait be implemented multiple times for a type?

**Options**:
- A) No - single implementation per (type, trait) pair (coherence)
- B) Yes - allow multiple implementations

**Recommendation**: A (coherence) - prevents ambiguity and ensures predictable behavior.

### 6. Cross-Module Traits

**Question**: Can traits be implemented for types from other modules?

**Options**:
- A) Yes - orphan instances allowed
- B) No - must own either trait or type

**Recommendation**: Start with B (prevents conflicts), relax later if needed.

### 7. Effect Composition

**Question**: Can multiple effects be combined/composed?

**Options**:
- A) Independent - each effect handled separately
- B) Composed - effects can interact/depend on each other

**Recommendation**: Start with A, B is advanced feature for later.

### 8. Monomorphization Code Size

**Question**: How do we prevent code explosion from monomorphization?

**Options**:
- A) Aggressive specialization (fast code, large binary)
- B) Dynamic dispatch for some cases (smaller binary, slower)
- C) Smart heuristics (specialize hot paths only)

**Recommendation**: Start with A, add B/C based on profiling.

## References

- `/docs/COMPILER_ARCHITECTURE.md` - High-level architecture
- Haskell type classes - https://www.haskell.org/tutorial/classes.html
- Rust traits - https://doc.rust-lang.org/book/ch10-02-traits.html
- Algebraic effects - https://www.eff-lang.org/handlers-tutorial.pdf
- Row polymorphism - https://en.wikipedia.org/wiki/Row_polymorphism

## Next Steps

1. Review this design document with stakeholders
2. Make decisions on open questions
3. Begin Phase 1 implementation (trait system basics)
4. Create test files for trait definitions
5. Implement trait special forms
6. Add trait support to type system
