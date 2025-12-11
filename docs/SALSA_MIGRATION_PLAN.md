# Salsa Migration Plan

## Executive Summary

This document outlines a comprehensive plan to migrate Cadenza's compiler infrastructure to use [Salsa](https://github.com/salsa-rs/salsa) version 0.24, a framework for incremental computation. This migration will address key concerns about:

- **Incremental compilation**: Only recompute what changes when source files are modified
- **Fast iteration**: Enable quick feedback cycles for development
- **Efficient LSP queries**: Query types for single functions without recomputing the entire program
- **Memory efficiency**: Share computation results across multiple queries

## Background

### Current Architecture

Cadenza currently uses a direct evaluation model:

```rust
pub struct Compiler {
    defs: Map<Value>,                    // Variable/function definitions
    macros: Map<Value>,                  // Macro definitions
    diagnostics: Vec<Diagnostic>,        // Accumulated diagnostics
    units: UnitRegistry,                 // Dimensional analysis
    type_inferencer: TypeInferencer,     // Type inference
    ir_generator: Option<IrGenerator>,   // Code generation
    trait_registry: TraitRegistry,       // Trait system
}
```

Evaluation happens via:
```rust
pub struct EvalContext<'a> {
    env: &'a mut Env,           // Scoped bindings
    compiler: &'a mut Compiler, // Shared state
}
```

**Problems with current approach:**
1. No incremental computation - every evaluation starts from scratch
2. No memoization of results - repeated queries recompute everything
3. Difficult to determine what needs recomputation when inputs change
4. LSP queries require full re-evaluation for accurate results
5. No way to share work between different query types (e.g., type checking vs code generation)

### Why Salsa?

Salsa provides:
1. **Automatic incrementality**: Tracks dependencies and only recomputes what changes
2. **Memoization**: Caches results of pure functions automatically
3. **Fine-grained queries**: Query specific information without computing everything
4. **LSP-friendly**: Used in rust-analyzer, designed for IDE integration
5. **Accumulator pattern**: Collect diagnostics/errors across the compilation pipeline

### Inspiration: Salsa Calc Example

The calc example demonstrates key patterns we'll follow:

```rust
// Input - base data that can be mutated
#[salsa::input]
pub struct SourceProgram {
    #[returns(ref)]
    pub text: String,
}

// Interned - deduplicated immutable values
#[salsa::interned]
pub struct FunctionId<'db> {
    #[returns(ref)]
    pub text: String,
}

// Tracked function - memoized computation
#[salsa::tracked]
pub fn parse_statements(db: &dyn Db, source: SourceProgram) -> Program<'_> {
    // Parse implementation
}

// Accumulator - collect diagnostics
#[salsa::accumulator]
pub struct Diagnostic {
    pub start: usize,
    pub end: usize,
    pub message: String,
}

// Database trait - ties everything together
#[salsa::db]
pub trait Db: salsa::Database {}
```

## Migration Strategy

### Overview

The migration will happen in phases to minimize disruption and allow for incremental validation:

1. **Phase 1**: Foundation - Add Salsa dependency and database infrastructure
2. **Phase 2**: Inputs - Convert source text and interner to Salsa inputs
3. **Phase 3**: Parsing - Make parsing a Salsa tracked function
4. **Phase 4**: Evaluation - Convert evaluation to Salsa tracked functions
5. **Phase 5**: Type Inference - Adapt type checking to use Salsa queries
6. **Phase 6**: LSP Integration - Wire LSP to query the Salsa database
7. **Phase 7**: Optimization - Fine-tune performance and incrementality

### Key Design Decisions

1. **Database as central hub**: All compilation state flows through the Salsa database
2. **Inputs are immutable**: Source files become `#[salsa::input]` structs
3. **Tracked functions are pure**: All compiler phases become pure, tracked functions
4. **Accumulators for diagnostics**: Errors/warnings collected via accumulators
5. **Intern identifiers**: Function names, type names, etc. use `#[salsa::interned]`

## Detailed Migration Plan

### Phase 1: Foundation and Database Setup

**Goal**: Add Salsa as a dependency and create the core database infrastructure.

**Tasks**:

