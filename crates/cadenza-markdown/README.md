# cadenza-markdown

Markdown parser as alternative Cadenza syntax.

This crate treats Markdown as an alternative lexer/parser for Cadenza, producing Cadenza-compatible AST directly that can be evaluated by `cadenza-eval`. Markdown documents become function calls to macros, with content and code blocks passed as arguments.

## Features

- **Direct AST Construction**: Builds `cadenza_syntax::ast::Root` from Markdown using Rowan CST
- **Zero String Generation**: No intermediate Cadenza code generation or re-parsing
- **Proper Offset Tracking**: Every byte accounted for with accurate source positions
- **Code Block Parameters**: Support for language specification and additional parameters
- **Multiple Markdown Elements**: Headings, paragraphs, lists, code blocks, inline code, links, emphasis
- **Flexible Code Block Handling**: Control execution, visibility, and output rendering via parameters

## Architecture

Markdown is parsed directly into Cadenza's AST format:
- **Headings** → Apply nodes (e.g., `h1 "Title"` becomes `[h1, "Title"]`)
- **Paragraphs** → Apply nodes (e.g., `p "Text content"`)
- **Code blocks** → Apply nodes with language and parameters (e.g., `code "cadenza" "let x = 1"` with optional params)
- **Lists** → Apply nodes (e.g., `ul [item1, item2]`)
- **Inline elements** → Nested apply nodes for emphasis, code, links

## Example

```rust
use cadenza_markdown::parse;
use cadenza_eval::{eval, BuiltinMacro, Compiler, Env, Type, Value};

let markdown = r#"# Hello World

This is a paragraph.

```cadenza
let x = 42
```
"#;

// Parse Markdown into Cadenza AST
let parse_result = parse(markdown);
let root = parse_result.ast();

let mut compiler = Compiler::new();
let mut env = Env::new();

// Register markdown macros
compiler.define_macro("h1".into(), Value::BuiltinMacro(BuiltinMacro {
    name: "h1",
    signature: Type::function(vec![Type::String], Type::Nil),
    func: |args, ctx| {
        // Handler receives heading text
        Ok(Value::Nil)
    },
}));

compiler.define_macro("p".into(), Value::BuiltinMacro(BuiltinMacro {
    name: "p",
    signature: Type::function(vec![Type::String], Type::Nil),
    func: |args, ctx| {
        // Handler receives paragraph text
        Ok(Value::Nil)
    },
}));

compiler.define_macro("code".into(), Value::BuiltinMacro(BuiltinMacro {
    name: "code",
    signature: Type::function(vec![Type::String, Type::String], Type::Nil),
    func: |args, ctx| {
        // Handler receives language and code content
        // args[0] = language (e.g., "cadenza")
        // args[1] = code content
        Ok(Value::Nil)
    },
}));

// Evaluate - eval doesn't care this came from Markdown!
let results = eval(&root, &mut env, &mut compiler);
```

### Input and Output

**Markdown Input:**
```markdown
# Physics Tutorial

The range of a projectile is calculated as:

```cadenza
let velocity = 20
let angle = 45
```

You can experiment with different values!
```

Parsed AST:
```
[h1, "Physics Tutorial"]
[p, "The range of a projectile is calculated as:"]
[code, "cadenza", "let velocity = 20\nlet angle = 45"]
[p, "You can experiment with different values!"]
```

**Code Block with Parameters:**
```markdown
```cadenza editable hidden
let setup = initialize()
```
```

Parsed AST:
```
[code, ["cadenza", "editable", "hidden"], "let setup = initialize()"]
```

Handler macros receive the markdown content and can:
- Render content appropriately for the output format
- Execute code blocks in sequence maintaining state
- Apply parameters to control code block behavior (editable, hidden, output format)
- Handle inline elements like emphasis and links
- Build interactive educational content

## Benefits

1. **Simpler Architecture**: Direct AST construction, no string manipulation
2. **Flexible Semantics**: Handler macros control all content interpretation
3. **Natural Integration**: Full access to Cadenza's type system and dimensional analysis
4. **Better Errors**: Stack traces point to original Markdown source locations
5. **Interactive Content**: Enable executable documentation and interactive books

## Vision

This is the first step toward using Cadenza for interactive educational content where code examples are executable and modifiable inline. See `docs/VISUAL_ART_AND_INTERACTIVE_BOOKS.md` for the full vision.
