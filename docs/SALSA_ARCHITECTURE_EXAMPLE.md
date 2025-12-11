# Salsa Architecture Example

This document shows concrete examples of how Cadenza's architecture will look after migrating to Salsa, based on patterns from the [Salsa calc example](https://github.com/salsa-rs/salsa/tree/master/examples/calc).

## Before and After Comparison

### Current Architecture (Before)

```rust
// Current: Mutable compiler state
pub struct Compiler {
    defs: Map<Value>,
    macros: Map<Value>,
    diagnostics: Vec<Diagnostic>,
    units: UnitRegistry,
    type_inferencer: TypeInferencer,
    ir_generator: Option<IrGenerator>,
    trait_registry: TraitRegistry,
}

// Current: Mutable evaluation context
pub struct EvalContext<'a> {
    env: &'a mut Env,
    compiler: &'a mut Compiler,
}

// Current: Direct evaluation
pub fn eval(ctx: &mut EvalContext, expr: &Expr) -> Result<Value> {
    // Mutates ctx.compiler during evaluation
    // No memoization
    // Must re-evaluate everything on changes
}
```

### After Salsa Migration

```rust
// After: Immutable database
#[salsa::db]
pub trait CadenzaDb: salsa::Database {}

// After: Input for source files
#[salsa::input]
pub struct SourceFile {
    #[returns(ref)]
    pub path: String,
    
    #[returns(ref)]
    pub text: String,
}

// After: Tracked parsing
#[salsa::tracked]
pub fn parse_file(db: &dyn CadenzaDb, source: SourceFile) -> ParsedFile<'_> {
    // Memoized - only re-parses if source.text changed
    let text = source.text(db);
    let cst = cadenza_syntax::parse(text);
    ParsedFile::new(db, source, cst)
}

// After: Tracked evaluation
#[salsa::tracked]
pub fn evaluate_module(
    db: &dyn CadenzaDb,
    parsed: ParsedFile<'_>
) -> CompiledModule<'_> {
    // Memoized - only re-evaluates if CST changed
    // Returns immutable result
    // No mutation, pure function
}
```

## Core Database Infrastructure

### Database Trait

```rust
/// The Cadenza compiler database.
/// 
/// All compilation queries flow through this trait. It provides
/// access to all Salsa-tracked data and functions.
#[salsa::db]
pub trait CadenzaDb: salsa::Database {
    // No methods needed here - Salsa macro adds them
}
```

### Database Implementation (for CLI and Testing)

```rust
/// Default database implementation for CLI and tests.
#[salsa::db]
#[derive(Default)]
pub struct CadenzaDbImpl {
    storage: salsa::Storage<Self>,
}

#[salsa::db]
impl salsa::Database for CadenzaDbImpl {}

#[salsa::db]
impl CadenzaDb for CadenzaDbImpl {}

impl CadenzaDbImpl {
    /// Create a new database instance.
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Create a database with event logging enabled (for testing).
    #[cfg(test)]
    pub fn with_logging() -> Self {
        Self {
            storage: salsa::Storage::new(Some(Box::new(|event| {
                eprintln!("Salsa event: {:?}", event);
            }))),
        }
    }
}
```

### Database for LSP (Thread-Safe)

```rust
/// Thread-safe database wrapper for LSP.
/// 
/// The LSP server needs to handle requests concurrently,
/// so we need a thread-safe wrapper around the database.
pub struct LspDatabase {
    db: parking_lot::Mutex<CadenzaDbImpl>,
    // Cache file URI -> SourceFile mappings
    file_map: parking_lot::RwLock<HashMap<Url, SourceFile>>,
}

impl LspDatabase {
    pub fn new() -> Self {
        Self {
            db: parking_lot::Mutex::new(CadenzaDbImpl::new()),
            file_map: parking_lot::RwLock::new(HashMap::new()),
        }
    }
    
    /// Handle file opened in editor
    pub fn did_open(&self, uri: Url, text: String) {
        let mut db = self.db.lock();
        let source = SourceFile::new(&*db, uri.to_string(), text);
        
        let mut file_map = self.file_map.write();
        file_map.insert(uri, source);
    }
    
    /// Handle file changed in editor
    pub fn did_change(&self, uri: Url, text: String) {
        let file_map = self.file_map.read();
        if let Some(&source) = file_map.get(&uri) {
            let mut db = self.db.lock();
            // This is the magic! Setting the text invalidates all
            // dependent queries automatically.
            source.set_text(&mut *db).to(text);
        }
    }
    
    /// Handle file closed in editor
    pub fn did_close(&self, uri: &Url) {
        let mut file_map = self.file_map.write();
        file_map.remove(uri);
        // Note: We don't remove from Salsa db - it will GC eventually
    }
}
```

