# Cadenza K Framework Implementation Guide

This document provides a comprehensive guide to the Cadenza language semantics implemented in the K framework.

## Overview

The K framework implementation provides a formal, executable specification of Cadenza's operational semantics. Unlike the Rust implementation which focuses on efficiency and practical tooling, the K implementation prioritizes:

1. **Formal correctness** - Mathematical precision in semantics
2. **Executability** - The specification can be run directly
3. **Verification** - Enables formal reasoning about programs
4. **Tool generation** - Automatically generates interpreters and analyzers

## Architecture

The implementation is organized into several modules:

### Core Modules

- **syntax.k** - Defines the abstract syntax using parenthesized AST notation
- **configuration.k** - Defines the runtime state (cells) for program execution
- **cadenza.k** - Main module that combines all semantic rules

### Semantic Modules (semantics/)

- **literals.k** - Literal value evaluation (integers, floats, strings, booleans)
- **variables.k** - Variable bindings, lookup, and scoping
- **operators.k** - Arithmetic, comparison, and logical operators
- **functions.k** - Function definitions, applications, closures

## K Framework Concepts

### Configurations

Configurations define the program state using **cells**. Each cell holds a specific aspect of runtime state:

```k
configuration
  <cadenza>
    <k> $PGM:Pgm </k>          // Current computation
    <env> .List </env>          // Environment stack
    <scope> .Map </scope>        // Current scope (variable bindings)
    <return> .K </return>        // Return value (for functions)
    <output> .List </output>     // Output stream
    <error> .K </error>          // Error messages
  </cadenza>
```

#### Key Cells

- **`<k>`** - The computation cell holds the current expression/statement being evaluated
- **`<env>`** - Stack of parent scopes for lexical scoping
- **`<scope>`** - Current scope mapping identifiers to values
- **`<error>`** - Holds error messages when evaluation fails

### Rewrite Rules

Rewrite rules define how the state evolves. A rule has the form:

```k
rule <k> PATTERN => REPLACEMENT ... </k>
     <cell1> CELL1_PATTERN => CELL1_REPLACEMENT </cell1>
  requires CONDITION
```

Example from literals.k:

```k
rule <k> 'Integer(I:Int) => 'Integer(I) ... </k>
```

This says: "An integer literal evaluates to itself."

Example from variables.k:

```k
rule <k> 'Let(X:Id, E:Expr) => E ~> 'BindLet(X) ... </k>
```

This says: "To evaluate a let binding, first evaluate the expression E, then bind the result to X."

### Sequencing with `~>`

The `~>` operator sequences computations:

```k
A ~> B
```

Means: "Do A, then do B with the result of A still on the stack."

## Semantic Decisions

### Numeric Tower

Cadenza uses a numeric tower where types automatically promote:

```
Integer < Rational < Float
```

Implemented in operators.k:

- Integer operations return Integer (if exact)
- Integer division returns Rational (to preserve precision)
- Operations with Float return Float (precision lost)

### Lexical Scoping

Variables are looked up using lexical scoping:

1. Search current scope (`<scope>`)
2. If not found, search parent scopes (`<env>` stack)
3. If still not found, raise "undefined variable" error

Implemented in variables.k using:
- `pushScope` - Create new scope
- `popScope` - Return to parent scope
- Identifier lookup searches scope chain

### Closures

Functions capture their lexical environment:

```k
'Closure(Name, Params, Body, CapturedEnv)
```

When a function is defined:
1. Capture current scope as `CapturedEnv`
2. Store closure in environment
3. Return Unit

When a function is called:
1. Create new scope from `CapturedEnv`
2. Bind parameters to arguments
3. Evaluate body in new scope
4. Restore previous scope

This enables:
- First-class functions
- Closures over free variables
- Recursion (function name in its own captured environment)

### Evaluation Order

Cadenza uses **strict evaluation** (call-by-value):

1. Function expressions are evaluated first
2. Arguments are evaluated left-to-right
3. Results are bound to parameters
4. Function body is evaluated

## Parenthesized AST Format

Since K doesn't support Pratt parsing (used by Cadenza's concrete syntax), programs must be written in **parenthesized AST format**.

### Syntax Mapping

| Cadenza Concrete Syntax | Parenthesized AST Format |
|-------------------------|--------------------------|
| `42` | `'Integer(42)` |
| `3.14` | `'Float(3.14)` |
| `"hello"` | `'String("hello")` |
| `true` | `'Bool(true)` |
| `x` | `'Ident(x)` |
| `let x = 42` | `'Let(x, 'Integer(42))` |
| `x = 10` | `'Assign(x, 'Integer(10))` |
| `1 + 2` | `'Apply('Op(+), 'Integer(1), 'Integer(2))` |
| `fn double x = x * 2` | `'Fn(double, (x, .Ids), 'Apply('Op(*), 'Ident(x), 'Integer(2)))` |
| `double 5` | `'Apply('Ident(double), ('Integer(5), .Exprs))` |

### Lists

K uses special syntax for lists:

- Empty list: `.Ids`, `.Exprs`, `.Patterns`, etc.
- Single element: `(x, .Ids)`
- Multiple elements: `(x, y, z, .Ids)` or `(x, (y, (z, .Ids)))`

