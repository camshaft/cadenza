# cadenza-markdown Status

## Overview

Markdown parser as an alternative syntax frontend for Cadenza, similar to the gcode parser. Converts Markdown documents into Cadenza AST that can be evaluated using macros.

## Implementation Status

### ✅ Completed Features

#### Core Parsing
- **Headings (h1-h6)**: Parse `#` through `######` syntax
  - AST representation: `[#, "content"]`, `[##, "content"]`, etc.
  - Hash count indicates heading level
  
- **Paragraphs**: Plain text content
  - AST representation: Just the string content directly (no wrapper)
  - Paragraphs separated by blank lines
  
- **Unordered Lists**: Items prefixed with `-` or `*`
  - AST representation: `[-, "item1", "item2", "item3"]`
  - First list marker becomes the function identifier
  
- **Code Blocks**: Fenced with ` ``` ` or `~~~`
  - AST representation: `[```, "language", "code content"]`
  - Language identifier extracted from fence line
  - Code content preserved exactly as written

#### Infrastructure
- ✅ Build system with snapshot test generation
- ✅ Test data files covering different markdown features
- ✅ CST and AST snapshot tests
- ✅ Integration with cadenza-eval

### Implementation Approach

The markdown parser follows a different strategy than typical Markdown parsers:

1. **Direct AST Construction**: Markdown syntax is parsed directly into Cadenza's AST format using Rowan CST
2. **Syntax as Identifiers**: Markdown syntax becomes function identifiers:
   - `#` becomes a function that receives heading content
   - ` ``` ` becomes a function that receives language and code
   - `-` becomes a function that receives list items
3. **Macro-Based Evaluation**: Handler macros registered in the eval context process markdown elements
4. **Zero String Generation**: No intermediate Cadenza code generation

### 🚧 Partial/Limited Features

- **Code Block Parameters**: Fence lines like ` ```cadenza editable hidden` parse the language but ignore extra parameters
  - TODO: Support passing parameters as additional arguments or metadata

### ❌ Not Yet Implemented

#### Inline Elements
- **Emphasis**: `*italic*` and `**bold**` 
- **Code**: `` `inline code` ``
- **Links**: `[text](url)`
- **Images**: `![alt](url)`

#### Block Elements
- **Ordered Lists**: `1. item`
- **Blockquotes**: `> quote`
- **Horizontal Rules**: `---` or `***`
- **Tables**: GitHub-style tables

#### Advanced Features
- **HTML pass-through**: Raw HTML in markdown
- **Task lists**: `- [ ]` and `- [x]`
- **Footnotes**: `[^1]` syntax
- **Definition lists**

## Design Decisions

### Why Use Markdown Syntax as Identifiers?

Instead of translating `#` to `h1`, we use `#` directly as the function identifier. This approach:
- Keeps all source bytes accounted for in the CST
- Preserves the original markdown syntax
- Allows macro handlers to interpret the syntax flexibly
- Follows the same pattern as the gcode parser

### Why Skip CST Coverage Validation?

Unlike gcode where every source byte directly corresponds to a token, markdown involves:
- Content transformation (extracting text from between markers)
- Synthetic structure (implicit paragraph boundaries)
- Whitespace handling (blank lines as separators)

CST coverage validation is not meaningful for this transformation. The AST tests verify correct parsing.

### Content Handling

Markdown content (paragraph text, code blocks, heading text) is emitted as `StringContent` tokens. These tokens reference source slices directly to maintain proper byte-level tracking.

## Testing

### Test Files

Located in `test-data/`:
- `simple.md`: Basic heading and paragraph
- `headings.md`: All heading levels h1-h6
- `lists.md`: Unordered list with multiple items
- `code-blocks.md`: Code fence with language specification
- `code-block-params.md`: Code fences with language and parameters
- `complex.md`: Multiple element types combined

### Snapshot Tests

Each test file generates:
- **CST snapshot**: Concrete syntax tree showing all tokens
- **AST snapshot**: Abstract syntax tree showing structure

Snapshots are automatically updated with `INSTA_UPDATE=always cargo test`.

## Usage Example

```rust
use cadenza_markdown::parse;
use cadenza_eval::{eval, BuiltinMacro, Compiler, Env, Type, Value};

let markdown = r#"# Hello World

This is a paragraph.

```cadenza
let x = 42
```
"#;

let parse_result = parse(markdown);
let root = parse_result.ast();

let mut compiler = Compiler::new();
let mut env = Env::new();

// Register markdown element macros
compiler.define_macro("#".into(), Value::BuiltinMacro(BuiltinMacro {
    name: "#",
    signature: Type::function(vec![Type::String], Type::Nil),
    func: |args, ctx| {
        // Handle heading content
        Ok(Value::Nil)
    },
}));

compiler.define_macro("```".into(), Value::BuiltinMacro(BuiltinMacro {
    name: "```",
    signature: Type::function(vec![Type::String, Type::String], Type::Nil),
    func: |args, ctx| {
        // args[0] = language (e.g., "cadenza")
        // args[1] = code content
        Ok(Value::Nil)
    },
}));

let results = eval(&root, &mut env, &mut compiler);
```

## Future Work

### High Priority
- [ ] Inline emphasis and code support
- [ ] Code block parameter passing
- [ ] Nested list support

### Medium Priority
- [ ] Ordered lists
- [ ] Blockquotes
- [ ] Links and images
- [ ] Tables

### Low Priority
- [ ] HTML pass-through
- [ ] Extended markdown features
- [ ] Custom directives

## Relationship to Documentation Vision

This implementation is the first step toward the vision outlined in `docs/VISUAL_ART_AND_INTERACTIVE_BOOKS.md`:

- ✅ Markdown front-end syntax parsing
- ✅ Macro-based processing in eval context
- ✅ Mode switching for code blocks (language specification)
- 🚧 Code fence arguments for control (partially implemented)
- ❌ Interactive elements (future work)
- ❌ Math notation support (future work)

The current implementation provides the foundation for executable documents where code blocks can be evaluated sequentially, with state carrying forward between blocks.
