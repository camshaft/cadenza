# Salsa Migration Plan

## Motivation

The current compiler architecture uses a direct evaluation model with mutable state (`Compiler` struct with `Map<Value>` for definitions). As Cadenza grows, we need a solid foundation for on-demand computation that allows us to query specific information efficiently without recomputing everything.

### Why Salsa?

[Salsa](https://github.com/salsa-rs/salsa) is a framework for incremental computation that provides a database-centric architecture. By organizing the toolchain around a Salsa database, we gain:

1. **On-demand computation**: Query only what you need, when you need it
2. **Automatic incrementality**: Salsa tracks dependencies and recomputes only what changed
3. **Extensibility**: Easy to add new queries without reinventing patterns
4. **Proven foundation**: Used in production by rust-analyzer

The database becomes the central organizing principle, making it straightforward to add LSP features, watch mode, and other functionality that needs to query compiler state.

## Current Architecture

```rust
pub struct Compiler {
    defs: Map<Value>,
    macros: Map<Value>,
    diagnostics: Vec<Diagnostic>,
    units: UnitRegistry,
    type_inferencer: TypeInferencer,
    ir_generator: Option<IrGenerator>,
    trait_registry: TraitRegistry,
}

pub struct EvalContext<'a> {
    env: &'a mut Env,
    compiler: &'a mut Compiler,
}

pub fn eval(ctx: &mut EvalContext, expr: &Expr) -> Result<Value> {
    // Mutates ctx.compiler during evaluation
    // No memoization
    // Must re-evaluate everything on changes
}
```

**Issues:**
- Mutable state makes it hard to reason about what's computed when
- No natural way to query specific information (e.g., "what's the type of this function?")
- Adding new queries requires threading state through the system
- Difficult to implement features like incremental compilation or efficient LSP

## Proposed Architecture with Salsa

### Core Concepts

**Database**: Central hub for all queries
```rust
#[salsa::db]
pub trait CadenzaDb: salsa::Database {}
```

**Inputs**: Mutable data from outside (source files)
```rust
#[salsa::input]
pub struct SourceFile {
    #[returns(ref)]
    pub text: String,
}
```

**Tracked Functions**: Pure functions that Salsa memoizes
```rust
#[salsa::tracked]
pub fn parse_file(db: &dyn CadenzaDb, source: SourceFile) -> ParsedFile<'_> {
    let text = source.text(db);
    let cst = cadenza_syntax::parse(text);
    ParsedFile::new(db, source, cst)
}
```

**Interned Types**: Deduplicated immutable values
```rust
#[salsa::interned]
pub struct Identifier<'db> {
    #[returns(ref)]
    pub text: String,
}
```

**Accumulators**: Collect diagnostics during computation
```rust
#[salsa::accumulator]
pub struct Diagnostic {
    pub span: Span,
    pub message: String,
}
```

### Benefits

1. **On-demand queries**: Can ask "what's the type at this position?" without computing types for the whole file
2. **Automatic memoization**: Salsa caches results and only recomputes when dependencies change
3. **Clear dependencies**: The database tracks what depends on what automatically
4. **Extensible**: Adding a new query is straightforward - just add a tracked function
5. **LSP-friendly**: Easy to implement hover, completion, etc. as queries against the database

## Migration Phases

### Phase 1: Foundation (1-2 days)

Add Salsa dependency and create database infrastructure.

**Tasks:**
- Add `salsa = "0.24"` to Cargo.toml
- Create `CadenzaDb` trait
- Create `CadenzaDbImpl` for CLI/testing
- Create thread-safe wrapper for LSP

```rust
#[salsa::db]
pub trait CadenzaDb: salsa::Database {}

#[salsa::db]
#[derive(Default)]
pub struct CadenzaDbImpl {
    storage: salsa::Storage<Self>,
}
```

### Phase 2: Source Tracking (2-3 days)

Migrate source text and string interning to Salsa.

**Tasks:**
- Define `SourceFile` input
- Define `Identifier` interned type
- Update interner to use Salsa

```rust
#[salsa::input]
pub struct SourceFile {
    #[returns(ref)]
    pub path: String,
    #[returns(ref)]
    pub text: String,
}

#[salsa::interned]
pub struct Identifier<'db> {
    #[returns(ref)]
    pub text: String,
}
```

### Phase 3: Parsing (3-4 days)

Make parsing a tracked function.

**Tasks:**
- Define `ParsedFile` tracked struct
- Create `parse_file` tracked function
- Setup diagnostic accumulator

```rust
#[salsa::tracked]
pub struct ParsedFile<'db> {
    pub source: SourceFile,
    #[tracked]
    #[returns(ref)]
    pub cst: SyntaxNode,
}

#[salsa::tracked]
pub fn parse_file(db: &dyn CadenzaDb, source: SourceFile) -> ParsedFile<'_> {
    let text = source.text(db);
    let cst = cadenza_syntax::parse(text);
    
    // Accumulate diagnostics
    for error in cst.errors() {
        Diagnostic {
            span: error.span(),
            message: error.message().to_string(),
        }.accumulate(db);
    }
    
    ParsedFile::new(db, source, cst)
}
```

### Phase 4: Evaluation (5-7 days)

Convert evaluation to tracked functions. This is the most complex phase.

**Tasks:**
- Define `CompiledModule` tracked struct
- Create `evaluate_module` tracked function
- Handle macro expansion (likely as separate query)
- Migrate `Env` to persistent data structure

```rust
#[salsa::tracked]
pub struct CompiledModule<'db> {
    pub source: SourceFile,
    #[tracked]
    #[returns(ref)]
    pub definitions: Map<Identifier<'db>, Value>,
    #[tracked]
    #[returns(ref)]
    pub macros: Map<Identifier<'db>, Value>,
}

#[salsa::tracked]
pub fn evaluate_module(
    db: &dyn CadenzaDb,
    parsed: ParsedFile<'_>
) -> CompiledModule<'_> {
    // Pure evaluation - no mutation
    // Returns immutable result
}
```

**Challenge**: Current evaluator mutates state. Need to make it pure.

**Solution**: Use persistent data structures (e.g., `im` crate) and return new values instead of mutating.

### Phase 5: Type Inference (4-5 days)

Make type checking a set of queries.

**Tasks:**
- Define `infer_function_type` query
- Implement `type_at_position` query for LSP
- Use accumulators for type errors

```rust
#[salsa::tracked]
pub fn infer_function_type(
    db: &dyn CadenzaDb,
    module: CompiledModule<'_>,
    func_name: Identifier<'_>
) -> InferredType {
    // Type check just this function
}

#[salsa::tracked]
pub fn type_at_position(
    db: &dyn CadenzaDb,
    source: SourceFile,
    line: u32,
    column: u32
) -> Option<Type> {
    // For LSP hover
    let parsed = parse_file(db, source);
    let module = evaluate_module(db, parsed);
    // Find and return type at position
}
```

### Phase 6: LSP Integration (3-4 days)

Wire LSP server to query the database.

**Tasks:**
- Create thread-safe `LspDatabase` wrapper
- Implement LSP handlers as database queries
- Handle file change notifications

```rust
pub struct LspDatabase {
    db: Mutex<CadenzaDbImpl>,
    files: RwLock<HashMap<Url, SourceFile>>,
}

impl LspDatabase {
    pub fn hover(&self, uri: &Url, position: Position) -> Option<Hover> {
        let db = self.db.lock();
        let source = self.files.read().get(uri).copied()?;
        let ty = type_at_position(&*db, source, position.line, position.character)?;
        Some(Hover { /* format type */ })
    }
    
    pub fn did_change(&self, uri: &Url, text: String) {
        let mut db = self.db.lock();
        let source = self.files.read().get(uri).copied()?;
        // This invalidates dependent queries automatically
        source.set_text(&mut *db).to(text);
    }
}
```

### Phase 7: Optimization (2-3 days)

Fine-tune Salsa configuration.

**Tasks:**
- Configure durability for rarely-changing inputs
- Optimize query granularity
- Add performance logging
- Profile and identify bottlenecks

## Implementation Strategy

### Incremental Approach

- Build new Salsa code alongside existing code
- Use feature flags to enable new path
- Each phase is independently testable
- Validate thoroughly before moving to next phase

### Testing

- Unit tests for each query
- Integration tests for query composition
- Test memoization behavior
- Verify incrementality works correctly

### Rollout

1. **Alpha**: Feature-flagged, not default
2. **Beta**: Default path, old code still available
3. **Stable**: Remove old path
4. **Polish**: Optimize based on real usage

## Inspiration: Salsa Calc Example

The migration follows patterns from [Salsa's calc example](https://github.com/salsa-rs/salsa/tree/master/examples/calc):

```rust
// Input
#[salsa::input]
pub struct SourceProgram {
    #[returns(ref)]
    pub text: String,
}

// Interned identifiers
#[salsa::interned]
pub struct FunctionId<'db> {
    #[returns(ref)]
    pub text: String,
}

// Tracked parsing
#[salsa::tracked]
pub fn parse_statements(db: &dyn Db, source: SourceProgram) -> Program<'_> {
    // Parser implementation
}

// Accumulator for diagnostics
#[salsa::accumulator]
pub struct Diagnostic {
    pub start: usize,
    pub end: usize,
    pub message: String,
}

// Database
#[salsa::db]
pub trait Db: salsa::Database {}
```

## Key Challenges

### 1. Macro Expansion

**Problem**: Current macros mutate compiler state.

**Solution**: Make macro expansion return new CST nodes. Track as separate query.

### 2. Mutable Environment

**Problem**: Current `Env` uses mutable scopes.

**Solution**: Switch to persistent data structures. Pass environments as values.

### 3. Pure Functions

**Problem**: Need to convert stateful code to pure functions.

**Solution**: Return new values instead of mutating. Use `im` crate for efficient immutable collections.

## Success Criteria

- All existing tests pass
- LSP queries work correctly
- No regressions in behavior
- Code is cleaner and more maintainable
- Easy to add new queries

## Timeline

| Phase | Duration | Dependencies |
|-------|----------|--------------|
| Phase 1: Foundation | 1-2 days | None |
| Phase 2: Source Tracking | 2-3 days | Phase 1 |
| Phase 3: Parsing | 3-4 days | Phase 2 |
| Phase 4: Evaluation | 5-7 days | Phase 3 |
| Phase 5: Type Inference | 4-5 days | Phase 4 |
| Phase 6: LSP Integration | 3-4 days | Phase 5 |
| Phase 7: Optimization | 2-3 days | Phase 6 |
| **Total** | **20-28 days** | Sequential |

## References

- [Salsa GitHub Repository](https://github.com/salsa-rs/salsa)
- [Salsa Book](https://salsa-rs.github.io/salsa)
- [Salsa Calc Example](https://github.com/salsa-rs/salsa/tree/master/examples/calc)
- [rust-analyzer architecture](https://github.com/rust-lang/rust-analyzer/blob/master/docs/dev/architecture.md)

## Next Steps

1. Review and approve this plan
2. Create feature branch: `feature/salsa-migration`
3. Start Phase 1: Add Salsa dependency
4. Implement phases incrementally
5. Regular progress updates
