# Converting Cadenza Concrete Syntax to K Parenthesized AST

This guide explains how to manually convert Cadenza programs from their concrete syntax to the parenthesized AST format required by the K framework implementation.

## Basic Principles

1. Every syntactic construct becomes a function-like term: `'Constructor(args...)`
2. Single quotes around constructors distinguish them from K builtins
3. Lists use special K syntax: `.ListName` for empty, `(elem, rest)` for non-empty
4. The AST structure mirrors what Cadenza's parser produces

## Literal Values

### Integers
```cadenza
42
```
```k
'Integer(42)
```

### Floats
```cadenza
3.14
```
```k
'Float(3.14)
```

### Strings
```cadenza
"hello"
```
```k
'String("hello")
```

### Booleans
```cadenza
true
false
```
```k
'Bool(true)
'Bool(false)
```

### Unit (empty tuple)
```cadenza
()
```
```k
'Unit()
```

## Identifiers

Any identifier becomes `'Ident(name)`:

```cadenza
x
my_variable
counter
```
```k
'Ident(x)
'Ident(my_variable)
'Ident(counter)
```

## Let Bindings

### Simple binding
```cadenza
let x = 42
```
```k
'Let(x, 'Integer(42))
```

### Binding with expression
```cadenza
let result = 10 + 5
```
```k
'Let(result, 'Apply('Op(+), 'Integer(10), 'Integer(5)))
```

### Multiple statements
```cadenza
let x = 1
let y = 2
```
```k
'Let(x, 'Integer(1)) ;
'Let(y, 'Integer(2))
```

## Operators

All binary operators use `'Apply` with `'Op`:

### Arithmetic
```cadenza
1 + 2
10 - 5
3 * 4
10 / 2
10 % 3
```
```k
'Apply('Op(+), 'Integer(1), 'Integer(2))
'Apply('Op(-), 'Integer(10), 'Integer(5))
'Apply('Op(*), 'Integer(3), 'Integer(4))
'Apply('Op(/), 'Integer(10), 'Integer(2))
'Apply('Op(%), 'Integer(10), 'Integer(3))
```

### Comparison
```cadenza
x == y
x != y
x < y
x <= y
x > y
x >= y
```
```k
'Apply('Op(==), 'Ident(x), 'Ident(y))
'Apply('Op(!=), 'Ident(x), 'Ident(y))
'Apply('Op(<), 'Ident(x), 'Ident(y))
'Apply('Op(<=), 'Ident(x), 'Ident(y))
'Apply('Op(>), 'Ident(x), 'Ident(y))
'Apply('Op(>=), 'Ident(x), 'Ident(y))
```

### Logical
```cadenza
true && false
true || false
!true
```
```k
'Apply('Op(&&), 'Bool(true), 'Bool(false))
'Apply('Op(||), 'Bool(true), 'Bool(false))
'Apply('Op(!), 'Bool(true))
```

### Nested expressions
```cadenza
(1 + 2) * 3
```
```k
'Apply('Op(*), 
       'Apply('Op(+), 'Integer(1), 'Integer(2)), 
       'Integer(3))
```

## Functions

### Function definition with no parameters
```cadenza
fn get_value = 42
```
```k
'Fn(get_value, .Ids, 'Integer(42))
```

### Function definition with one parameter
```cadenza
fn double x = x * 2
```
```k
'Fn(double, (x, .Ids), 
    'Apply('Op(*), 'Ident(x), 'Integer(2)))
```

### Function definition with multiple parameters
```cadenza
fn add x y = x + y
```
```k
'Fn(add, (x, y, .Ids), 
    'Apply('Op(+), 'Ident(x), 'Ident(y)))
```

### Function definition with three parameters
```cadenza
fn sum3 a b c = a + b + c
```
```k
'Fn(sum3, (a, b, c, .Ids), 
    'Apply('Op(+), 
           'Apply('Op(+), 'Ident(a), 'Ident(b)), 
           'Ident(c)))
```

## Function Application

### No arguments (zero-parameter function)
```cadenza
get_value
```
```k
'Ident(get_value)
```
Note: Zero-parameter functions are auto-applied when referenced.

### One argument
```cadenza
double 5
```
```k
'Apply('Ident(double), ('Integer(5), .Exprs))
```

### Multiple arguments
```cadenza
add 3 7
```
```k
'Apply('Ident(add), ('Integer(3), 'Integer(7), .Exprs))
```

