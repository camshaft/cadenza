# Special Forms Refactoring Guide

This document describes the pattern for migrating `BuiltinMacro` implementations to the new `BuiltinSpecialForm` architecture.

## Overview

Special forms are fundamental language constructs that have both evaluation semantics and IR generation logic. They differ from macros in that they provide both runtime evaluation and compile-time IR generation capabilities.

## Migration Pattern

### 1. Create a New Module

Create a file in `src/special_form/` named after the special form (e.g., `let_form.rs`, `block_form.rs`).

### 2. Module Structure

Each special form module should follow this structure:

```rust
//! The `<name>` special form for <description>.

use crate::{
    context::EvalContext,
    diagnostic::{BoxedDiagnosticExt, Diagnostic, Result},
    ir::{BlockBuilder, IrGenContext, SourceLocation, ValueId},
    special_form::BuiltinSpecialForm,
    value::{Type, Value},
    Eval,
};
use cadenza_syntax::ast::Expr;
use std::sync::OnceLock;

/// Returns the `<name>` special form for <purpose>.
///
/// # Evaluation
/// - Describe evaluation behavior
///
/// # IR Generation
/// - Describe IR generation behavior (or note if not supported)
///
/// # Examples
/// ```cadenza
/// <example code>
/// ```
pub fn get() -> &'static BuiltinSpecialForm {
    static FORM: OnceLock<BuiltinSpecialForm> = OnceLock::new();
    FORM.get_or_init(|| BuiltinSpecialForm {
        name: "<name>",
        signature: Type::function(vec![...], ...),
        eval_fn: eval_<name>,
        ir_fn: ir_<name>,
    })
}

fn eval_<name>(args: &[Expr], ctx: &mut EvalContext<'_>) -> Result<Value> {
    // Implementation
}

fn ir_<name>(
    args: &[Expr],
    block: &mut BlockBuilder,
    ctx: &mut IrGenContext,
    source: SourceLocation,
    gen_expr: &mut dyn FnMut(&Expr, &mut BlockBuilder, &mut IrGenContext) -> Result<ValueId>,
) -> Result<ValueId> {
    // Implementation or return error if not supported
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Compiler, Env};
    use cadenza_syntax::parse::parse;

    #[test]
    fn test_<name>_special_form() {
        // Test cases
    }
}
```

### 3. Key Points

#### OnceLock Pattern
Use `OnceLock` to lazily initialize the static `BuiltinSpecialForm`:

```rust
static FORM: OnceLock<BuiltinSpecialForm> = OnceLock::new();
FORM.get_or_init(|| BuiltinSpecialForm { ... })
```

#### Error Handling
- Import `BoxedDiagnosticExt` trait for `.with_span()` on boxed diagnostics
- Use `Diagnostic::syntax()`, `Diagnostic::type_error()`, etc. which return `Box<Diagnostic>`
- Chain `.with_span()` to add source location information

#### IR Generation
- If IR generation is not yet supported, return an error:
  ```rust
  Err(Diagnostic::syntax("<feature> not yet supported in IR generation"))
  ```
- Use the `gen_expr` callback to generate IR for sub-expressions
- IR doesn't have lexical scoping - all variables are in the same SSA namespace

### 4. Register the Module

Add the module to `src/special_form.rs`:

```rust
pub mod <name>_form;
```

### 5. Update Environment Registration

Update `Env::register_standard_builtins()` in `src/env.rs`:

```rust
// Change from:
self.define(id, Value::BuiltinMacro(builtin_<name>()));

// To:
self.define(id, Value::SpecialForm(special_form::<name>_form::get()));
```

### 6. Update IR Generator (if needed)

If the special form needs special handling in IR generation, update `IrGenerator::gen_apply()` in `src/ir/generator.rs`:

```rust
if func_name_str == "<name>" {
    // Handle special form
    return self.gen_<name>(apply, block, ctx);
}
```

## Examples

### Completed Migrations

1. **let_form** - Variable declarations (`let x = 42`)
   - Located in `src/special_form/let_form.rs`
   - Supports both evaluation and IR generation
   - Demonstrates variable binding pattern

2. **block_form** - Block expressions (`__block__`)
   - Located in `src/special_form/block_form.rs`
   - Creates lexical scopes during evaluation
   - IR generation doesn't use scopes (SSA)

3. **list_form** - List literals (`__list__`)
   - Located in `src/special_form/list_form.rs`
   - Evaluation supported
   - IR generation not yet implemented

4. **assert_form** - Runtime assertions (`assert`)
   - Located in `src/special_form/assert_form.rs`
   - Evaluation with rich error messages
   - IR generation not yet implemented

5. **typeof_form** - Type queries (`typeof`)
   - Located in `src/special_form/typeof_form.rs`
   - Uses type inferencer for compile-time type information
   - IR generation not yet implemented

### Remaining Migrations

The following builtins still need to be migrated:

- `=` - Assignment operator
- `fn` - Function definition
- `__record__` - Record literals
- `measure` - Unit definitions
- `|>` - Pipeline operator
- `.` - Field access

Note: `match` pattern matching has been migrated to a special form.

## Testing

Each special form should have:
1. Unit tests in the module's `tests` module
2. Integration tests using `test-data/*.cdz` files
3. IR generation tests (snapshots)

## Benefits of Special Forms

1. **Unified Architecture**: Single type for language constructs
2. **Type Safety**: Compile-time guarantees for signatures
3. **IR Integration**: Built-in support for code generation
4. **Documentation**: Clear contract for evaluation and IR generation
5. **Extensibility**: Easy to add new special forms

## Migration Checklist

For each builtin to migrate:

- [ ] Create `<name>_form.rs` module
- [ ] Implement `get()` function with `OnceLock`
- [ ] Implement `eval_<name>` function
- [ ] Implement `ir_<name>` function
- [ ] Add module to `special_form.rs`
- [ ] Update `Env::register_standard_builtins()`
- [ ] Update IR generator if needed
- [ ] Add tests
- [ ] Run `cargo test` to verify
- [ ] Run `cargo clippy` to check for warnings
