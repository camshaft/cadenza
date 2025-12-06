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

Markdown is parsed directly into Cadenza's AST format using synthetic tokens:
- **Headings** → Apply nodes with synthetic tokens (e.g., `[h1, "Title"]`, `[h2, "Subtitle"]`)
- **Paragraphs** → Apply nodes (e.g., `[p, "This is text"]`)
- **Code blocks** → Apply nodes (e.g., `[code, "cadenza", [__block__, [let, x, 1]]]`)
  - Cadenza code blocks are fully parsed into AST
  - Non-Cadenza blocks remain as strings
- **Lists** → Apply nodes (e.g., `[ul, "item1", "item2", "item3"]`)
- **Inline elements** → Not yet implemented

Synthetic tokens (SyntheticMarkdownH1-H6, SyntheticMarkdownParagraph, SyntheticMarkdownList, SyntheticMarkdownCode) provide clean identifiers (h1-h6, p, ul, code) for macro handlers. The actual markdown syntax (# symbols, backticks, list markers) is emitted as trivia in the CST.

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
    signature: Type::function(vec![Type::String, Type::Any], Type::Nil),
    func: |args, ctx| {
        // Handler receives language and either:
        // - A parsed Cadenza AST block (for cadenza language)
        // - A string (for other languages)
        // args[0] = language (e.g., "cadenza", "javascript")
        // args[1] = code content (parsed AST or string)
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
[code, "cadenza", [__block__, [measure, meter], [measure, degree], [=, [let, velocity], 20], [=, [let, angle], 45]]]
[p, "You can experiment with different values!"]
```

Note how Cadenza code is fully parsed into AST with a __block__ wrapper, allowing macros to work with structured code rather than strings.

**Code Block with Parameters:**
```markdown
```cadenza editable hidden
let setup = initialize()
```
```

Parsed AST:
```
[code, "cadenza", [__block__, [=, [let, setup], [initialize]]]]
```

Note: Parameters like "editable" and "hidden" are currently not captured in the AST (future enhancement).

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
