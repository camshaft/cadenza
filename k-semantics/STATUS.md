# Status and Limitations

This document tracks the implementation status of Cadenza features in the K framework and documents known limitations.

## Implemented Features

### ✅ Literals
- [x] Integer literals (128-bit signed)
- [x] Float literals (64-bit IEEE 754)
- [x] Rational literals (created via division)
- [x] String literals
- [x] Boolean literals (true/false)
- [x] Unit literal (empty tuple)

### ✅ Variables and Bindings
- [x] Let bindings with identifier patterns
- [x] Variable lookup with lexical scoping
- [x] Shadowing (rebinding same name)
- [x] Reassignment (modifying existing binding)
- [x] Scope stack for nested blocks

### ✅ Operators
- [x] Arithmetic operators (+, -, *, /, %)
- [x] Integer arithmetic
- [x] Float arithmetic
- [x] Rational arithmetic with automatic simplification
- [x] Mixed-type arithmetic with numeric tower
- [x] Comparison operators (==, !=, <, <=, >, >=)
- [x] Logical operators (&&, ||, !)

### ✅ Functions
- [x] Function definitions with fn keyword
- [x] Zero-parameter functions (auto-applied)
- [x] Single-parameter functions
- [x] Multi-parameter functions
- [x] Function application
- [x] Closures (capture lexical environment)
- [x] Recursion (function name in own scope)
- [x] Higher-order functions

### ✅ Program Structure
- [x] Sequence of statements
- [x] Expression evaluation
- [x] Block expressions (partial)

## Partially Implemented Features

### ⚠️ Pattern Matching
- [x] Identifier patterns
- [x] Wildcard patterns
- [ ] Tuple patterns (syntax defined, semantics TODO)
- [ ] Record patterns (syntax defined, semantics TODO)
- [ ] Literal patterns in match arms
- [ ] List patterns

### ⚠️ Blocks
- [x] Basic block syntax
- [x] Block creates new scope
- [ ] Block returns last expression value
- [ ] Proper cleanup/scope restoration

### ⚠️ Match Expressions
- [x] Basic match syntax
- [x] Pattern matching against arms
- [ ] Exhaustiveness checking
- [ ] Guard clauses
- [ ] Proper error on no match

## Not Yet Implemented

### ❌ Data Types

#### Tuples
- [ ] Tuple literal syntax
- [ ] Tuple value type
- [ ] Tuple destructuring
- [ ] Tuple field access

#### Records
- [ ] Record literal syntax
- [ ] Record value type
- [ ] Record field access
- [ ] Record updates
- [ ] Partial record patterns

#### Lists
- [ ] List literal syntax
- [ ] List value type
- [ ] List operations (head, tail, cons, append)
- [ ] List pattern matching

### ❌ Advanced Features

#### Type System
- [ ] Type annotations
- [ ] Type inference
- [ ] Type checking rules
- [ ] Generic types
- [ ] Type aliases

#### Units of Measure
- [ ] Measure annotations
- [ ] Dimensional analysis
- [ ] Unit conversions
- [ ] Measure arithmetic

#### Cell/Reference Types
- [ ] Cell.new for shared mutable state
- [ ] Cell.get for reading
- [ ] Cell.set for writing
- [ ] Reference counting semantics

#### Macros
- [ ] Macro definition
- [ ] Macro expansion
- [ ] Special forms

#### Documentation
- [ ] Doc comments
- [ ] Doc extraction

### ❌ I/O and Effects
- [ ] Print/output operations
- [ ] File I/O
- [ ] Standard library functions
- [ ] Error types and handling

### ❌ Advanced Control Flow
- [ ] If-then-else expressions
- [ ] While loops
- [ ] For loops
- [ ] Break/continue

### ❌ Modules and Imports
- [ ] Module definitions
- [ ] Import statements
- [ ] Module paths
- [ ] Visibility modifiers

## Known Limitations

### 1. Parser Limitations

The K implementation requires **parenthesized AST format** as input because:
- K doesn't support Pratt parsing (operator precedence parsing)
- K's parser generator is designed for traditional grammars
- Cadenza's concrete syntax uses Pratt parsing for operators

**Workaround:** Programs must be written in parenthesized AST format or converted from concrete syntax using external tools.

### 2. Error Messages