1. **Add Salsa dependency** (`Cargo.toml`)
   ```toml
   [dependencies]
   salsa = "0.24"
   ```

2. **Create database trait** (`crates/cadenza-eval/src/db.rs`)
   ```rust
   /// The Cadenza compiler database.
   /// All compilation queries flow through this trait.
   #[salsa::db]
   pub trait CadenzaDb: salsa::Database {}
   ```

3. **Create database implementation** (for testing and CLI)
   ```rust
   #[salsa::db]
   #[derive(Default)]
   pub struct CadenzaDbImpl {
       storage: salsa::Storage<Self>,
   }
   
   #[salsa::db]
   impl salsa::Database for CadenzaDbImpl {}
   
   #[salsa::db]
   impl CadenzaDb for CadenzaDbImpl {}
   ```

4. **Create database wrapper for LSP**
   - Need thread-safe wrapper with parking_lot::Mutex for LSP
   - Pattern from rust-analyzer: `RootDatabase`

**Success Criteria**:
- Salsa compiles and basic database can be created
- Unit tests pass demonstrating database creation
- No impact on existing functionality (runs in parallel)

**Estimated Effort**: 1-2 days

---

### Phase 2: Source Tracking and Interner

**Goal**: Migrate source text tracking and string interning to Salsa.

**Tasks**:

1. **Define SourceFile input**
   ```rust
   #[salsa::input]
   pub struct SourceFile {
       #[returns(ref)]
       pub path: String,
       
       #[returns(ref)]
       pub text: String,
   }
   ```

2. **Define interned identifiers**
   ```rust
   #[salsa::interned]
   pub struct Identifier<'db> {
       #[returns(ref)]
       pub text: String,
   }
   ```

3. **Migrate InternedString to use Salsa**
   - Current: `InternedString` uses custom interner
   - New: Use `Identifier<'db>` with Salsa's interning
   - Benefits: Automatic deduplication, tied to database lifetime

4. **Update syntax nodes to reference SourceFile**
   - Keep rowan CST structure
   - Add `SourceFile` reference to root nodes
   - Enables tracking which file a node comes from

**Success Criteria**:
- Can create SourceFile inputs and read their text
- Identifiers intern correctly and compare by identity
- Existing tests pass with new interner (adapter layer if needed)

**Estimated Effort**: 2-3 days

---

### Phase 3: Parsing as Tracked Functions

**Goal**: Make parsing incremental - only re-parse files that changed.

**Tasks**:

1. **Define AST types for Salsa**
   ```rust
   #[salsa::tracked]
   pub struct ParsedFile<'db> {
       #[tracked]
       pub source: SourceFile,
       
       #[tracked]
       #[returns(ref)]
       pub cst_root: SyntaxNode,
   }
   ```

2. **Create tracked parsing function**
   ```rust
   #[salsa::tracked]
   pub fn parse_file(db: &dyn CadenzaDb, source: SourceFile) -> ParsedFile<'_> {
       let text = source.text(db);
       let cst = cadenza_syntax::parse(text);
       ParsedFile::new(db, source, cst)
   }
   ```

3. **Setup diagnostic accumulator**
   ```rust
   #[salsa::accumulator]
   pub struct ParseDiagnostic {
       pub span: Span,
       pub message: String,
       pub severity: DiagnosticLevel,
   }
   ```

4. **Emit diagnostics during parsing**
   ```rust
   #[salsa::tracked]
   pub fn parse_file(db: &dyn CadenzaDb, source: SourceFile) -> ParsedFile<'_> {
       // ... parse ...
       for error in errors {
           ParseDiagnostic { /* ... */ }.accumulate(db);
       }
       // ...
   }
   ```

**Success Criteria**:
- Parsing a file produces ParsedFile result
- Parsing is memoized - unchanged files return cached CST
- Diagnostics accumulate correctly
- Can query diagnostics: `parse_file::accumulated::<ParseDiagnostic>(db, source)`

**Estimated Effort**: 3-4 days

---

### Phase 4: Evaluation and Compiler State

**Goal**: Convert the evaluator to use Salsa tracked functions, preserving macro expansion.

