# IR Module

The Intermediate Representation (IR) for the Cadenza compiler.

## Overview

This module provides a typed, SSA-like (Single Static Assignment) IR that serves as a target-independent intermediate representation for the Cadenza compiler. The IR is designed to:

- Be easy to generate from the typed AST
- Support optimization passes
- Target WebAssembly with WasmGC for memory management
- Preserve source location information for debugging

## Target Architecture

The IR targets **WebAssembly with WasmGC**, which provides:
- Automatic memory management via garbage collection
- Native browser execution
- AOT/JIT compilation for native execution via wasmtime
- Simplified backend - no need to manage allocations manually

This greatly simplifies the compiler architecture compared to targeting multiple backends directly.

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
use crate::ir::*;

// Create a simple function that returns 42
let mut builder = IrBuilder::new();
let mut func_builder = builder.function(
    InternedString::new("get_answer"),
    vec![],
    Type::Integer,
);

let mut block = func_builder.block();
let val = block.const_val(IrConst::Integer(42), source);
let (block, next_val) = block.ret(Some(val), source);
func_builder.add_block(block, next_val);

let func = func_builder.build();
builder.add_function(func);
```

## Future Work

### IR Generation (Phase 5)
- [x] Builder API for constructing IR from typed AST
- [x] Basic IR generation from AST expressions (literals, identifiers, binary operators)
- [x] Function generation with parameter bindings
- [x] Support for function calls (including recursive calls)
- [ ] Automatic SSA conversion (for mutable variables and control flow)
- [ ] Type-driven IR generation (using type inference results)
- [ ] Support for conditionals (if expressions with control flow)
- [ ] Support for records and field access
- [ ] Support for lists/tuples

### Optimization Passes (Phase 5)
- [ ] Constant folding
- [ ] Dead code elimination
- [ ] Common subexpression elimination
- [ ] Function inlining
- [ ] Configurable optimization pipeline

### Code Generation (Phase 5)
- [ ] WASM backend with WasmGC
- [ ] Native execution via wasmtime (AOT/JIT)

### Analysis and Validation
- [ ] IR verification (type checking, SSA validation)
- [ ] Control flow analysis
- [ ] Liveness analysis

## References

See `/docs/COMPILER_ARCHITECTURE.md` for detailed information about:
- The multi-phase compilation pipeline
- How IR fits into the overall architecture
- Code generation strategy
- Optimization strategies