### Nested function calls
```cadenza
double (add 3 4)
```
```k
'Apply('Ident(double), 
       ('Apply('Ident(add), ('Integer(3), 'Integer(4), .Exprs)), 
        .Exprs))
```

## Complete Program Examples

### Example 1: Simple arithmetic
```cadenza
let x = 10
let y = 20
x + y
```
```k
'Let(x, 'Integer(10)) ;
'Let(y, 'Integer(20)) ;
'Apply('Op(+), 'Ident(x), 'Ident(y))
```

### Example 2: Function definition and use
```cadenza
fn square x = x * x
square 5
```
```k
'Fn(square, (x, .Ids), 
    'Apply('Op(*), 'Ident(x), 'Ident(x))) ;
'Apply('Ident(square), ('Integer(5), .Exprs))
```

### Example 3: Closure
```cadenza
let multiplier = 10
fn multiply_by_ten x = x * multiplier
multiply_by_ten 5
```
```k
'Let(multiplier, 'Integer(10)) ;
'Fn(multiply_by_ten, (x, .Ids), 
    'Apply('Op(*), 'Ident(x), 'Ident(multiplier))) ;
'Apply('Ident(multiply_by_ten), ('Integer(5), .Exprs))
```

### Example 4: Higher-order function
```cadenza
fn apply f x = f x
fn double x = x * 2
apply double 5
```
```k
'Fn(apply, (f, x, .Ids), 
    'Apply('Ident(f), ('Ident(x), .Exprs))) ;
'Fn(double, (x, .Ids), 
    'Apply('Op(*), 'Ident(x), 'Integer(2))) ;
'Apply('Ident(apply), 
       ('Ident(double), 'Integer(5), .Exprs))
```

## List Construction Rules

### Empty Lists
```k
.Ids        // Empty identifier list
.Exprs      // Empty expression list
.Patterns   // Empty pattern list
.MatchArms  // Empty match arms list
```

### Single Element Lists
```k
(x, .Ids)           // Single identifier
('Integer(1), .Exprs)  // Single expression
```

### Multiple Element Lists
```k
(x, y, z, .Ids)                    // Three identifiers
('Integer(1), 'Integer(2), .Exprs) // Two expressions
```

Alternative nested form (also valid):
```k
(x, (y, (z, .Ids)))                      // Three identifiers
('Integer(1), ('Integer(2), .Exprs))     // Two expressions
```

## Tips and Tricks

1. **Start with innermost expressions**: Work from inside out when converting nested expressions.

2. **Count your parentheses**: K syntax requires precise parenthesization. Each `(` needs a matching `)`.

3. **Don't forget list terminators**: Lists must end with `.Ids`, `.Exprs`, etc.

4. **Use whitespace**: K allows arbitrary whitespace. Use it for readability.

5. **Test incrementally**: Start with simple expressions and build up complexity.

6. **Pattern for operators**: `'Apply('Op(OPERATOR), LEFT, RIGHT)` for binary operators.

7. **Pattern for functions**: `'Fn(NAME, (PARAMS..., .Ids), BODY)` for definitions.

8. **Pattern for calls**: `'Apply(FUNC, (ARGS..., .Exprs))` for applications.

## Common Mistakes

### Mistake 1: Missing list terminators
```k
// WRONG
'Fn(double, (x), ...)

// RIGHT
'Fn(double, (x, .Ids), ...)
```

### Mistake 2: Wrong quotes
```k
// WRONG
"Integer"(42)

// RIGHT
'Integer(42)
```

### Mistake 3: Missing 'Apply for operators
```k
// WRONG
'Op(+)('Integer(1), 'Integer(2))

// RIGHT
'Apply('Op(+), 'Integer(1), 'Integer(2))
```

### Mistake 4: Wrong argument structure
```k
// WRONG
'Apply('Ident(f), 'Integer(5))

// RIGHT
'Apply('Ident(f), ('Integer(5), .Exprs))
```

## Automation

In the future, a tool will be provided to automatically convert Cadenza concrete syntax to K format:

```bash
cadenza ast input.cdz -o output.k
```

Until then, manual conversion using this guide is required.

## Further Reading

- [K Framework Tutorial](https://kframework.org/k-distribution/k-tutorial/)
- [K Syntax Guide](https://kframework.org/docs/user_manual/)
- [Cadenza K Implementation Guide](GUIDE.md)
