# Cadenza

A statically-typed language for creative coding with dimensional analysis and interactive development.

## Documentation

### Architecture & Design
- **[Architecture Review](docs/ARCHITECTURE_REVIEW.md)** - Comprehensive review of the compiler pipeline (Parse → Evaluate → Type Check → IR → Codegen)
- **[Architecture Quick Reference](docs/ARCHITECTURE_REVIEW_QUICK_REFERENCE.md)** - TL;DR of architectural decisions
- **[Compiler Architecture](docs/COMPILER_ARCHITECTURE.md)** - Detailed specification of the multi-phase compiler
- **[Evaluator Design](crates/cadenza-eval/DESIGN.md)** - Design of the tree-walk evaluator

### Use Cases
- **[3D Modeling Environment](docs/3D_MODELING_ENVIRONMENT.md)** - Code-based 3D modeling like OpenSCAD
- **[Music Composition Environment](docs/MUSIC_COMPOSITION_ENVIRONMENT.md)** - Algorithmic music composition
- **[REPL/Calculator Environment](docs/REPL_CALCULATOR_ENVIRONMENT.md)** - Fast scratch computation with units
- **[Visual Art & Interactive Books](docs/VISUAL_ART_AND_INTERACTIVE_BOOKS.md)** - Generative art and visualizations
- **[G-code Interpreter Environment](docs/GCODE_INTERPRETER_ENVIRONMENT.md)** - CNC/3D printer control
- **[Web Compiler Explorer](docs/WEB_COMPILER_EXPLORER.md)** - In-browser interactive development

## Project Status

This is an early-stage project focused on getting the architecture and core features right. See:
- **[Evaluator Status](crates/cadenza-eval/STATUS.md)** - Current implementation status and roadmap
- **[Agent Guidelines](AGENTS.md)** - Guidelines for contributors and AI agents

## Getting Started

```bash
# Build the project
cargo build

# Run tests
cargo test

# Run CI checks (formatting, clippy, tests)
cargo xtask ci
```

See [AGENTS.md](AGENTS.md) for more details on development workflow.