## Input Types

### Source Files

```rust
/// A source file in the Cadenza project.
/// 
/// This is an input - it can be mutated from outside the computation.
/// When the text changes, all queries that depend on this file will
/// be re-executed (if needed).
#[salsa::input]
pub struct SourceFile {
    /// The file path (for diagnostics and identification)
    #[returns(ref)]
    pub path: String,
    
    /// The source text
    #[returns(ref)]
    pub text: String,
}

// Example usage:
fn example(db: &dyn CadenzaDb) {
    // Create a source file
    let source = SourceFile::new(
        db,
        "example.cdz".to_string(),
        "let x = 42".to_string(),
    );
    
    // Read the text
    let text: &String = source.text(db);
    
    // Update the text (requires &mut db)
    // source.set_text(&mut db).to("let x = 43".to_string());
}
```

## Interned Types

### Identifiers

```rust
/// An interned identifier (variable name, function name, etc.)
/// 
/// Salsa automatically deduplicates these - multiple "foo" identifiers
/// will have the same underlying ID, making comparisons very cheap.
#[salsa::interned]
pub struct Identifier<'db> {
    #[returns(ref)]
    pub text: String,
}

// Example usage:
fn example(db: &dyn CadenzaDb) {
    let id1 = Identifier::new(db, "foo".to_string());
    let id2 = Identifier::new(db, "foo".to_string());
    
    // These are the same interned value
    assert_eq!(id1, id2);
    
    // Comparison is just pointer equality, very fast
    let id3 = Identifier::new(db, "bar".to_string());
    assert_ne!(id1, id3);
}
```

### Type Names

```rust
/// An interned type name.
#[salsa::interned]
pub struct TypeName<'db> {
    #[returns(ref)]
    pub name: String,
}
```

### Module Paths

```rust
/// An interned module path (e.g., "std.math" or "my_lib.utils").
#[salsa::interned]
pub struct ModulePath<'db> {
    #[returns(ref)]
    pub segments: Vec<String>,
}
```

## Tracked Structs

### Parsed File

```rust
/// A parsed source file.
/// 
/// This is a tracked struct - it's created during computation and
/// tracked by Salsa. If the input (source) changes, this will be
/// recomputed.
#[salsa::tracked]
pub struct ParsedFile<'db> {
    /// The source file that was parsed
    pub source: SourceFile,
    
    /// The parsed CST (concrete syntax tree)
    #[tracked]
    #[returns(ref)]
    pub cst: SyntaxNode,
    
    /// Number of parse errors (for quick checking)
    #[tracked]
    pub error_count: usize,
}
```

### Compiled Module

```rust
/// A compiled module with all definitions.
#[salsa::tracked]
pub struct CompiledModule<'db> {
    /// The source file
    pub source: SourceFile,
    
    /// Variable and function definitions
    #[tracked]
    #[returns(ref)]
    pub definitions: Map<Identifier<'db>, Value>,
    
    /// Macro definitions
    #[tracked]
    #[returns(ref)]
    pub macros: Map<Identifier<'db>, Value>,
    
    /// Type definitions
    #[tracked]
    #[returns(ref)]
    pub types: Map<Identifier<'db>, Type>,
    
    /// Exported identifiers
    #[tracked]
    #[returns(ref)]
    pub exports: Vec<Identifier<'db>>,
}
```

### Function Definition