This is the most complex phase due to the current mutable evaluation model.

**Challenges**:
- Current evaluator mutates `Compiler` state during evaluation
- Salsa tracked functions must be pure (no mutation)
- Macros need to be handled carefully

**Approach**:

1. **Define module result structure**
   ```rust
   #[salsa::tracked]
   pub struct CompiledModule<'db> {
       #[tracked]
       pub source: SourceFile,
       
       #[tracked]
       #[returns(ref)]
       pub definitions: Map<Identifier<'db>, Value>,
       
       #[tracked]
       #[returns(ref)]
       pub macros: Map<Identifier<'db>, Value>,
       
       #[tracked]
       #[returns(ref)]
       pub types: Map<Identifier<'db>, Type>,
   }
   ```

2. **Create evaluation query**
   ```rust
   #[salsa::tracked]
   pub fn evaluate_module(
       db: &dyn CadenzaDb, 
       parsed: ParsedFile<'_>
   ) -> CompiledModule<'_> {
       // Build definitions from CST
       // Expand macros (tricky part!)
       // Return immutable result
   }
   ```

3. **Handle macros**
   - Option A: Expand macros during evaluation, track expanded CST
   - Option B: Make macro expansion a separate tracked query
   - **Recommended**: Option B for better incrementality
   
   ```rust
   #[salsa::tracked]
   pub fn expand_macros(
       db: &dyn CadenzaDb,
       parsed: ParsedFile<'_>
   ) -> ExpandedModule<'_> {
       // Return CST with macros expanded
   }
   ```

