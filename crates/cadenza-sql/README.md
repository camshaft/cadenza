# cadenza-sql

SQL parser as alternative Cadenza syntax.

This crate treats SQL as an alternative lexer/parser for Cadenza, producing Cadenza-compatible AST directly that can be evaluated by `cadenza-eval`. SQL statements become function calls, allowing for flexible interpretation and execution.

## Features

- **Direct AST Construction**: Builds `cadenza_syntax::ast::Root` from SQL using Rowan CST
- **Zero String Generation**: No intermediate Cadenza code generation or re-parsing
- **Proper Offset Tracking**: Every byte accounted for with accurate source positions
- **Comment Preservation**: Comments are included in the CST as trivia
  - Line comments: `-- comment text`
  - Block comments: `/* comment text */`
- **Common SQL Statements**: Support for SELECT, INSERT, UPDATE, DELETE, CREATE, DROP, ALTER
- **SQL Clauses**: WHERE, ORDER BY, LIMIT, and other common clauses

## Architecture

SQL is parsed directly into Cadenza's AST format:
- **SQL statements** → Apply nodes (e.g., `SELECT` becomes a function call)
- **Clauses** → Arguments to the statement (e.g., `WHERE`, `FROM`)
- **Expressions** → Nested Apply nodes or literals
- **Comments** → Comment tokens in CST

## Example

```rust
use cadenza_sql::parse;
use cadenza_eval::{eval, BuiltinMacro, Compiler, Env, Type, Value};

let sql = "SELECT * FROM users WHERE age > 18";

// Parse SQL into Cadenza AST
let parse_result = parse(sql);
let root = parse_result.ast();

let mut compiler = Compiler::new();
let mut env = Env::new();

// Register SELECT macro
compiler.define_macro("SELECT".into(), Value::BuiltinMacro(BuiltinMacro {
    name: "SELECT",
    signature: Type::function(vec![Type::Unknown], Type::Nil),
    func: |args, ctx| {
        // Handler receives SQL clauses as arguments
        // args[0] = column list
        // args[1] = FROM keyword
        // args[2] = table name
        // args[3] = WHERE keyword
        // args[4] = condition expression
        Ok(Value::Nil)
    },
}));

// Evaluate - eval doesn't care this came from SQL!
let results = eval(&root, &mut env, &mut compiler);
```

### Input and Output

**SQL Input:**
```sql
SELECT id, name FROM users WHERE age > 18
```

Parsed AST:
```
[SELECT, [id, name], FROM, users, WHERE, [>, age, 18]]
```

**With Comments:**
```sql
-- Get adult users
SELECT * FROM users WHERE age > 18;

/* Multi-line
   comment */
INSERT INTO logs (message) VALUES ('test');
```

Comments are preserved in the CST as trivia tokens.

Handler macros receive SQL clauses and can:
- Execute queries against a database
- Transform SQL to other query languages
- Validate SQL syntax and semantics
- Implement custom SQL dialects
- Build query visualization tools

## Benefits

1. **Simpler Architecture**: Direct AST construction, no string manipulation
2. **Flexible Semantics**: Handler macros control all SQL interpretation
3. **Natural Integration**: Full access to Cadenza's type system and dimensional analysis
4. **Better Errors**: Stack traces point to original SQL source locations
5. **Educational Tools**: Build interactive SQL tutorials and visualizations

## Vision

This is the first step toward using Cadenza for interactive educational content where SQL queries are executable and modifiable inline, similar to the markdown and gcode parsers. This enables interactive database tutorials and query visualization tools.