```rust
/// A function definition in the compiled module.
#[salsa::tracked]
pub struct FunctionDef<'db> {
    /// Function name
    pub name: Identifier<'db>,
    
    /// Parameter names
    #[tracked]
    #[returns(ref)]
    pub params: Vec<Identifier<'db>>,
    
    /// Function body AST
    #[tracked]
    #[returns(ref)]
    pub body: Expr,
    
    /// Source span for diagnostics
    #[tracked]
    pub span: Span,
}
```

## Tracked Functions (Queries)

### Parsing

```rust
/// Parse a source file into a CST.
/// 
/// This query is memoized - if the source text hasn't changed,
/// the cached ParsedFile is returned immediately.
#[salsa::tracked]
pub fn parse_file(db: &dyn CadenzaDb, source: SourceFile) -> ParsedFile<'_> {
    let text = source.text(db);
    let cst = cadenza_syntax::parse(text);
    
    // Count errors for quick checking
    let error_count = count_parse_errors(&cst);
    
    // Accumulate detailed diagnostics
    for error in cst.errors() {
        ParseDiagnostic {
            span: error.span(),
            message: error.message().to_string(),
            severity: DiagnosticLevel::Error,
        }
        .accumulate(db);
    }
    
    ParsedFile::new(db, source, cst, error_count)
}
```

### Evaluation

```rust
/// Evaluate a parsed file to produce a compiled module.
/// 
/// This query is memoized - only re-evaluates if the CST changed.
#[salsa::tracked]
pub fn evaluate_module(
    db: &dyn CadenzaDb,
    parsed: ParsedFile<'_>,
) -> CompiledModule<'_> {
    let cst = parsed.cst(db);
    
    // Build definitions by walking the CST
    let mut definitions = Map::default();
    let mut macros = Map::default();
    let mut types = Map::default();
    let mut exports = Vec::new();
    
    for item in cst.items() {
        match item {
            Item::Function(func) => {
                let name = Identifier::new(db, func.name().text());
                let value = evaluate_function(db, parsed, func);
                definitions.insert(name, value);
                exports.push(name);
            }
            Item::Macro(mac) => {
                let name = Identifier::new(db, mac.name().text());
                let value = evaluate_macro(db, parsed, mac);
                macros.insert(name, value);
            }
            // ... other items
        }
    }
    
    CompiledModule::new(
        db,
        parsed.source(db),
        definitions,
        macros,
        types,
        exports,
    )
}

/// Evaluate a specific function definition.
/// 
/// This is a separate query so that changing one function doesn't
/// require re-evaluating all functions in the module.
#[salsa::tracked]
pub fn evaluate_function(
    db: &dyn CadenzaDb,
    parsed: ParsedFile<'_>,
    func: FunctionNode,
) -> Value {
    // Evaluate function body
    // This is pure - no mutation
    // Returns Value
}
```

### Type Inference

```rust
/// Infer the type of a function.
/// 
/// This query is called on-demand (e.g., for LSP hover).
/// It only type-checks the specific function requested.
#[salsa::tracked]
pub fn infer_function_type(
    db: &dyn CadenzaDb,
    module: CompiledModule<'_>,
    func_name: Identifier<'_>,
) -> InferredType {
    // Get the function definition
    let func_def = module.definitions(db).get(&func_name)
        .expect("function not found");
    
    // Collect constraints
    let constraints = collect_constraints(db, module, func_def);
    
    // Solve constraints
    let solution = solve_constraints(db, constraints);
    
    // Return inferred type
    solution.get_type(func_name)
}

/// Get the type at a specific position in the source file.
/// 
/// This is the query used by LSP hover.
#[salsa::tracked]
pub fn type_at_position(
    db: &dyn CadenzaDb,
    source: SourceFile,
    line: u32,
    column: u32,
) -> Option<Type> {
    // Parse the file
    let parsed = parse_file(db, source);
    
    // Find the node at the position
    let node = find_node_at_position(parsed.cst(db), line, column)?;
    
    // Get the module
    let module = evaluate_module(db, parsed);
    
    // Infer type for the expression at this position
    infer_expression_type(db, module, node)
}
```

### Macro Expansion