4. **Migrate Env to be immutable**
   - Current: Env is mutable with push_scope/pop_scope
   - New: Env becomes a persistent data structure (arc'd maps)
   - Use im-rc or similar for efficient immutable collections

**Success Criteria**:
- Module evaluation produces CompiledModule
- Definitions are tracked and cached
- Changing one definition only re-evaluates dependent definitions
- Macro expansion works incrementally

**Estimated Effort**: 5-7 days

---

### Phase 5: Type Inference with Salsa

**Goal**: Make type checking incremental - only re-check affected functions.

**Tasks**:

1. **Define type checking queries**
   ```rust
   #[salsa::tracked]
   pub fn infer_function_type(
       db: &dyn CadenzaDb,
       module: CompiledModule<'_>,
       func_name: Identifier<'_>
   ) -> InferredType {
       // Run Hindley-Milner inference for single function
   }
   ```

2. **Implement fine-grained type queries**
   ```rust
   #[salsa::tracked]
   pub fn type_at_position(
       db: &dyn CadenzaDb,
       source: SourceFile,
       position: Position
   ) -> Option<Type> {
       // For LSP hover - get type at specific position
   }
   ```

3. **Track constraints as separate query**
   ```rust
   #[salsa::tracked]
   pub fn collect_constraints(
       db: &dyn CadenzaDb,
       func: FunctionDef<'_>
   ) -> Vec<Constraint> {
       // Collect type constraints for function
   }
   ```

4. **Use accumulators for type errors**
   ```rust
   #[salsa::accumulator]
   pub struct TypeError {
       pub span: Span,
       pub message: String,
   }
   ```

**Success Criteria**:
- Can query type of individual functions
- Changing one function only re-checks its dependents
- Type errors accumulate correctly
- LSP can get type at position efficiently

**Estimated Effort**: 4-5 days

---

### Phase 6: LSP Integration

**Goal**: Wire the LSP server to query the Salsa database for fast responses.

**Tasks**:

1. **Create LSP database wrapper**
   ```rust
   pub struct LspDatabase {
       db: parking_lot::Mutex<CadenzaDbImpl>,
   }
   ```

2. **Implement LSP query methods**
   ```rust
   impl LspDatabase {
       pub fn hover(&self, uri: Url, position: Position) -> Option<Hover> {
           let db = self.db.lock();
           let source = self.uri_to_source(&db, &uri)?;
           let ty = type_at_position(&db, source, position)?;
           Some(Hover { contents: format_type(&ty) })
       }
   }
   ```

3. **Handle file changes**
   ```rust
   pub fn did_change(&self, uri: Url, text: String) {
       let mut db = self.db.lock();
       let source = self.uri_to_source(&db, &uri);
       source.set_text(&mut db).to(text);
       // Salsa automatically invalidates affected queries!
   }
   ```

4. **Implement completion**
   ```rust
   #[salsa::tracked]
   pub fn completions_at_position(
       db: &dyn CadenzaDb,
       source: SourceFile,
       position: Position
   ) -> Vec<CompletionItem> {
       // Get visible identifiers at position
   }
   ```

**Success Criteria**:
- LSP hover shows types
- LSP completion works
- File edits trigger minimal recomputation
- Response times under 100ms for typical queries

**Estimated Effort**: 3-4 days

---

### Phase 7: Optimization and Tuning

**Goal**: Fine-tune Salsa configuration for optimal performance.

**Tasks**:

1. **Configure durability**
   - Mark rarely-changing inputs as high durability
   - Example: Stdlib definitions never change during session

2. **Optimize query granularity**
   - Too coarse: Poor incrementality
   - Too fine: Query overhead dominates
   - Find the sweet spot through profiling

3. **Implement query groups**
   ```rust
   #[salsa::db]
   pub trait ParserQueries: CadenzaDb {
       fn parse_file(&self, source: SourceFile) -> ParsedFile<'_>;
   }
   
   #[salsa::db]
   pub trait TypeCheckerQueries: CadenzaDb {
       fn infer_type(&self, func: FunctionDef<'_>) -> Type;
   }
   ```

4. **Add performance logging**
   - Track query execution times
   - Identify hotspots
   - Use Salsa's built-in event logging

5. **Benchmark against current implementation**
   - Measure initial parse time (will be similar)
   - Measure incremental recompilation (should be much faster)
   - Measure LSP query latency (should be much faster)

**Success Criteria**:
- Incremental recompilation 10x faster than full recompilation
- LSP queries respond in under 50ms on average
- Memory usage reasonable (Salsa caches results)
- Documentation on performance characteristics

**Estimated Effort**: 2-3 days

---

## Implementation Guidelines

### General Principles

1. **Incremental Migration**: Keep existing code working during migration
2. **Feature Flags**: Use feature flags to enable new Salsa-based code paths
3. **Parallel Implementation**: Build Salsa version alongside current version
4. **Comprehensive Testing**: Add tests for each phase before moving to next
5. **Documentation**: Update architecture docs as we go

### Testing Strategy

1. **Unit Tests**: Test each Salsa query in isolation
2. **Integration Tests**: Test query composition and incrementality
3. **Snapshot Tests**: Verify diagnostic output matches expected
4. **Performance Tests**: Benchmark incremental vs full recompilation
5. **LSP Tests**: Test LSP integration with real-world scenarios

### Rollout Plan

1. **Alpha**: Salsa implementation feature-flagged, not default
2. **Beta**: Salsa becomes default, old code path still available
3. **Stable**: Remove old evaluation path entirely
4. **Polish**: Optimize performance, improve error messages

## Expected Benefits

### Immediate Benefits

1. **LSP Performance**: Hover, completion, etc. will be near-instant
2. **Incremental Compilation**: Only recompile changed functions
3. **Memory Efficiency**: Shared computation across queries
4. **Better Error Recovery**: Continue compilation despite errors

### Long-term Benefits

1. **Watch Mode**: Implement file watching for REPL/compiler
2. **Parallel Compilation**: Salsa supports parallel query evaluation
3. **Better Debugging**: Query dependency graphs for debugging
4. **Easier Experimentation**: Try different compilation strategies without breaking everything

## Potential Challenges

### Challenge 1: Macro Expansion

**Issue**: Current macro expansion mutates compiler state.

**Solution**: 
- Make macro expansion return new CST nodes
- Track expanded CST as separate query result
- May need multiple expansion passes

### Challenge 2: Mutable Environment

**Issue**: Current Env uses mutable scopes.

**Solution**:
- Switch to persistent data structures (im-rc)
- Pass environments as values, not references
- Use Arc for sharing without cloning

### Challenge 3: Performance Overhead

**Issue**: Salsa adds overhead for tracking dependencies.

**Solution**:
- Coarser queries for hot paths
- Cache expensive operations
- Profile and optimize critical paths

### Challenge 4: Debugging

**Issue**: Salsa's memoization can make debugging harder.

**Solution**:
- Use Salsa's debug logging
- Add query visualization tools
- Comprehensive unit tests for each query

## Migration Risks and Mitigation

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Performance regression | High | Medium | Benchmark continuously, optimize hot paths |
| Breaking changes | High | High | Feature flags, parallel implementation |
| Macro expansion complexity | Medium | High | Prototype early, get feedback |
| Learning curve | Medium | Medium | Good documentation, examples |
| Incomplete migration | High | Medium | Phase-by-phase approach, each phase is useful standalone |

## Timeline Estimate

| Phase | Duration | Dependencies |
|-------|----------|--------------|
| Phase 1: Foundation | 1-2 days | None |
| Phase 2: Inputs & Interning | 2-3 days | Phase 1 |
| Phase 3: Parsing | 3-4 days | Phase 2 |
| Phase 4: Evaluation | 5-7 days | Phase 3 |
| Phase 5: Type Inference | 4-5 days | Phase 4 |
| Phase 6: LSP Integration | 3-4 days | Phase 5 |
| Phase 7: Optimization | 2-3 days | Phase 6 |
| **Total** | **20-28 days** | Sequential |

Note: Timeline assumes full-time work on migration. Can be parallelized partially.

## Success Metrics

1. **Performance**:
   - Incremental recompilation: 10x faster than full recompilation
   - LSP hover latency: < 50ms p95
   - LSP completion latency: < 100ms p95

2. **Correctness**:
   - All existing tests pass
   - No regressions in behavior
   - Diagnostics match expected output

3. **Developer Experience**:
   - Clear documentation on Salsa usage
   - Easy to add new queries
   - Good error messages when things go wrong

## References

- [Salsa GitHub Repository](https://github.com/salsa-rs/salsa)
- [Salsa Book](https://salsa-rs.github.io/salsa)
- [Salsa Calc Example](https://github.com/salsa-rs/salsa/tree/master/examples/calc)
- [rust-analyzer architecture](https://github.com/rust-lang/rust-analyzer/blob/master/docs/dev/architecture.md)
- Current Cadenza architecture: `docs/COMPILER_ARCHITECTURE.md`

## Next Steps

1. **Review and Approve Plan**: Get feedback on this plan from stakeholders
2. **Create Feature Branch**: `feature/salsa-migration`
3. **Start Phase 1**: Add Salsa dependency and create database
4. **Regular Status Updates**: Weekly progress reports
5. **Iterative Refinement**: Adjust plan based on learnings

## Questions for Discussion

1. Should we target Salsa 0.24 or wait for a newer version?
2. What's our tolerance for temporary performance regressions during migration?
3. Should we keep the old evaluation path as a fallback indefinitely?
4. How do we handle the WASM build? (Salsa may need special handling for WASM)
5. What's the priority: LSP performance or incremental compilation?

## Appendix: Key Salsa Concepts

### Database

The central hub of Salsa. All queries go through the database.

```rust
#[salsa::db]
pub trait MyDb: salsa::Database {}
```

### Input

Base data that can be mutated from outside.

```rust
#[salsa::input]
pub struct SourceFile {
    #[returns(ref)]
    pub text: String,
}
```

### Tracked Function

A memoized pure function. Results are cached.

```rust
#[salsa::tracked]
pub fn parse(db: &dyn MyDb, source: SourceFile) -> Ast {
    // implementation
}
```

### Interned

Deduplicated immutable values.

```rust
#[salsa::interned]
pub struct Identifier<'db> {
    #[returns(ref)]
    pub text: String,
}
```

### Accumulator

Collect values during computation.

```rust
#[salsa::accumulator]
pub struct Diagnostic {
    pub message: String,
}
```

### Tracked Struct

Like input but created during computation.

```rust
#[salsa::tracked]
pub struct Function<'db> {
    pub name: Identifier<'db>,
    #[tracked]
    pub body: Expr<'db>,
}
```
