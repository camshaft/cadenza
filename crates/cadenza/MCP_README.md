# Cadenza MCP Server

The Cadenza MCP (Model Context Protocol) server enables LLMs to interact with the Cadenza programming language environment.

## Features

The MCP server exposes the following tools:

### Core Tools

- **eval** - Evaluate Cadenza expressions and return results
  - Run calculations
  - Test code
  - Perform dimensional analysis
  
- **parse** - Parse Cadenza code and display the Abstract Syntax Tree (AST)
  - Understand code structure
  - Debug parsing issues
  
- **infer_type** - Infer the type of a Cadenza expression
  - Understand what type an expression evaluates to
  - Verify type correctness
  
- **check_dimensions** - Check dimensional analysis for expressions with units
  - Verify unit compatibility
  - Check unit conversions
  - Validate dimensional correctness

### Discovery Tools

- **list_builtins** - List all built-in functions, operators, and special forms
  - Discover available functionality
  - Explore language features
  
- **get_docs** - Get documentation for specific symbols, functions, or operators
  - Learn how to use language features
  - Understand operator behavior
  
- **about_cadenza** - Get comprehensive information about the Cadenza language
  - Learn about language features
  - Understand design philosophy
  - See example code

## Usage

### Starting the Server

```bash
cadenza mcp
```

The server communicates over standard input/output (stdio transport), following the MCP protocol.

### Configuring with Claude Desktop

Add this configuration to your Claude Desktop config file (`claude_desktop_config.json`):

**macOS/Linux:**
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

**Windows:**
```json
{
  "mcpServers": {
    "cadenza": {
      "command": "C:\\path\\to\\cadenza.exe",
      "args": ["mcp"]
    }
  }
}
```

### Example Interactions

Once configured, you can ask Claude to:

- **Run calculations**: "Use cadenza to calculate 100 meters per second in miles per hour"
- **Check types**: "What type does this expression evaluate to: `fn x -> x * 2`?"
- **Verify units**: "Check if `10meter + 5second` is valid in Cadenza"
- **Explore syntax**: "Show me the AST for this Cadenza code: `let x = 42`"
- **Learn features**: "What operators are available in Cadenza?"

## About Cadenza

Cadenza is a functional programming language with first-class support for units of measure and dimensional analysis. Key features include:

- **Units of Measure**: Built-in support for physical units with automatic dimensional analysis
- **Functional Programming**: Functions are first-class values
- **Type Safety**: Static type checking with type inference
- **Interactive REPL**: Immediate feedback and exploration

### Example Code

```cadenza
# Define units
measure meter
measure second

# Use units in calculations
let distance = 100meter
let time = 10second
let speed = distance / time  # Automatically: meter/second

# Define functions
fn kinetic_energy mass velocity =
    0.5 * mass * velocity * velocity

# Use functions
let energy = kinetic_energy 1000kilogram 20meter/second
```

## Development

The MCP server is implemented in the `crates/cadenza/src/mcp.rs` module using the [rmcp](https://github.com/modelcontextprotocol/rust-sdk) Rust SDK.

### Architecture

- Each MCP tool call creates a fresh `Compiler` and `Env` instance
- This ensures isolation between requests
- No persistent state is maintained across calls
- Thread-safe by design (stateless)

### Testing

You can test the MCP server using the MCP Inspector:

```bash
npx @modelcontextprotocol/inspector /path/to/cadenza mcp
```

This provides an interactive UI for testing the MCP tools.

## Protocol Version

The server implements MCP protocol version `2024-11-05`.

## License

MIT
