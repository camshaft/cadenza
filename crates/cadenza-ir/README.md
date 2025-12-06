# Cadenza IR

The Intermediate Representation (IR) for the Cadenza compiler.

## Overview

This crate provides a typed, SSA-like (Single Static Assignment) IR that serves as a target-independent intermediate representation for the Cadenza compiler. The IR is designed to:

- Be easy to generate from the typed AST
- Support optimization passes
- Enable code generation to multiple backends
- Preserve source location information for debugging

## IR Design

### Key Concepts

**SSA Form**: Each value is assigned exactly once. Values are identified by unique `ValueId`s.

**Basic Blocks**: Instructions are organized into basic blocks, each ending with a terminator instruction (branch, jump, or return).

**Type Information**: The IR preserves type information from the type checker for code generation.

**Source Tracking**: Every instruction includes source location information (file, line, column) for generating source maps and accurate error messages.

## IR Structure

### Values

- `ValueId` - Unique identifier for SSA values (e.g., `%0`, `%1`)
- `BlockId` - Unique identifier for basic blocks (e.g., `block_0`)
- `FunctionId` - Unique identifier for functions (e.g., `@func_0`)

### Instructions

The IR supports the following instruction types:

- **Const** - Load constant values (nil, bool, integer, float, string, quantity)
- **BinOp** - Binary operations (add, sub, mul, div, eq, lt, etc.)
- **UnOp** - Unary operations (neg, not)
- **Call** - Function calls
- **Record** - Record construction
- **Field** - Field access
- **Tuple** - Tuple/list construction
- **Phi** - SSA phi nodes for control flow joins

### Terminators

Basic blocks end with one of these control flow instructions:

- **Branch** - Conditional branch (`br %cond, then: block_1, else: block_2`)
- **Jump** - Unconditional jump (`jmp block_3`)
- **Return** - Return from function (`ret %value`)

## Example IR

Here's what IR looks like for a simple function:

```
function add_one(%0: integer) -> integer {
  block_0:
    %1 = const 1
    %2 = add %0 %1
    ret %2
}
```

This represents the Cadenza function:
```cadenza
fn add_one x = x + 1
```

## Usage

The IR types can be constructed programmatically:

```rust
use cadenza_ir::*;
use cadenza_eval::{InternedString, Type};

// Create a simple function that returns 42
let func = IrFunction {
    id: FunctionId(0),
    name: InternedString::new("get_answer"),
    params: vec![],
    return_ty: Type::Integer,
    blocks: vec![
        IrBlock {
            id: BlockId(0),
            instructions: vec![
                IrInstr::Const {
                    result: ValueId(0),
                    value: IrConst::Integer(42),
                    source: SourceLocation { /* ... */ },
                }
            ],
            terminator: IrTerminator::Return {
                value: Some(ValueId(0)),
                source: SourceLocation { /* ... */ },
            },
        }
    ],
    entry_block: BlockId(0),
};
```

## Future Work

This crate currently provides the IR data structures. Future work includes:

### IR Generation (Phase 5)
- [ ] Builder API for constructing IR from typed AST
- [ ] Automatic SSA conversion
- [ ] Type-driven IR generation

### Optimization Passes (Phase 5)
- [ ] Constant folding
- [ ] Dead code elimination
- [ ] Common subexpression elimination
- [ ] Function inlining
- [ ] Configurable optimization pipeline

### Code Generation (Phase 5)
- [ ] TypeScript backend
- [ ] JavaScript backend
- [ ] Rust backend (emit Rust code)
- [ ] WASM backend (optional)
- [ ] LLVM backend (optional)

### Analysis and Validation
- [ ] IR verification (type checking, SSA validation)
- [ ] Control flow analysis
- [ ] Liveness analysis

## References

See `/docs/COMPILER_ARCHITECTURE.md` for detailed information about:
- The multi-phase compilation pipeline
- How IR fits into the overall architecture
- Code generation targets and their tradeoffs
- Optimization strategies
