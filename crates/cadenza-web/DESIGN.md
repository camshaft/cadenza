# Cadenza Web — Design Document

## Overview

The `cadenza-web` crate provides WebAssembly bindings for the Cadenza compiler, enabling the compiler to run directly in web browsers. This supports two primary goals:

1. **Interactive Compiler Development**: A web-based "compiler explorer" that displays compilation stages side-by-side, making it easy to visualize and debug the compiler's behavior.

2. **Portable, Web-Friendly Compiler**: Ensuring the Cadenza compiler is portable and web-friendly from the earliest stages of development prevents platform-specific assumptions from creeping into the codebase.

## Architecture

### WASM Bindings

The crate exposes the following compiler stages as WASM-callable functions:

```
┌─────────────────────────────────────────────────────────────────┐
│                        Browser                                   │
│  ┌────────────┐    ┌────────────┐    ┌────────────┐             │
│  │   Editor   │───▶│   WASM     │───▶│   Panels   │             │
│  │  (Monaco)  │    │  Compiler  │    │  (Output)  │             │
│  └────────────┘    └────────────┘    └────────────┘             │
└─────────────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────────┐
│                    cadenza-web (WASM)                            │
│  ┌──────────┐   ┌──────────┐   ┌──────────┐   ┌──────────┐     │
│  │  lex()   │──▶│ parse()  │──▶│  ast()   │──▶│  eval()  │     │
│  └──────────┘   └──────────┘   └──────────┘   └──────────┘     │
└─────────────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────────┐
│                    Rust Crates                                   │
│  ┌────────────────┐           ┌────────────────┐                │
│  │ cadenza-syntax │           │  cadenza-eval  │                │
│  │  - lexer       │           │  - interner    │                │
│  │  - parser      │           │  - env         │                │
│  │  - AST         │           │  - compiler    │                │
│  └────────────────┘           └────────────────┘                │
└─────────────────────────────────────────────────────────────────┘
```

### API Surface

The WASM module exports the following functions, each returning JSON:

1. **`lex(source: string) -> LexResult`**
   - Tokenizes the source code
   - Returns array of tokens with their kinds, spans, and text

2. **`parse(source: string) -> ParseResult`**
   - Parses the source code into a concrete syntax tree (CST)
   - Returns the CST as a serializable tree structure

3. **`ast(source: string) -> AstResult`**
   - Converts the CST to an abstract syntax tree (AST)
   - Returns the AST as a serializable tree structure

4. **`eval(source: string) -> EvalResult`**
   - Evaluates the source code
   - Returns the resulting values and any diagnostics

### Data Serialization

All outputs are serialized to JSON using `serde-wasm-bindgen` for efficient transfer between Rust and JavaScript. The JSON structures mirror the internal Rust types but are simplified for web consumption.

## Compiler Explorer Application

The companion TypeScript application (`crates/cadenza-web/app`) provides an interactive UI:

### Stack

- **Vite**: Fast development server and build tool
- **React**: UI framework
- **TypeScript**: Type safety
- **Tailwind CSS**: Styling
- **Monaco Editor**: Same editor as VS Code for the source input

### Layout

```
┌─────────────────────────────────────────────────────────────────┐
│                         Cadenza Compiler Explorer                │
├─────────────────────────────────────────────────────────────────┤
│ ┌─────────────────────────┐ ┌─────────────────────────────────┐ │
│ │                         │ │  Tabs: [Tokens][CST][AST][Eval] │ │
│ │     Source Editor       │ ├─────────────────────────────────┤ │
│ │       (Monaco)          │ │                                 │ │
│ │                         │ │        Output Panel             │ │
│ │                         │ │                                 │ │
│ │                         │ │                                 │ │
│ └─────────────────────────┘ └─────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────┘
```

### Features

- Real-time compilation as you type (debounced)
- Tab-based view for different compilation stages
- Syntax highlighting in output panels
- Error display with source locations
- Resizable panels

## Non-Goals

- **Production runtime**: This is a development/educational tool
- **Full IDE features**: Basic editing only, no autocomplete or refactoring
- **Optimization**: Readability over performance in the output

## Implementation Notes

### WASM Considerations

- The interned strings in `cadenza-eval` use `OnceLock` which works in WASM
- No filesystem or network access is needed
- All computation is synchronous

### Future Extensions

- Add more compilation stages as they are implemented
- Support for stepping through evaluation
- Export/share functionality
- Theme customization