## Example Programs

### Hello World

```k
'String("Hello, world!")
```

### Variable Binding

```k
'Let(message, 'String("Hello")) ;
'Ident(message)
```

### Arithmetic

```k
'Let(x, 'Integer(10)) ;
'Let(y, 'Integer(20)) ;
'Apply('Op(+), 'Ident(x), 'Ident(y))
```

### Function Definition and Call

```k
'Fn(square, (x, .Ids), 'Apply('Op(*), 'Ident(x), 'Ident(x))) ;
'Apply('Ident(square), ('Integer(5), .Exprs))
```

Output: `'Integer(25)`

### Closure

```k
'Let(factor, 'Integer(10)) ;
'Fn(multiply_by_factor, (x, .Ids), 'Apply('Op(*), 'Ident(x), 'Ident(factor))) ;
'Apply('Ident(multiply_by_factor), ('Integer(5), .Exprs))
```

Output: `'Integer(50)` (the function captures `factor`)

### Recursion

```k
'Fn(factorial, (n, .Ids), 
  'Match('Ident(n),
    ('MatchArm('Integer(0), 'Integer(1)),
     'MatchArm(n, 'Apply('Op(*), 'Ident(n), 
                         'Apply('Ident(factorial), 
                                ('Apply('Op(-), 'Ident(n), 'Integer(1)), .Exprs)))),
     .MatchArms))) ;
'Apply('Ident(factorial), ('Integer(5), .Exprs))
```

## Extending the Semantics

### Adding a New Operator

1. Add the operator to `OpName` in syntax.k:
   ```k
   syntax OpName ::= "+" | ... | "**"  // Add power operator
   ```

2. Add semantic rules in operators.k:
   ```k
   rule <k> 'Apply('Op(**), 'Integer(Base:Int), 'Integer(Exp:Int)) 
            => 'Integer(Base ^Int Exp) ... </k>
   ```

3. Add tests in tests/:
   ```k
   // Test: Power operator
   'Apply('Op(**), 'Integer(2), 'Integer(10))
   ```

### Adding a New Language Feature

1. Define syntax in syntax.k
2. Add configuration cells if needed in configuration.k
3. Implement semantic rules in appropriate semantics/ file
4. Add tests
5. Update documentation

### Example: Adding Lists

1. Syntax (syntax.k):
   ```k
   syntax Value ::= ListValue
   syntax ListValue ::= "'List'" "(" Exprs ")" [klabel('List)]
   ```

2. Semantics (semantics/lists.k):
   ```k
   rule <k> 'List(Es:Exprs) => 'List(evaluateList(Es)) ... </k>
   
   rule <k> 'Apply('Op(head), 'List((E:Expr, _:Exprs))) => E ... </k>
   
   rule <k> 'Apply('Op(tail), 'List((_:Expr, Es:Exprs))) => 'List(Es) ... </k>
   ```

3. Test:
   ```k
   'Let(mylist, 'List(('Integer(1), 'Integer(2), 'Integer(3), .Exprs))) ;
   'Apply('Op(head), 'Ident(mylist))
   ```

## Debugging K Definitions

### Common Issues

1. **Syntax errors** - Check parentheses and commas in lists
2. **Non-terminating rules** - Add `requires` conditions to prevent infinite loops
3. **Overlapping rules** - Use `[owise]` attribute for default cases
4. **Type mismatches** - Ensure patterns match actual terms

### Debugging Techniques

1. **Use `--debug` flag**:
   ```bash
   krun --debug program.cdz
   ```

2. **Add output rules**:
   ```k
   rule <k> E:Expr => E ... </k>
        <output> ... .List => ListItem(E) </output>
   ```

3. **Search for all paths**:
   ```bash
   krun --search program.cdz
   ```

4. **Check intermediate states**:
   ```bash
   krun --depth 10 program.cdz
   ```

## Future Work

### Not Yet Implemented

- [ ] Pattern matching (only basic patterns work)
- [ ] Tuple values and destructuring
- [ ] Record values and field access
- [ ] List operations
- [ ] Cell/reference types for shared mutable state
- [ ] Type system (types are dynamic in current implementation)
- [ ] Units of measure
- [ ] Error recovery and better error messages

### Planned Extensions

1. **Type checking** - Add a separate type checking phase
2. **Optimization** - Add rewrite rules for constant folding
3. **Concurrency** - Model parallel execution
4. **Effects** - Model side effects and I/O
5. **Verification** - Prove properties about programs

## References

- [K Framework Homepage](https://kframework.org/)
- [K Tutorial](https://kframework.org/k-distribution/k-tutorial/)
- [K User Manual](https://kframework.org/docs/user_manual/)
- [Cadenza Semantics Documentation](../docs/semantics/)
- [K Examples Repository](https://github.com/runtimeverification/k/tree/master/k-distribution/pl-tutorial)

## Contributing

When adding new semantic rules:

1. Follow existing code structure
2. Add comprehensive tests
3. Document the semantics in comments
4. Update this guide with examples
5. Ensure rules are deterministic (unless modeling non-determinism)
6. Use meaningful variable names in patterns
7. Add `requires` clauses to prevent infinite loops

## License

Same as the main Cadenza project (MIT).
