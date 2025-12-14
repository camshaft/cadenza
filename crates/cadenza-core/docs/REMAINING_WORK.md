# cadenza-core: Remaining Work

This document tracks the remaining implementation work for the cadenza-core HIR-based compiler infrastructure.

## Completed ✅

### Foundation
- ✅ Salsa database infrastructure (`db` module)
  - CadenzaDb trait
  - CadenzaDbImpl implementation
  - SourceFile input type
  - ParsedFile tracked struct
  - parse_file tracked function
  - Diagnostic accumulator with severity levels
  - All tests passing (10 tests)

### HIR Definition
- ✅ HIR expression types (`hir` module)
  - ExprKind enum with all common expression types
  - Span tracking on every expression
  - HirId system for node referencing
  - Literal types (Integer, Float, String, Bool, Nil)
  - Binary and unary operators
  - Pattern types for match expressions
  - Module structure
  - All tests passing

### Documentation
- ✅ ARCHITECTURE.md - Complete architecture overview
- ✅ README.md - Usage and comparison with cadenza-eval
- ✅ REMAINING_WORK.md - This document
- ✅ Updated root docs/SALSA_MIGRATION.md
- ✅ Updated root docs/COMPILER_ARCHITECTURE.md

## TODO: Critical Path

### 1. Lower Module (AST → HIR)

**Priority: HIGH** - Required before any other work can proceed.

Create `src/lower.rs` module that transforms the CST/AST to HIR:

```rust
#[salsa::tracked]
pub fn lower_file(db: &dyn CadenzaDb, parsed: ParsedFile) -> HirModule {
    // Walk CST, produce HIR with preserved spans
}
```

**Tasks:**
- [ ] Create `lower.rs` module
- [ ] Implement `lower_file` tracked function
- [ ] Walk CST nodes and create HIR expressions
- [ ] Preserve source spans through lowering
- [ ] Handle all expression types:
  - [ ] Literals (integer, float, string, bool)
  - [ ] Identifiers
  - [ ] Let bindings
  - [ ] Function definitions
  - [ ] Function calls (Apply nodes)
  - [ ] Binary operations
  - [ ] Unary operations
  - [ ] Blocks (synthetic __block__ nodes)
  - [ ] Lists (synthetic __list__ nodes)
  - [ ] Records (synthetic __record__ nodes)
  - [ ] Field access (Attr nodes)
  - [ ] If expressions
  - [ ] Match expressions
- [ ] Desugar complex syntax to simple HIR forms
- [ ] Emit diagnostics for malformed syntax
- [ ] Write comprehensive tests
- [ ] Update ARCHITECTURE.md with implementation details

**Key Design Questions:**
- How to handle synthetic nodes (__block__, __list__, __record__)?
- How to desugar Apply nodes into proper function calls?
- How to handle macro invocations during lowering?

### 2. Eval Module (HIR Evaluation & Expansion)

**Priority: HIGH** - Core functionality, required for LSP to be useful.

Create `src/eval.rs` module for HIR evaluation and macro expansion:

```rust
#[salsa::tracked]
pub fn evaluate_module(db: &dyn CadenzaDb, hir: HirModule) -> ExpandedHirModule {
    // Evaluate compile-time expressions
    // Expand macros: HIR → HIR
    // Track unevaluated branches
}
```

**Tasks:**
- [ ] Create `eval.rs` module
- [ ] Define ExpandedHirModule tracked struct
- [ ] Implement `evaluate_module` tracked function
- [ ] Implement HIR evaluator (tree-walk interpreter on HIR)
- [ ] Implement macro expansion:
  - [ ] Recognize macro invocations in HIR
  - [ ] Expand macros to generate new HIR nodes
  - [ ] Preserve/synthesize spans for generated code
- [ ] Implement compile-time evaluation:
  - [ ] Loop unrolling for code generation
  - [ ] Constant folding
  - [ ] List comprehensions
