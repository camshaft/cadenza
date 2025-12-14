# Cadenza Core Architecture

This document describes the architecture of `cadenza-core`, the HIR-based incremental compilation system for Cadenza using Salsa.

## Overview

Cadenza Core implements a compilation pipeline that transforms source code into an evaluated, type-checked representation suitable for code generation and LSP queries.

```text
Source Code
    ↓
  Parse (CST)
    ↓
  Lower (HIR with spans)
    ↓
  Evaluate/Expand (Expanded HIR with spans)
    ↓
  Type Inference (Typed HIR)
    ↓
  LSP Queries / Code Generation
```

## Key Design Decisions

### 1. HIR-First Approach

Unlike the original POC in `cadenza-eval` which worked directly on the AST, `cadenza-core` uses a High-level Intermediate Representation (HIR):

**Why HIR?**
- **Simplified Structure**: Complex syntax is desugared during lowering
- **Span Preservation**: Every HIR node tracks its source location
- **Evaluation-Ready**: HIR can be directly evaluated/expanded
- **Analysis-Friendly**: Easier to analyze than raw AST

**HIR vs AST:**
- AST: Direct representation of source syntax (from parser)
- HIR: Simplified, desugared representation with spans

### 2. Evaluation on HIR

Evaluation happens on the HIR, not the AST:

```rust
// HIR evaluation produces expanded HIR
fn evaluate_hir(db: &dyn CadenzaDb, hir: HirModule) -> ExpandedHirModule {
    // Macro expansion: HIR → HIR
    // Compile-time evaluation: HIR → HIR
    // Still preserves spans for error reporting
}
```

**Benefits:**
- Macro expansion can generate new HIR nodes with proper spans
- Compile-time evaluation (e.g., loop unrolling for code generation) works on HIR
- Type inference operates on expanded HIR, sees all generated code
- LSP operates on expanded HIR, aware of macro-generated code

### 3. Span Tracking Throughout

Every transformation preserves source spans:

```rust
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,  // Always present!
    pub id: Option<HirId>,
}
```

This enables:
- Accurate error messages pointing to source
- IDE features (hover, go-to-definition) even on generated code
- Debugging information in compiled output

### 4. Post-Expansion LSP

The LSP operates on **expanded** HIR, not pre-expansion:

**Why?**
- Type inference needs to see macro-generated code
- LSP features (completion, hover) need to know about generated symbols
- Module scope includes compile-time generated definitions

**Example:**
```cadenza
// User writes:
for name in ["foo", "bar", "baz"]
    fn ${name} x = x + 1

// LSP sees (after expansion):
fn foo x = x + 1
fn bar x = x + 1
fn baz x = x + 1
```

Without expansion, LSP wouldn't know about `foo`, `bar`, `baz`.

## Architecture Components

### Database (`db` module)

Salsa database infrastructure:

```rust
#[salsa::db]
pub trait CadenzaDb: salsa::Database {}

#[salsa::input]
pub struct SourceFile { ... }

#[salsa::tracked]
pub struct ParsedFile<'db> { ... }

#[salsa::accumulator]
pub struct Diagnostic { ... }
```

**Implemented:**
- ✅ Source file tracking
- ✅ Parse file tracking
- ✅ Diagnostic accumulation

**TODO:**
- [ ] HIR lowering query
- [ ] HIR evaluation/expansion query
- [ ] Type inference query
- [ ] LSP queries (hover, completion, etc.)

### HIR (`hir` module)

High-level Intermediate Representation definitions:

```rust
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
    pub id: Option<HirId>,
}

pub enum ExprKind {
    Literal(Literal),
    Ident(String),
    Let { name, value },
    Fn { name, params, body },
    Call { func, args },
    // ... more variants
}
```

**Implemented:**
- ✅ HIR expression types
- ✅ Span tracking
- ✅ HirId for node referencing

**TODO:**
- [ ] HIR for types
- [ ] HIR for patterns (partially done)
- [ ] HIR for modules/imports

### Lower (TODO - `lower` module)

AST → HIR lowering with span preservation:

