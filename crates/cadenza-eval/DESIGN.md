# Cadenza Evaluator — Initial Design

## Background

Cadenza is a statically typed, unit-aware, rational-arithmetic language for creative coding. The parser produces a lossy AST where all applications are represented uniformly as `Apply` nodes: a receiver (callee) followed by zero or more arguments. Top-level source files are a flat sequence of expressions.

The evaluator must interpret this AST to bootstrap the language itself. Core language constructs (`let`, `fn`, etc.) will be defined in a Cadenza prelude as macros that call back into a compiler API provided by the host.

## Objective

Implement a minimal tree-walk evaluator that can:
- Evaluate a sequence of top-level expressions.
- Distinguish between normal function calls and macro calls.
- Support macro expansion.
- Provide a compiler API that evaluated code can call to register definitions, emit IR, etc.

## Core Idea

The evaluator is the first stage of the compiler. Every top-level expression is evaluated immediately.  
If the expression is a macro call, the macro receives the unevaluated argument subtrees and returns new AST to be spliced in.  
If the expression is a normal function call, arguments are evaluated recursively and the function is applied.

The prelude contains macros that do not produce runtime values — they produce side effects on a shared `Compiler` object exposed to the evaluator. This `Compiler` object is the only mutable state and serves as the explicit API the language uses to build the module.

This design directly follows:

- Common Lisp’s `eval-when (:compile-toplevel)` and macro system
- Racket’s phase separation and `define-for-syntax`
- Elixir’s macros that call `Module.put_attribute`, `Code.quote`, etc.
- Zig’s `comptime` evaluation that can emit declarations

## Design

### 1. Value Representation
```rust
enum Value {
    Rational(rug::Rational),
    Unit { value: rug::Rational, unit: Unit },
    Symbol(InternedId),
    List(Vec<Value>),
    Function { params: Vec<InternedId>, body: AstNode, env: Env },
    Macro { params: Vec<InternedId>, expander: AstNode, env: Env },
    BuiltinFn(fn(&[Value]) -> Result<Value>),
    BuiltinMacro(fn(&[AstNode]) -> Result<AstNode>),
}
```

### 2. Identifier Interning
All identifiers in the AST and environment are interned once during parsing.  
A single global `Interner` (using `fxhash::FxHashMap<String, InternedId>`) maps source strings to a `u32` ID.  
`InternedId` implements `Copy`, `Eq`, `Hash` with zero-cost comparison.  
The interner is the single source of truth for identifier hashing — if the hasher ever needs to change, only this struct is modified.

### 3. Environment
Stack of `FxHashMap<InternedId, Value>`.  
Top frame is mutable; closures capture the environment by reference.

### 4. Compiler API Surface (initial)
```rust
struct Compiler {
    defs: FxHashMap<InternedId, Value>,
    types: FxHashMap<InternedId, Type>,
    // … will grow
}

impl Compiler {
    fn define_var(&mut self, name: InternedId, value: Value);
    fn define_macro(&mut self, name: InternedId, expander: Value);
    // additional methods added as needed
}
```

All internal compiler tables use `FxHashMap` with `InternedId` keys.

### 5. Evaluation Rules (top-level only for now)

For each child expression `expr` in the root `Apply` list:

1. Evaluate:: Evaluate `expr` → `Value`
2. If the result is a macro invocation (detected by looking up the callee name and finding a `Value::Macro`), expand it:
   - Pass the original argument subtrees (unevaluated `AstNode`s) to the macro expander.
   - The expander returns new `AstNode`s.
   - Splice and re-evaluate the resulting nodes.
3. If the result is a normal function call, evaluate arguments recursively, apply the function.
4. Most built-in compiler API functions return `Value::Nil` — their purpose is the side effect on `Compiler`.

### 6. Bootstrapping Flow
1. Create empty `Interner`, `Env`, and `Compiler`.
2. Evaluate the standard prelude (written in Cadenza).
   - Prelude defines `defmacro`, `let`, `def`, `compiler!`, etc.
   - These are macros that call `compiler.define_var`, `compiler.define_macro`, etc.
3. Evaluate the user file in the same environment.

At this point, the `Compiler` struct contains all registered definitions, ready for the next compiler stage (type checking, code generation).

## Implementation Tasks (strictly incremental)

1. Implement `Interner` with `FxHashMap<String, u32>` and `InternedId` wrapper.
2. Implement `Value` enum and basic display.
3. Implement `Env` with scoped `FxHashMap<InternedId, Value>`.
4. Write tree-walk `eval(&AstNode, &mut Env, &mut Compiler) -> Value` that handles:
   - Literals (Rational, Symbol → interned)
   - Lists/vectors
   - Simple applications (lookup callee, eval args, apply)
5. Add special handling for macro expansion:
   - Detect `Value::Macro`
   - Call expander with unevaluated argument nodes
   - Recursively eval expansion result
6. Implement `Compiler` struct with `define_var` and `define_macro`.
7. Write a minimal prelude that defines:
   - `defmacro`
   - `let` (as macro calling `compiler.define_var`)
   - `def` (as macro calling `compiler.define_var`)
   - `compiler!` syntax macro that expands to built-in calls
8. Test end-to-end: a file containing `(def x 42) (def y (+ x 8))` results in `Compiler.defs` containing both bindings.

This is the complete minimal evaluator. All subsequent features (tasks, `@design`, etc.) will be built on top of this foundation using the same mechanisms.
