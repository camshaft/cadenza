# Salsa Quick Reference for Cadenza Developers

This is a quick reference guide for developers working with Salsa in Cadenza after the migration is complete.

## Core Concepts

### Database

The database is the central hub for all Salsa queries. Think of it as a memoization cache + dependency tracker.

```rust
// Get database reference
fn my_function(db: &dyn CadenzaDb) {
    // Use db to call queries
}

// Mutable database for changing inputs
fn my_function(db: &mut dyn CadenzaDb) {
    // Use db to modify inputs
}
```

### Inputs

Inputs are data that can change from outside. When an input changes, dependent queries are invalidated.

```rust
#[salsa::input]
pub struct SourceFile {
    #[returns(ref)]
    pub text: String,
}

// Create
let source = SourceFile::new(db, "example.cdz".to_string(), "let x = 1".to_string());

// Read
let text: &String = source.text(db);

// Update (requires &mut db)
source.set_text(&mut db).to("let x = 2".to_string());
```

### Tracked Functions (Queries)

Tracked functions are pure functions that Salsa memoizes. If inputs haven't changed, the cached result is returned.

```rust
#[salsa::tracked]
pub fn parse_file(db: &dyn CadenzaDb, source: SourceFile) -> ParsedFile<'_> {
    // This only runs if source.text() changed
    let text = source.text(db);
    // ... parse ...
}

// Call it like a normal function
let parsed = parse_file(db, source);
```

### Interned Types

Interned types are deduplicated immutable values. Multiple instances with the same data share storage.

```rust
#[salsa::interned]
pub struct Identifier<'db> {
    #[returns(ref)]
    pub text: String,
}

// These are the same object internally
let id1 = Identifier::new(db, "foo".to_string());
let id2 = Identifier::new(db, "foo".to_string());
assert_eq!(id1, id2); // Very fast comparison
```

### Tracked Structs

Tracked structs are created during computation and tracked by Salsa.

```rust
#[salsa::tracked]
pub struct ParsedFile<'db> {
    pub source: SourceFile,
    
    #[tracked]
    #[returns(ref)]
    pub cst: SyntaxNode,
}

// Create
let parsed = ParsedFile::new(db, source, cst);

// Read fields
let cst = parsed.cst(db);
```

### Accumulators

Accumulators collect values during computation (e.g., diagnostics, warnings).

```rust
#[salsa::accumulator]
pub struct Diagnostic {
    pub message: String,
}

// Inside a tracked function, accumulate values
#[salsa::tracked]
fn parse_file(db: &dyn CadenzaDb, source: SourceFile) -> ParsedFile<'_> {
    // ... parsing ...
    Diagnostic { message: "error".to_string() }.accumulate(db);
    // ...
}

// Query accumulated values
let diagnostics = parse_file::accumulated::<Diagnostic>(db, source);
```

## Common Patterns

### Pattern: Input File

```rust
#[salsa::input]
pub struct SourceFile {
    #[returns(ref)]
    pub path: String,
    
    #[returns(ref)]
    pub text: String,
}
```

**When to use**: For data that comes from outside (user input, files on disk, etc.)

### Pattern: Interned Identifier

```rust
#[salsa::interned]
pub struct Identifier<'db> {
    #[returns(ref)]
    pub text: String,
}
```

**When to use**: For values that are compared frequently and should be deduplicated (names, paths, etc.)

### Pattern: Parsing

```rust
#[salsa::tracked]
pub fn parse_file(db: &dyn CadenzaDb, source: SourceFile) -> ParsedFile<'_> {
    let text = source.text(db);
    let cst = cadenza_syntax::parse(text);
    ParsedFile::new(db, source, cst)
}
```

**When to use**: For converting inputs into structured data

### Pattern: Error Collection

```rust
#[salsa::accumulator]
pub struct ParseError {
    pub span: Span,
    pub message: String,
}

#[salsa::tracked]
pub fn parse_file(db: &dyn CadenzaDb, source: SourceFile) -> ParsedFile<'_> {
    // ... parsing ...
    if error {
        ParseError { span, message }.accumulate(db);
    }
    // ...
}
```

**When to use**: For collecting errors/warnings/hints during computation

### Pattern: Fine-Grained Query

```rust
#[salsa::tracked]
pub fn type_at_position(
    db: &dyn CadenzaDb,
    source: SourceFile,
    line: u32,
    column: u32,
) -> Option<Type> {
    let parsed = parse_file(db, source);
    // ... find type at position ...
}
```

**When to use**: For LSP queries that need specific information without computing everything

