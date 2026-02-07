# Example: Converting Cadenza to K-parseable AST

This example shows the complete flow from Cadenza source to K-parseable AST.

## Input: Cadenza Source (example.cdz)

```cadenza
42
3.14
"hello world"
[add, 1, 2]
```

## Command

```bash
cargo run --bin cadenza ast example.cdz
```

## Output: S-expression AST

```
(Int "42")
(Float "3.14")
(String "hello world")
(Apply (Synthetic "__list__") (Ident "add") (Int "1") (Int "2"))
```

## What K Framework Sees

K can now parse this unambiguously:
- `(Int "42")` matches the `Int(String)` syntax rule
- `(Float "3.14")` matches the `Float(String)` syntax rule  
- `(String "hello world")` matches the `String(String)` syntax rule
- `(Apply ...)` matches the `Apply(Expr, Exprs)` syntax rule

## Why This Matters

1. **No Parser Ambiguity**: K doesn't support Pratt parsing, so we pre-parse to AST
2. **Clean Separation**: Parsing logic stays in Rust, semantics in K
3. **Testable**: We can validate AST output independently
4. **Composable**: Other tools can use the AST format

## Complex Example: Let Binding

### Input

```cadenza
let x = 42
x
```

### AST Output

```
(Apply (Op "=") (Apply (Ident "let") (Ident "x")) (Int "42"))
(Ident "x")
```

### What This Shows

The parser correctly handles:
- Operators (`=`) as operator nodes
- Special forms (`let`) as identifiers in applications
- Nesting of applications

## Next Steps

Once K framework is installed:

```bash
cd crates/cadenza-k
make kompile          # Compile K definition
make run FILE=example.ast  # Run through K interpreter
```

This will evaluate the AST according to the semantic rules in `cadenza.k`.
