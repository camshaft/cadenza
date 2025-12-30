# cadenza-compiler - Status

This crate is the canonical definition of the Cadenza programming language semantics, defined using the cadenza-meta framework.

## Purpose

Instead of hand-writing an interpreter/compiler, we define the language semantics declaratively using cadenza-meta, which generates efficient query-based implementations. This approach:

- Provides a single source of truth for language semantics
- Enables multiple compilation targets from one definition
- Makes semantics explicit and verifiable
- Allows for formal reasoning about the language

## Current State

### Completed ✅

- Created crate structure
- Set up build script using cadenza-meta
- Defined initial semantic queries:
  - `eval`: Evaluate expression to value (3 rules: integer, symbol, apply)
  - `type_of`: Infer expression type (external)
  - `lookup_var`: Variable lookup (external)
  - `eval_apply`: Function application (external)
- Code generation working (generates Rust code to src/generated/semantics.rs)

### Known Issues

**Generated Code Compilation Errors:**

1. `Type::String` not implemented in codegen → generates `todo!("other types")`
2. Some control flow issues in generated code
3. `capture_all` pattern support incomplete
4. External query signatures need refinement

**Root Causes:**

- cadenza-meta doesn't yet support all type variants
- Some pattern types (CaptureAll) need more work
- Generated code structure needs refinement

## Next Steps

### Immediate (Fix Generated Code)

1. Add `Type::String` and other basic types to cadenza-meta codegen
2. Implement `capture_all` pattern properly
3. Fix control flow generation for multiple rules
4. Ensure external query signatures match expectations

### Short Term (Expand Language Coverage)

1. Add more expression types to eval (let, if, operators)
2. Define operators as semantic rules
3. Add literal types (float, string, bool)
4. Implement basic type inference rules

### Medium Term (Complete Core Language)

1. Function definitions and closures
2. Let bindings and scoping
3. Pattern matching
4. Records and tuples
5. Lists and arrays

### Long Term (Advanced Features)

1. Type inference with HM algorithm
2. Module system
3. Traits and effects
4. Macro expansion

## Design Decisions

**External vs Generated Queries:**

- Start with external queries for complex operations (variable lookup, function application)
- Gradually move logic into semantic rules as patterns become clear
- External queries serve as extension points for hand-optimized code

**Incremental Approach:**

- Begin with minimal working semantics (just integers)
- Add rules incrementally, keeping build passing
- Each addition should be testable

**Integration Strategy:**

- Generated code provides query interface
- Hand-written code implements Database trait
- External queries bridge to existing infrastructure (Env, Compiler, etc.)

## Architecture

```
build/main.rs
  └─> Defines semantics using cadenza-meta builders
  └─> Generates src/generated/semantics.rs at build time

src/lib.rs
  └─> Re-exports generated queries
  └─> Defines Database trait
  └─> Implements external queries

Integration with evaluator:
  └─> Evaluator implements Database trait
  └─> Calls generated query functions
  └─> Provides external query implementations
```

## Success Criteria

The cadenza-compiler crate is successful when:

1. All Cadenza language constructs have semantic definitions
2. Generated code compiles without errors
3. Query implementations pass comprehensive tests
4. Performance is acceptable for real-world code
5. Semantics are clear enough for formal verification