## Common Mistakes

### ❌ Mistake: Mutating in tracked function

```rust
#[salsa::tracked]
pub fn parse_file(db: &dyn CadenzaDb, source: SourceFile) -> ParsedFile<'_> {
    let mut state = State::new(); // ❌ Mutable state
    state.parse(); // ❌ Mutation
    // This breaks memoization!
}
```

**Fix**: Make the function pure. Return new values instead of mutating.

### ❌ Mistake: Forgetting #[returns(ref)]

```rust
#[salsa::input]
pub struct SourceFile {
    pub text: String, // ❌ Will clone every time you access it
}
```

**Fix**: Add `#[returns(ref)]` for large values:

```rust
#[salsa::input]
pub struct SourceFile {
    #[returns(ref)]
    pub text: String, // ✅ Returns &String
}
```

### ❌ Mistake: Too fine-grained queries

```rust
#[salsa::tracked]
pub fn parse_token_0(db: &dyn CadenzaDb, source: SourceFile) -> Token { /* ... */ }

#[salsa::tracked]
pub fn parse_token_1(db: &dyn CadenzaDb, source: SourceFile) -> Token { /* ... */ }
// ... 1000 more token queries
```

**Fix**: Group related computations:

```rust
#[salsa::tracked]
pub fn parse_file(db: &dyn CadenzaDb, source: SourceFile) -> ParsedFile<'_> {
    // Parse all tokens at once
}
```

### ❌ Mistake: Too coarse-grained queries

```rust
#[salsa::tracked]
pub fn compile_everything(db: &dyn CadenzaDb) -> CompiledProgram {
    // Compiles the entire program in one go
    // Can't reuse work when one file changes
}
```

**Fix**: Break into per-file or per-function queries:

```rust
#[salsa::tracked]
pub fn compile_file(db: &dyn CadenzaDb, source: SourceFile) -> CompiledFile<'_> {
    // Compile one file
}
```

## Performance Tips

### Tip 1: Query Granularity

Find the right balance:
- **Too fine**: Query overhead dominates
- **Too coarse**: Poor incrementality
- **Just right**: Matches natural units of work (files, functions, modules)

### Tip 2: Use #[returns(ref)]

For large values, return references from the database instead of cloning:

```rust
#[salsa::tracked]
pub struct ParsedFile<'db> {
    #[tracked]
    #[returns(ref)]  // ✅ Returns &Vec instead of cloning
    pub tokens: Vec<Token>,
}
```

### Tip 3: Durability

Mark rarely-changing inputs with high durability:

```rust
source.set_text(&mut db)
    .with_durability(salsa::Durability::HIGH)
    .to(text);
```

This tells Salsa that this input rarely changes, optimizing query invalidation.

### Tip 4: Lazy Queries

Structure queries so that work is only done when needed:

```rust
// ✅ Good: Only type-checks if type is requested
#[salsa::tracked]
pub fn function_type(db: &dyn CadenzaDb, func: Function<'_>) -> Type {
    // Type checking happens here
}

// ❌ Bad: Type-checks everything upfront
#[salsa::tracked]
pub fn compile_module(db: &dyn CadenzaDb, source: SourceFile) -> CompiledModule<'_> {
    for func in functions {
        function_type(db, func); // Types everything
    }
}
```

## Debugging

### Enable Salsa Logging

```rust
let db = CadenzaDbImpl {
    storage: salsa::Storage::new(Some(Box::new(|event| {
        eprintln!("Salsa event: {:?}", event);
    }))),
};
```

This logs all Salsa events (query executions, cache hits, invalidations, etc.)

### Check What Changed

```rust
// In tests, track which queries executed
let mut logs = Vec::new();
let db = CadenzaDbImpl::with_logging(&mut logs);

// ... do stuff ...

// See what queries ran
for log in logs {
    println!("{}", log);
}
```

### Visualize Dependencies

Salsa can show you the dependency graph between queries (useful for understanding why something recomputed).

## Testing

### Basic Test Pattern

```rust
#[test]
fn test_incremental() {
    let mut db = CadenzaDbImpl::new();
    
    // Create input
    let source = SourceFile::new(&db, "test.cdz".to_string(), "let x = 1".to_string());
    
    // Run query
    let result1 = my_query(&db, source);
    
    // Modify input
    source.set_text(&mut db).to("let x = 2".to_string());
    
    // Run query again (should recompute)
    let result2 = my_query(&db, source);
    
    assert_ne!(result1, result2);
}
```

### Testing Memoization