K's error messages are less informative than Rust implementation:
- No source location tracking
- No syntax highlighting in errors
- Limited context in error reports

**Mitigation:** Better error rules could be added to the K definition.

### 3. Performance

K semantics are designed for **correctness and verification**, not performance:
- Interpreted execution is slower than compiled code
- No optimization passes
- Memory management is implicit

**Note:** The K implementation is for formal specification and verification, not production use.

### 4. Argument Evaluation

Current implementation has simplified argument evaluation:
- Arguments should be evaluated left-to-right
- Side effects should be ordered
- Current implementation may not preserve evaluation order correctly

**TODO:** Implement proper sequential argument evaluation.

### 5. Environment Capture

Function closures capture the environment by **value** (immutable):
- Captured variables are copied, not referenced
- Reassignment of captured variables doesn't affect closures
- Different from the Rust implementation's reference semantics

**Note:** Cell types (when implemented) will provide shared mutable state.

### 6. No Standard Library

K implementation has no standard library:
- No built-in functions beyond operators
- No string operations
- No list operations
- No I/O functions

**Future:** Standard library functions can be added as K rules.

### 7. Integer Overflow

K uses arbitrary precision integers:
- No 128-bit limit in K (unlike semantics spec)
- Overflow behavior differs from Rust implementation
- Could add explicit overflow checks

### 8. Float Precision

K's float support may differ from Rust:
- Different rounding behavior
- Different handling of infinities/NaN
- May not match IEEE 754 exactly

## Comparison: K vs Rust Implementation

| Feature | K Implementation | Rust Implementation |
|---------|------------------|---------------------|
| **Purpose** | Formal specification | Production tooling |
| **Input** | Parenthesized AST | Concrete syntax |
| **Parser** | K parser generator | Hand-written Pratt parser |
| **Execution** | Interpreted by K | Compiled to native |
| **Performance** | Slow (for verification) | Fast (for real use) |
| **Error Messages** | Basic | Rich with spans |
| **Type System** | Not implemented | Not yet implemented |
| **Formal Proofs** | Supported by K | Not applicable |
| **Tool Generation** | Auto (from K def) | Manual (separate tools) |
| **Completeness** | Core features only | More complete |

## Testing Strategy

### Current Tests (15 test cases)
1. Literal values (integers, floats, strings, booleans)
2. Basic arithmetic (addition, subtraction, multiplication)
3. Division creating rationals
4. Rational simplification
5. Mixed arithmetic
6. Comparison operators
7. Logical operators
8. Let bindings
9. Variable references
10. Shadowing
11. Function definitions
12. Function applications
13. Multi-parameter functions
14. Closures
15. Higher-order functions

### Needed Tests
- [ ] Error cases (division by zero, undefined variables, type mismatches)
- [ ] Edge cases (max int, min int, NaN, infinity)
- [ ] Nested scopes
- [ ] Mutual recursion
- [ ] Pattern matching
- [ ] Blocks with multiple statements

## Future Enhancements

### Short Term
1. Complete pattern matching implementation
2. Add tuple support
3. Add record support
4. Improve error messages
5. Add more test cases

### Medium Term
1. Implement type checking rules
2. Add list operations
3. Add Cell/reference types
4. Add standard library functions
5. Create AST converter tool (concrete → K format)

### Long Term
1. Prove correctness properties using K's verification tools
2. Generate verified compiler from K semantics
3. Model concurrency and parallelism
4. Add effect system
5. Prove type soundness

## Contributing

To contribute to the K implementation:

1. **Pick a feature from "Not Yet Implemented"**
2. **Add syntax to syntax.k**
3. **Add configuration cells if needed**
4. **Write semantic rules** in appropriate semantics/ module
5. **Add test cases** in tests/
6. **Document** the feature in GUIDE.md
7. **Update this status document**

See [GUIDE.md](GUIDE.md) for implementation details.

## Resources

- [K Framework Documentation](https://kframework.org/docs/)
- [K Tutorial](https://kframework.org/k-distribution/k-tutorial/)
- [Cadenza Semantics Docs](../docs/semantics/)
- [K Examples](https://github.com/runtimeverification/k/tree/master/k-distribution/pl-tutorial)

## Version History

- **v0.1** (2026-02-07): Initial implementation
  - Core literals, variables, operators, functions
  - 15 test cases
  - Basic documentation
