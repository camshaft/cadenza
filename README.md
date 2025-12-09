# Cadenza

Cadenza is a programming language with a comprehensive toolchain.

## Installation

Build from source:

```bash
cargo build --release
```

The `cadenza` binary will be available at `target/release/cadenza`.

## Usage

### REPL

Start an interactive REPL (Read-Eval-Print Loop):

```bash
cadenza repl
```

The REPL provides:
- **Command history**: Navigate through previous commands with up/down arrows (saved to `~/.cadenza_history`)
- **Syntax highlighting**: Real-time syntax highlighting for Cadenza code
- **Auto-completion**: Tab completion for built-in identifiers and keywords
- **Expression evaluation**: Evaluate expressions interactively and see results immediately

#### Loading files

Load a Cadenza source file into the REPL scope before starting:

```bash
cadenza repl --load path/to/file.cdz
```

This pre-loads all definitions from the file, making them available in the REPL session.

### Language Server Protocol (LSP)

Start the LSP server for editor integration:

```bash
cadenza lsp
```

This enables IDE features like code completion, diagnostics, and go-to-definition in editors that support LSP.

### Model Context Protocol (MCP)

Start the MCP server for LLM integration:

```bash
cadenza mcp
```

This enables LLMs like Claude to interact with Cadenza, providing code evaluation, type checking, dimensional analysis, and more.

Configure with Claude Desktop by adding to `claude_desktop_config.json`:
```json
{
  "mcpServers": {
    "cadenza": {
      "command": "/path/to/cadenza",
      "args": ["mcp"]
    }
  }
}
```

## License

MIT