```rust
/// Expand macros in a parsed file.
/// 
/// This is a separate query from evaluation to allow better
/// incrementality - if macro definitions change, we only need
/// to re-expand, not re-parse.
#[salsa::tracked]
pub fn expand_macros(
    db: &dyn CadenzaDb,
    parsed: ParsedFile<'_>,
) -> ExpandedModule<'_> {
    let cst = parsed.cst(db);
    let module = evaluate_module(db, parsed);
    
    // Walk CST and expand macro calls
    let expanded_cst = expand_all_macros(db, cst, module.macros(db));
    
    ExpandedModule::new(db, parsed, expanded_cst)
}

#[salsa::tracked]
pub struct ExpandedModule<'db> {
    pub parsed: ParsedFile<'db>,
    
    #[tracked]
    #[returns(ref)]
    pub cst: SyntaxNode,
}
```

## Accumulators (for Diagnostics)

### Parse Diagnostics

```rust
/// A diagnostic from the parser.
#[salsa::accumulator]
pub struct ParseDiagnostic {
    pub span: Span,
    pub message: String,
    pub severity: DiagnosticLevel,
}

impl ParseDiagnostic {
    pub fn error(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
            severity: DiagnosticLevel::Error,
        }
    }
    
    pub fn warning(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
            severity: DiagnosticLevel::Warning,
        }
    }
}

// Usage:
#[salsa::tracked]
fn parse_file(db: &dyn CadenzaDb, source: SourceFile) -> ParsedFile<'_> {
    // ... parsing ...
    
    // Accumulate a diagnostic
    ParseDiagnostic::error(
        span,
        "unexpected token",
    ).accumulate(db);
    
    // ... more parsing ...
}

// Query diagnostics:
fn get_all_diagnostics(db: &dyn CadenzaDb, source: SourceFile) -> Vec<ParseDiagnostic> {
    parse_file::accumulated::<ParseDiagnostic>(db, source)
}
```

### Type Errors

```rust
/// A type error from type inference.
#[salsa::accumulator]
pub struct TypeError {
    pub span: Span,
    pub message: String,
}

// Usage:
#[salsa::tracked]
fn infer_function_type(
    db: &dyn CadenzaDb,
    module: CompiledModule<'_>,
    func_name: Identifier<'_>,
) -> InferredType {
    // ... type inference ...
    
    if incompatible {
        TypeError {
            span,
            message: format!(
                "type mismatch: expected {}, got {}",
                expected, actual
            ),
        }
        .accumulate(db);
    }
    
    // ... more inference ...
}
```

### Evaluation Warnings

```rust
/// A warning from evaluation.
#[salsa::accumulator]
pub struct EvalWarning {
    pub span: Span,
    pub message: String,
}
```

## LSP Integration Example

### Hover Provider

```rust
impl LspDatabase {
    /// Handle hover request from LSP client.
    pub fn hover(&self, uri: &Url, position: Position) -> Option<Hover> {
        let db = self.db.lock();
        
        // Get the source file
        let file_map = self.file_map.read();
        let source = *file_map.get(uri)?;
        
        // Query the type at position (this is memoized!)
        let ty = type_at_position(
            &*db,
            source,
            position.line,
            position.character,
        )?;
        
        // Format the type for display
        Some(Hover {
            contents: HoverContents::Scalar(MarkedString::String(
                format!("{}", ty)
            )),
            range: None,
        })
    }
}
```

### Completion Provider

```rust
#[salsa::tracked]
pub fn completions_at_position(
    db: &dyn CadenzaDb,
    source: SourceFile,
    line: u32,
    column: u32,
) -> Vec<CompletionItem> {
    let parsed = parse_file(db, source);
    let module = evaluate_module(db, parsed);
    
    // Get visible identifiers at this position
    let scope = get_scope_at_position(db, parsed, line, column);
    
    let mut completions = Vec::new();
    
    // Add definitions from module
    for (name, _) in module.definitions(db).iter() {
        completions.push(CompletionItem {
            label: name.text(db).clone(),
            kind: Some(CompletionItemKind::FUNCTION),
            ..Default::default()
        });
    }
    
    // Add local variables from scope
    for (name, _) in scope.bindings.iter() {
        completions.push(CompletionItem {
            label: name.text(db).clone(),
            kind: Some(CompletionItemKind::VARIABLE),
            ..Default::default()
        });
    }
    
    completions
}

impl LspDatabase {
    pub fn completion(&self, uri: &Url, position: Position) -> Vec<CompletionItem> {
        let db = self.db.lock();
        let file_map = self.file_map.read();
        
        if let Some(&source) = file_map.get(uri) {
            completions_at_position(
                &*db,
                source,
                position.line,
                position.character,
            )
        } else {
            Vec::new()
        }
    }
}
```

