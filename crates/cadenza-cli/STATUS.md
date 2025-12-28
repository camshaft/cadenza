# Cadenza CLI - Status Document

This document tracks the current state of the `cadenza` CLI crate and remaining work items.

## Overview

The Cadenza CLI provides command-line tools for working with the Cadenza language, including an interactive REPL and Language Server Protocol (LSP) server.

## Current State

### ✅ Completed

**REPL (Read-Eval-Print Loop):**
- Interactive evaluation with expression-by-expression feedback
- Persistent command history (saved to `~/.cadenza_history`)
- Syntax highlighting using cadenza-syntax lexer with ANSI colors
- Basic tab completion for built-in keywords and operators
- File pre-loading with `--load` flag
- Proper string escaping in output (quotes, newlines, tabs, backslashes)
- Clean error reporting for parse and evaluation errors

**LSP Server:**
- Stdio transport for editor integration
- Full integration with cadenza-lsp backend

**CLI Structure:**
- Clap-based subcommand architecture
- `repl` subcommand with optional `--load <FILE>` parameter
- `lsp` subcommand for starting LSP server

## Known Gaps & Future Enhancements

### REPL Auto-completion

**Current Limitation:**
The auto-completion system uses a hardcoded list of built-in keywords and operators. It cannot suggest:
- User-defined variables from the environment
- User-defined functions
- Identifiers loaded from `--load` files
- Dynamically defined symbols

**Root Cause:**
The `Env` struct doesn't expose an API to iterate over bindings. This means the REPL cannot query what identifiers are currently in scope.

**Potential Solutions:**
1. **Add iteration API to Env** - Add methods like `iter_bindings()` or `all_identifiers()` to expose available symbols
   - Pro: Clean, direct access to environment state
   - Con: May expose internal implementation details
   
2. **Maintain a separate completion registry** - Track identifiers separately in the REPL
   - Pro: Keeps Env API minimal
   - Con: Requires duplicate tracking and synchronization
   
3. **Extract identifiers from compiler** - Use the Compiler's knowledge of definitions
   - Pro: Leverages existing infrastructure
   - Con: May not capture all runtime bindings

**Related Code:**
- `crates/cadenza/src/repl.rs:57-62` - Hardcoded builtin list
- `crates/cadenza-eval/src/env.rs` - Env implementation

### REPL Syntax Highlighting

**Enhancement Opportunity:**
Current syntax highlighting is functional but basic. Potential improvements:
- Highlight user-defined identifiers differently from builtins
- Error highlighting for invalid syntax as you type
- Configurable color schemes
- More sophisticated token classification

### REPL History Management

**Current Behavior:**
History is saved to `~/.cadenza_history` with fallback to `.cadenza_history` in current directory.

**Potential Enhancements:**
- History search (Ctrl+R style reverse search)
- Multi-line history entries
- History size limits and rotation
- Per-project history files

### Multi-line Input

**Not Yet Implemented:**
The REPL currently evaluates single lines. For complex expressions spanning multiple lines, users must write them on one line or use semicolons.

**Desired Behavior:**
- Detect incomplete expressions and continue prompting
- Support for explicit multi-line mode (e.g., `\` continuation)
- Bracket/parenthesis matching to determine completeness

### REPL Commands

**Not Yet Implemented:**
No special REPL commands beyond Ctrl+D/Ctrl+C to exit.

**Potential Commands:**
- `:help` - Show REPL help
- `:quit` or `:exit` - Exit REPL
- `:load <file>` - Load file at runtime
- `:reload` - Reload the initially loaded file
- `:clear` - Clear environment or history
- `:type <expr>` - Show type of expression
- `:info <identifier>` - Show information about identifier

## Priority

**High Priority:**
- Add Env iteration API for better auto-completion (blocks full completion functionality)

**Medium Priority:**
- Multi-line input support (quality of life improvement)
- REPL commands (`:help`, `:load`, `:type`)

**Low Priority:**
- Enhanced syntax highlighting
- Configurable color schemes
- History search
