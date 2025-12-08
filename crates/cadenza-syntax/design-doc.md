# Cadenza Syntax Design

## Problem Statement

Cadenza needs a syntax parser that supports:

- **Minimal CST/AST**: Simple enough for runtime metaprogramming
- **Whitespace significance**: Indentation-based structure (no braces)
- **Interactive compilation**: Blur lines between compiler and LSP
- **No keywords**: Everything is function application
- **Incremental evaluation**: Top-level items evaluate themselves

## Goals

1. **Minimal node types** - Make tree structure simple enough for runtime manipulation
2. **Keyword-less** - All constructs (`let`, `fn`, `measure`) are just function applications
3. **Whitespace-significant** - Use indentation for blocks
4. **Metaprogramming-friendly** - Quote/unquote for syntax manipulation
5. **LSP-ready** - Preserve all whitespace and comments

## Non-Goals

- Complex type annotations at parse time (handled by semantic analysis)
- Operator precedence in parser (operators are just functions)
- Special syntax forms (everything uniformly applies)

## Design

### Node Types

The language has only 6 node types:

```rust
pub enum SyntaxKind {
    Root,           // Top-level document
    Apply,          // Function application (f x or f(x))
    Attr,           // @identifier args...
    Literal,        // Integer, Float, String
    Receiver,       // The function being called in an Apply
    Error,          // Parse error recovery
}
```

The `Apply` node has the following structure:

```
Apply {
    receiver: Receiver,  // What's being called
    args: [arg1, arg2, ...args]  // Arguments
}
```

Everything else (quotes, lists, blocks) is also represented as Apply:

- `'expr` → `Apply(__quote__, [expr])`
- `~expr` → `Apply(__unquote__, [expr])`
- `[x, y, z]` → `Apply(__list__, [x, y, z])`
- Indented blocks → `Apply(__block__, [items...])`

### Examples

#### Function Application

```cadenza
# Whitespace-separated (preferred)
let x = 5
# Parses as: Apply(Ident(let), [Ident(x), Ident(=), Lit(5)])

# Or with explicit parentheses for clarity
let(x = 5)
# Parses as: Apply(Ident(let), [Ident(x), Ident(=), Lit(5)])

# Or even wrapped on the outside
(let x = 5)
# Parses as: Apply(Ident(let), [Ident(x), Ident(=), Lit(5)])
```

#### Operators as Functions

```cadenza
x + y
# Parses as: Apply(Ident(+), [Ident(x), Ident(y)])

(+) x y  # Parentheses make the infix function a callable function
```

#### Attributes

```cadenza
@alias s
@si nano, micro, milli
measure second
# Item with 2 attributes, Apply(Ident(measure), [Ident(second)]) as body
```

#### Indentation Blocks

```cadenza
let foo =
    let bar = 1
    let baz = 2
    bar + baz
# Parses as: Apply(Ident(let), [Ident(foo), Ident(=), Apply(Ident(__block__), [
#    Apply(Ident(let), [Ident(bar), Ident(=), Lit(1)]),
#    Apply(Ident(let), [Ident(baz), Ident(=), Lit(2)]),
#    Apply(Ident(+), [Ident(bar), Ident(baz)])
# ])])
```

#### Quote/Unquote

```cadenza
let var = 'foo          # Capture identifier as syntax
# Parses as: Apply(Ident(let), [Ident(var), Ident(=), Apply(Ident(__quote__), [Ident(foo)])])

let ast = '
    let out = ~var      # Splice captured syntax
# Parses as: Apply(Ident(let), [Ident(ast), Ident(=), Apply(Ident(__quote__), [
#   Apply(Ident(__block__), [
#     Apply(Ident(let), [Ident(out), Ident(=), Apply(Ident(__unquote__), [Ident(var)])])
#   ])
# ])])

~ast                    # Evaluate constructed tree
# Parses as: Apply(Ident(__unquote__), [Ident(ast)])
```

#### Lists

```cadenza
[1, 2, 3]
# Parses as: Apply(Ident(__list__), [Lit(1), Lit(2), Lit(3)])

(foo, bar)
# Parses as: Apply(Ident(__list__), [Ident(foo), Ident(bar)])
```

#### Enums (Algebraic Data Types)

Enums are defined as function applications, like all other constructs:

```cadenza
enum Result {
  Ok = {
    value = Integer,
  },
  Error = {
    message = String,
  },
}

# Create enum values using variant constructors
let success = Result.Ok { value = 42 }
let failure = Result.Error { message = "error" }

# Pattern matching on enums
match success
  Result.Ok { value } -> value
  Result.Error { message } -> 0
```

Parses as:
```
# Simplified - showing one variant for brevity
enum Result { Ok = { value = Integer } }
# → Apply(Ident(enum), [
#     Ident(Result),
#     Apply(Ident(__record__), [
#       Apply(Ident(=), [
#         Ident(Ok),
#         Apply(Ident(__record__), [
#           Apply(Ident(=), [Ident(value), Ident(Integer)])
#         ])
#       ])
#     ])
#   ])

Result.Ok { value = 42 }
# → Apply(
#     Apply(Ident(.), [Ident(Result), Ident(Ok)]),
#     [Apply(Ident(__record__), [
#       Apply(Ident(=), [Ident(value), Lit(42)])
#     ])]
#   )

match success true => ... false => ...
# → Apply(Apply(Apply(Ident(match), [Ident(success)]),
#     [Apply(Ident(=>), [Ident(true), ...])]),
#     [Apply(Ident(=>), [Ident(false), ...])])
```

### Parser Strategy

1. **No keywords** - Lexer only produces identifiers, literals, operators, punctuation
2. **Parser tracks indentation** - No virtual INDENT/DEDENT tokens
3. **Everything is Apply** - Even operators
4. **Hand-written** - Direct integration with rowan's GreenNodeBuilder
5. **Error recovery** - Create Error nodes for invalid syntax

### Binding Power

Even though operators are functions, binding power is handled by the parser. So for example, the following will be parsed as:

```
a + b * c
```

As either:

- Left-to-right: `Apply(Ident(*), [Apply(Ident(+), [Ident(a), Ident(b)]), Ident(c)])`
- Or with parentheses: `a + (b * c)` to control grouping

### LSP Integration

The parser creates a lossless CST with all whitespace and comments. The compiler:

1. Initializes the module scope with the prelude
2. Walks the top-level CST nodes and evaluates them
3. Each evaluated expression calls into the compiler and resolves the identifiers from the current scope
4. The called handlers can attach semantic information for the nodes which can be used in the LSP for highlighting, inline type hints, refactoring, etc.
5. Incrementally re-evaluates on changes

## Implementation Plan

- [x] Design document
- [ ] Update token types (remove keywords)
- [ ] Implement minimal node types
- [ ] Implement parser:
  - [ ] Item parsing with attributes
  - [ ] Expression parsing (Apply only)
  - [ ] Block parsing (indentation-aware)
  - [ ] Quote/unquote support
  - [ ] List/parentheses support
- [ ] Test with example.cdz
- [ ] Add error recovery

## Next Steps

After basic parser:

1. Implement semantic analyzer that recognizes built-ins
2. Add incremental evaluation framework
3. Implement LSP semantic tokens
4. Add quote/unquote evaluation