```rust
#[salsa::tracked]
pub fn lower_file(db: &dyn CadenzaDb, parsed: ParsedFile) -> HirModule {
    // Walk AST, produce HIR with spans
}
```

**TODO:**
- [ ] Implement `lower` module
- [ ] Desugar complex syntax to simple HIR
- [ ] Preserve spans through lowering
- [ ] Handle syntax errors gracefully

### Eval (TODO - `eval` module)

HIR evaluation and macro expansion:

```rust
#[salsa::tracked]
pub fn evaluate_module(db: &dyn CadenzaDb, hir: HirModule) -> ExpandedHirModule {
    // Evaluate compile-time expressions
    // Expand macros (HIR → HIR)
    // Preserve spans in generated code
}
```

**TODO:**
- [ ] Implement `eval` module
- [ ] Macro expansion on HIR
- [ ] Compile-time evaluation
- [ ] Track unevaluated branches for type checking

### Queries (TODO - `queries` module)

LSP and type inference queries:

```rust
#[salsa::tracked]
pub fn infer_types(db: &dyn CadenzaDb, expanded: ExpandedHirModule) -> TypedHirModule {
    // Type inference on expanded HIR
}

#[salsa::tracked]
pub fn hover_info(db: &dyn CadenzaDb, source: SourceFile, pos: Position) -> Option<HoverInfo> {
    // Find HIR node at position, return type/docs
}
```

**TODO:**
- [ ] Implement type inference queries
- [ ] Implement LSP hover
- [ ] Implement LSP completion
- [ ] Implement LSP go-to-definition
- [ ] Implement LSP find-references

## Comparison with cadenza-eval

| Feature | cadenza-eval (POC) | cadenza-core (New) |
|---------|-------------------|-------------------|
| IR | Works on AST | Works on HIR |
| Evaluation | Mutates Compiler state | Pure HIR → HIR |
| Spans | Lost during evaluation | Preserved throughout |
| LSP | Operates on pre-expansion AST | Operates on post-expansion HIR |
| Salsa | Phases 1-3 only | Full pipeline |
| Macro Expansion | AST mutation | HIR → HIR transformation |

## Migration Status

### Completed

✅ **Phase 1: Foundation**
- Salsa database infrastructure
- Database trait and implementation

✅ **Phase 2: Source Tracking**
- SourceFile input
- Mutable source tracking

✅ **Phase 3: Parsing**
- ParsedFile tracked struct
- parse_file tracked function
- Diagnostic accumulator

✅ **Initial HIR Definition**
- HIR expression types
- Span tracking
- HirId system

### In Progress

🚧 **Phase 4: HIR Lowering**
- Need to implement `lower` module
- AST → HIR transformation
- Span preservation

### TODO

⬜ **Phase 5: HIR Evaluation**
- Implement `eval` module
- Macro expansion (HIR → HIR)
- Compile-time evaluation
- Track unevaluated branches

⬜ **Phase 6: Type Inference**
- Type inference on expanded HIR
- Type checking queries
- Type error reporting

⬜ **Phase 7: LSP Integration**
- Hover information
- Completion
- Go-to-definition
- Find references
- All operating on expanded HIR

## Next Steps

1. **Implement Lower Module**
   - Create `lower.rs`
   - Implement AST → HIR transformation
   - Add tests for lowering

2. **Document Remaining Work**
   - Update this document with specific TODOs
   - Document design decisions
   - Add examples

3. **Implement Eval Module**
   - Create `eval.rs`
   - Implement macro expansion
   - Implement compile-time evaluation

4. **Move Documentation**
   - Copy relevant docs from `/docs/SALSA_MIGRATION.md`
   - Update with new HIR-based approach

## References

- [Salsa Framework](https://github.com/salsa-rs/salsa)
- [rust-analyzer HIR](https://github.com/rust-lang/rust-analyzer/blob/master/docs/dev/architecture.md)
- Original migration plan: `/docs/SALSA_MIGRATION.md`
- Phase 4 challenges: `/docs/SALSA_PHASE_4_CHALLENGES.md`
