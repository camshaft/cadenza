# Salsa Phase 4 Challenges and Revised Approach

## Initial Attempt

Following the original `SALSA_MIGRATION.md` plan, we attempted to implement Phase 4 by:

1. Adding `im` crate for persistent data structures
2. Defining `CompiledModule` tracked struct with definitions/macros
3. Creating `evaluate_module` tracked function

## Problems Discovered

### Value Cannot Be Made Salsa-Compatible

Salsa tracked structs require fields to be `Send + Sync + Hash`. However, our `Value` enum contains:

1. **Function pointers** (`BuiltinFn`, `BuiltinMacro`)
2. **User closures** (`UserFunction` captures environment)
3. **Static references** (`SpecialForm` with `&'static`)

These cannot be made `Send + Sync + Hash` without major architectural changes:

```rust
// These don't work with Salsa:
pub struct BuiltinFn {
    pub func: fn(&[Value], &mut EvalContext<'_>) -> Result<Value>,  // fn pointer
}

pub struct UserFunction {
    pub env: Env,  // Contains Rc, not Send/Sync
    pub params: Vec<InternedString>,
    pub body: Expr,
}
```

### Why This Is Hard

Making `Value` Salsa-compatible would require:

1. **Value IDs instead of Values**: Replace direct Value storage with indices/IDs
2. **Separate value arena**: Store actual values outside Salsa
3. **Pure evaluation**: Remove all mutation, use persistent data structures
4. **Serializable closures**: Fundamentally challenging in Rust

This is essentially a rewrite of the evaluation system - much larger than a single phase.

## Revised Approach

Instead of trying to make evaluation fully Salsa-based in Phase 4, we should:

### Phase 4a: Symbol Collection (Achievable Now)

Add lightweight Salsa queries for information extraction:

```rust
#[salsa::interned]
pub struct Symbol<'db> {
    #[returns(ref)]
    pub name: String,
}

#[salsa::tracked]
pub struct SymbolTable<'db> {
    pub source: ParsedFile<'db>,
    #[returns(ref)]
    pub definitions: Vec<(Symbol<'db>, Span)>,
    #[returns(ref)]
    pub references: Vec<(Symbol<'db>, Span)>,
}

#[salsa::tracked]
pub fn collect_symbols(db: &dyn CadenzaDb, parsed: ParsedFile<'_>) -> SymbolTable<'_> {
    // Walk CST, extract defined names and their locations
    // This is pure - no evaluation needed
}
```

**Benefits:**
- Enables "Go to Definition" in LSP
- Enables "Find References" in LSP
- No evaluation required - just AST walking
- Fully compatible with Salsa

### Phase 4b: Macro Expansion as Query (Later)

Macro expansion is AST → AST, which could work with Salsa:

```rust
#[salsa::tracked]
pub fn expand_macros(db: &dyn CadenzaDb, parsed: ParsedFile<'_>) -> ParsedFile<'_> {
    // Apply macro expansion to produce new CST
    // This is pure transformation
}
```

### Phase 4c: Type Inference Integration (Much Later)

Once type inference is working, integrate it:

```rust
#[salsa::tracked]
pub fn infer_types(db: &dyn CadenzaDb, parsed: ParsedFile<'_>) -> TypedModule<'_> {
    // Run type inference
    // Store type information in Salsa
}
```

### Keep Mutable Evaluation Path

The current mutable `Compiler` + `EvalContext` + `Env` architecture should remain as the primary evaluation path. It works well for:

- REPL
- Script execution
- Code generation

Salsa queries can be added around it for LSP features without requiring a full rewrite.

## Lessons Learned

1. **Start with metadata, not execution**: Symbol tables, dependency graphs, and type information are easier to make Salsa-compatible than full evaluation.

2. **AST queries are tractable**: Queries that operate on syntax trees (parsing, symbol collection, macro expansion) work well with Salsa.

3. **Execution can stay mutable**: There's no requirement to make evaluation pure. LSP features can be built on metadata queries.

4. **Value representation matters**: If we want to make evaluation Salsa-based in the future, we'd need to redesign Value to use IDs/indices instead of direct storage.

## Next Steps

1. **Implement Phase 4a**: Symbol collection query (good next task)
2. **Update SALSA_MIGRATION.md**: Revise Phase 4 to reflect these learnings
3. **Focus on LSP metadata queries**: Build out queries for symbols, types, references
4. **Defer full evaluation migration**: This can be a much later phase if needed at all

## References

- rust-analyzer uses HIR (High-level IR) with IDs, not direct value storage
- Salsa calc example keeps evaluation separate from database
- LSP features like hover/completion can work with metadata queries alone