- [ ] Track unevaluated branches (if/else not taken, match arms)
- [ ] Accumulate evaluation diagnostics
- [ ] Maintain module scope (definitions, exports)
- [ ] Write comprehensive tests
- [ ] Update ARCHITECTURE.md

**Key Design Questions:**
- How to represent unevaluated branches for type checking?
- How to synthesize spans for macro-generated code?
- Should evaluation be pure or track side effects?
- How to handle infinite loops in compile-time evaluation?

### 3. Type Inference Module

**Priority: MEDIUM** - Required for type checking and advanced LSP features.

Create `src/typeinfer.rs` module for Hindley-Milner type inference on expanded HIR:

```rust
#[salsa::tracked]
pub fn infer_types(db: &dyn CadenzaDb, expanded: ExpandedHirModule) -> TypedHirModule {
    // Run HM type inference on expanded HIR
    // Check dimensional analysis
    // Validate trait constraints
}
```

**Tasks:**
- [ ] Create `typeinfer.rs` module
- [ ] Define TypedHirModule tracked struct
- [ ] Implement `infer_types` tracked function
- [ ] Implement Hindley-Milner algorithm:
  - [ ] Constraint generation
  - [ ] Unification
  - [ ] Generalization
  - [ ] Instantiation
- [ ] Type unevaluated branches
- [ ] Integrate dimensional analysis
- [ ] Check trait constraints
- [ ] Accumulate type errors as diagnostics
- [ ] Write comprehensive tests
- [ ] Update ARCHITECTURE.md

**Dependencies:**
- Requires eval module to be complete
- May reuse type inference code from cadenza-eval

### 4. LSP Queries Module

**Priority: MEDIUM** - Enables IDE features.

Create `src/queries.rs` module for LSP queries on expanded, typed HIR:

```rust
#[salsa::tracked]
pub fn hover_info(db: &dyn CadenzaDb, source: SourceFile, pos: Position) -> Option<HoverInfo> {
    // Find HIR node at position
    // Return type and documentation
}

#[salsa::tracked]
pub fn completions(db: &dyn CadenzaDb, source: SourceFile, pos: Position) -> Vec<Completion> {
    // Find context at position
    // Return available completions from scope
}

// etc.
```

**Tasks:**
- [ ] Create `queries.rs` module
- [ ] Implement hover information query
- [ ] Implement completion query
- [ ] Implement go-to-definition query
- [ ] Implement find-references query
- [ ] Implement semantic tokens query
- [ ] Implement document symbols query
- [ ] Write comprehensive tests
- [ ] Update ARCHITECTURE.md

**Dependencies:**
- Requires lower, eval, and typeinfer modules
- Operates on expanded, typed HIR

## TODO: Future Work

### 5. Code Generation

Create backend for generating executable code:

- [ ] Define code generation IR (or use HIR directly)
- [ ] Implement monomorphization
- [ ] Add optimization passes
- [ ] Create JavaScript backend
- [ ] Create WASM backend (optional)
- [ ] Create Cranelift backend (optional)

### 6. Module System

Implement cross-file compilation:

- [ ] Define module resolution strategy
- [ ] Implement import/export tracking
- [ ] Add cross-module type checking
- [ ] Create module registry

### 7. Effects & Traits

Extend type system:

- [ ] Define effect types in HIR
- [ ] Implement effect inference
- [ ] Define trait system
- [ ] Implement trait inference

### 8. Optimizations

- [ ] Dead code elimination
- [ ] Constant propagation
- [ ] Inline expansion
- [ ] Tail call optimization

## Success Criteria

For each module, success means:

1. **Tests Pass**: Comprehensive unit and integration tests
2. **Documentation**: Updated ARCHITECTURE.md with implementation details
3. **Error Handling**: Diagnostics for all error cases
4. **Span Tracking**: Source spans preserved through all transformations
5. **Incrementality**: Salsa properly caches and invalidates

## Getting Started

**Next immediate step**: Implement the Lower module (AST → HIR transformation).

See `ARCHITECTURE.md` for design guidance and the overall architecture vision.