### Diagnostics Provider

```rust
impl LspDatabase {
    /// Get all diagnostics for a file.
    pub fn diagnostics(&self, uri: &Url) -> Vec<lsp_types::Diagnostic> {
        let db = self.db.lock();
        let file_map = self.file_map.read();
        
        let source = match file_map.get(uri) {
            Some(&s) => s,
            None => return Vec::new(),
        };
        
        let mut diagnostics = Vec::new();
        
        // Collect parse diagnostics
        for diag in parse_file::accumulated::<ParseDiagnostic>(&*db, source) {
            diagnostics.push(to_lsp_diagnostic(diag));
        }
        
        // Collect type errors
        let parsed = parse_file(&*db, source);
        let module = evaluate_module(&*db, parsed);
        for (_name, _) in module.definitions(&*db).iter() {
            // Type check each function
            // (in reality, might want to batch this)
        }
        
        diagnostics
    }
}
```

## Example: End-to-End Flow

```rust
#[test]
fn test_incremental_recompilation() {
    // Create database
    let mut db = CadenzaDbImpl::new();
    
    // Create source file
    let source = SourceFile::new(
        &db,
        "test.cdz".to_string(),
        "fn add x y = x + y\nfn double x = add x x".to_string(),
    );
    
    // Parse the file (happens now)
    let parsed = parse_file(&db, source);
    assert_eq!(parsed.error_count(&db), 0);
    
    // Evaluate the module (happens now)
    let module = evaluate_module(&db, parsed);
    assert_eq!(module.definitions(&db).len(), 2);
    
    // Get type of 'add' function (happens now)
    let add_id = Identifier::new(&db, "add".to_string());
    let add_type = infer_function_type(&db, module, add_id);
    println!("Type of add: {}", add_type);
    
    // Now modify the source file
    source.set_text(&mut db).to(
        "fn add x y = x + y\nfn triple x = add x (add x x)".to_string()
    );
    
    // Re-parse (Salsa detects text changed, re-parses)
    let parsed2 = parse_file(&db, source);
    
    // Re-evaluate (Salsa detects CST changed, re-evaluates)
    let module2 = evaluate_module(&db, parsed2);
    
    // Type of 'add' is cached! Doesn't recompute.
    let add_type2 = infer_function_type(&db, module2, add_id);
    assert_eq!(add_type, add_type2);
    
    // Type of 'triple' is computed (new function)
    let triple_id = Identifier::new(&db, "triple".to_string());
    let triple_type = infer_function_type(&db, module2, triple_id);
    println!("Type of triple: {}", triple_type);
}
```

## Performance Characteristics

### What Gets Memoized

- **Parsing**: If source text unchanged, cached CST returned
- **Evaluation**: If CST unchanged, cached module returned
- **Type inference**: If function unchanged, cached type returned
- **LSP queries**: All queries are memoized

### What Triggers Recomputation

- Changing source text: Re-parses, re-evaluates, re-type-checks
- Adding new function: Only new function is evaluated/type-checked
- Changing one function: Only that function and dependents are re-checked

### Expected Performance Gains

- **Initial compilation**: Similar to current (no optimization, just tracking overhead)
- **Incremental recompilation**: 10-100x faster (only recompute what changed)
- **LSP queries**: 100-1000x faster (instant if cached, sub-millisecond if not)

## References

- [Salsa Book: Overview](https://salsa-rs.github.io/salsa/overview.html)
- [Salsa Calc Example](https://github.com/salsa-rs/salsa/tree/master/examples/calc)
- [rust-analyzer Architecture](https://github.com/rust-lang/rust-analyzer/blob/master/docs/dev/architecture.md)
