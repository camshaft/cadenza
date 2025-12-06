# cadenza-markdown Status

## Overview

Markdown parser as an alternative syntax frontend for Cadenza, similar to the gcode parser. Converts Markdown documents into Cadenza AST that can be evaluated using macros.

## Implementation Status

### ✅ Completed Features

#### Core Parsing
- **Headings (h1-h6)**: Parse `#` through `######` syntax
  - AST representation: `[h1, "content"]`, `[h2, "content"]`, etc.
  - Synthetic tokens provide clean identifiers for macros
  
- **Paragraphs**: Plain text content wrapped in macro calls  
  - AST representation: `[p, "content"]`
  - Paragraphs separated by blank lines
  
- **Unordered Lists**: Items prefixed with `-` or `*`
  - AST representation: `[ul, "item1", "item2", "item3"]`
  - Synthetic token `ul` as the function identifier
  
#### Code Blocks
- **Code Blocks**: Fenced with ` ``` ` or `~~~`
  - Cadenza blocks: `[code, "cadenza", [__block__, [parsed, ast], ...]]` (fully parsed into AST)
  - Other languages: `[code, "language", "code content"]` (preserved as string)
  - Language identifier extracted from fence line
  - **Parameters**: Additional space-separated tokens on fence line are passed as extra arguments
    - Example: ` ```cadenza editable hidden` produces `[code, "cadenza", [...], "editable", "hidden"]`
    - Parameters can be used by macro handlers to control behavior (visibility, editability, etc.)

#### Infrastructure
- ✅ Build system with snapshot test generation
- ✅ Test data files covering different markdown features
- ✅ CST and AST snapshot tests
- ✅ Integration with cadenza-eval

#### Inline Elements
- **Emphasis**: `*italic*` syntax
  - AST representation: `[em, "content"]`
  - Synthetic token `em` as the function identifier
  
- **Strong**: `**bold**` syntax
  - AST representation: `[strong, "content"]`
  - Synthetic token `strong` as the function identifier
  
- **Inline Code**: `` `code` `` syntax
  - AST representation: `[code_inline, "content"]`
  - Synthetic token `code_inline` as the function identifier
  - Takes precedence over emphasis to prevent parsing inside code spans
  
- **Mixed Inline Elements**: Multiple inline elements in a single paragraph
  - AST representation: `[p, [__list__, "text", [em, "italic"], " more text", [strong, "bold"]]]`
  - Content with inline elements is wrapped in a list structure
  - Plain text without inline elements remains as simple string: `[p, "plain text"]`

- **Nested Inline Elements**: Inline elements can be nested within each other
  - Example: `**bold with `code` inside**` produces `[strong, [__list__, "bold with ", [code_inline, "code"], " inside"]]`
  - Example: `**bold with *italic* inside**` produces `[strong, [__list__, "bold with ", [em, "italic"], " inside"]]`
  - Supports arbitrary nesting depth (e.g., bold → italic → code)
  - Inline code content is always literal (emphasis markers inside code are not interpreted)
  - The parser recursively processes content within emphasis/strong spans

### Implementation Approach

The markdown parser follows a different strategy than typical Markdown parsers:

1. **Direct AST Construction**: Markdown syntax is parsed directly into Cadenza's AST format using Rowan CST
2. **Synthetic Tokens as Identifiers**: Markdown elements use synthetic tokens:
   - Headings use `h1` through `h6` (not `#` symbols)
   - Paragraphs use `p`
   - Lists use `ul`
   - Code blocks use `code`
   - Inline emphasis uses `em`, `strong`, and `code_inline`
3. **Macro-Based Evaluation**: Handler macros registered in the eval context process markdown elements
4. **Zero String Generation**: No intermediate Cadenza code generation
5. **Parsed Cadenza Blocks**: Code blocks with language "cadenza" or empty are fully parsed into Cadenza AST
6. **Inline Element Precedence**: Inline code (backticks) is parsed first, preventing emphasis markers inside code from being interpreted
7. **Recursive Inline Parsing**: Inline elements support arbitrary nesting through recursive parsing

### 🚧 Partial/Limited Features

None currently.

### ❌ Not Yet Implemented

#### Inline Elements
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

### Why Use Synthetic Tokens as Identifiers?

Instead of translating `#` to `h1` or using `#` directly, we use synthetic tokens. This approach:
- Provides clean, semantic identifiers (h1-h6, p, ul, code) for macro definitions
- Keeps all source bytes accounted for in the CST (markdown syntax emitted as trivia)
- Allows macro handlers to work with intuitive function names
- Makes it easy to define macros in Cadenza code
- Follows best practices for first-class language integration

### CST Coverage for Embedded Cadenza Blocks

When Cadenza code blocks are parsed, the Cadenza AST is embedded into the markdown CST. This creates a challenge: Rowan's GreenNodeBuilder calculates token positions automatically based on sequential ordering and cumulative lengths, but embedded Cadenza tokens have their own position space.

Current status: CST coverage validation temporarily skipped for files with Cadenza code blocks. A proper solution requires either:
1. Adding offset support to cadenza-syntax parser (architectural change)
2. Implementing position remapping when copying Cadenza tokens
3. Accepting that embedded parsed blocks have independent position spaces

This doesn't affect AST correctness or evaluation - it's purely a source mapping concern for tooling (LSP, formatters).

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
- `code-block-many-params.md`: Code fences with multiple parameters
- `complex.md`: Multiple element types combined
- `inline-code.md`: Inline code spans with backticks
- `inline-emphasis.md`: Inline emphasis (italic and bold)
- `inline-mixed.md`: Multiple inline elements in paragraphs
- `inline-nested.md`: Nested inline elements (code inside emphasis, etc.)
- `inline-deeply-nested.md`: Deep nesting of inline elements

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
compiler.define_macro("h1".into(), Value::BuiltinMacro(BuiltinMacro {
    name: "h1",
    signature: Type::function(vec![Type::String], Type::Nil),
    func: |args, ctx| {
        // Handle heading content
        Ok(Value::Nil)
    },
}));

compiler.define_macro("code".into(), Value::BuiltinMacro(BuiltinMacro {
    name: "code",
    signature: Type::function(vec![Type::String, Type::Any], Type::Nil),
    func: |args, ctx| {
        // args[0] = language (e.g., "cadenza")
        // args[1] = code content (parsed AST block for Cadenza, string for others)
        // args[2..] = optional parameters (e.g., "editable", "hidden")
        Ok(Value::Nil)
    },
}));

let results = eval(&root, &mut env, &mut compiler);
```

## Future Work

### High Priority
- [x] Inline emphasis and code support (completed)
- [x] Code block parameter passing (completed)
- [x] Nested inline elements (completed)
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