```rust
#[test]
fn test_memoization() {
    let db = CadenzaDbImpl::with_logging();
    let source = SourceFile::new(&db, "test.cdz".to_string(), "let x = 1".to_string());
    
    db.enable_logging();
    
    // First call - executes
    let _result1 = my_query(&db, source);
    let logs1 = db.take_logs();
    assert!(logs1.iter().any(|log| log.contains("WillExecute")));
    
    // Second call - cached
    let _result2 = my_query(&db, source);
    let logs2 = db.take_logs();
    assert!(!logs2.iter().any(|log| log.contains("WillExecute")));
}
```

### Testing Incrementality

```rust
#[test]
fn test_incrementality() {
    let mut db = CadenzaDbImpl::with_logging();
    
    let source = SourceFile::new(&db, "test.cdz".to_string(), 
        "fn foo x = x\nfn bar y = y".to_string());
    
    // Type check both functions
    let module = evaluate_module(&db, parse_file(&db, source));
    let foo_id = Identifier::new(&db, "foo".to_string());
    let bar_id = Identifier::new(&db, "bar".to_string());
    
    db.enable_logging();
    let _foo_type = function_type(&db, module, foo_id);
    let _bar_type = function_type(&db, module, bar_id);
    db.take_logs(); // Clear
    
    // Change only foo
    source.set_text(&mut db).to("fn foo x = x + 1\nfn bar y = y".to_string());
    
    db.enable_logging();
    let module2 = evaluate_module(&db, parse_file(&db, source));
    let _foo_type2 = function_type(&db, module2, foo_id);
    let _bar_type2 = function_type(&db, module2, bar_id);
    
    let logs = db.take_logs();
    // foo should recompute, bar should be cached
    assert!(logs.iter().any(|log| log.contains("foo")));
    assert!(!logs.iter().any(|log| log.contains("bar")));
}
```

## LSP Integration

### Pattern: File Tracking

```rust
pub struct LspDatabase {
    db: Mutex<CadenzaDbImpl>,
    files: RwLock<HashMap<Url, SourceFile>>,
}

impl LspDatabase {
    pub fn did_open(&self, uri: Url, text: String) {
        let mut db = self.db.lock();
        let source = SourceFile::new(&*db, uri.to_string(), text);
        self.files.write().insert(uri, source);
    }
    
    pub fn did_change(&self, uri: &Url, text: String) {
        if let Some(&source) = self.files.read().get(uri) {
            let mut db = self.db.lock();
            source.set_text(&mut *db).to(text);
        }
    }
}
```

### Pattern: Query Handler

```rust
impl LspDatabase {
    pub fn hover(&self, uri: &Url, position: Position) -> Option<Hover> {
        let db = self.db.lock();
        let source = *self.files.read().get(uri)?;
        
        let ty = type_at_position(&*db, source, position.line, position.character)?;
        
        Some(Hover {
            contents: HoverContents::Scalar(
                MarkedString::String(format!("{}", ty))
            ),
            range: None,
        })
    }
}
```

## Common Questions

### Q: When should I use input vs tracked struct?

**A**: Use `#[salsa::input]` for data from outside (user input, files). Use `#[salsa::tracked]` for data computed within your queries.

### Q: When should I use interned vs tracked struct?

**A**: Use `#[salsa::interned]` for small values compared frequently (names, IDs). Use `#[salsa::tracked]` for larger computed results.

### Q: How do I handle mutable algorithms?

**A**: Convert to functional style. Instead of mutating a value, return a new value. Use persistent data structures if needed.

### Q: What if my query has side effects?

**A**: Tracked functions must be pure (no side effects). Use accumulators for collecting diagnostics. For real side effects (writing files), do them outside of queries.

### Q: How do I debug "query was called while still executing"?

**A**: This means you have a cycle in your queries (A calls B calls A). Restructure to break the cycle, or use `#[salsa::cycle]` to handle it explicitly.

## Further Reading

- [Salsa Book](https://salsa-rs.github.io/salsa)
- [Salsa GitHub](https://github.com/salsa-rs/salsa)
- [Salsa Calc Example](https://github.com/salsa-rs/salsa/tree/master/examples/calc)
- [rust-analyzer Architecture](https://github.com/rust-lang/rust-analyzer/blob/master/docs/dev/architecture.md) (uses Salsa extensively)
- Cadenza docs:
  - [SALSA_MIGRATION_PLAN.md](SALSA_MIGRATION_PLAN.md)
  - [SALSA_ARCHITECTURE_EXAMPLE.md](SALSA_ARCHITECTURE_EXAMPLE.md)
