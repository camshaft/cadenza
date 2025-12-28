# Functions

Functions are first-class values that encapsulate computation. They can be defined, passed as arguments, returned from other functions, and stored in data structures.

## Function Definition

The `fn` keyword defines a function with parameters and a body expression. Functions capture their lexical environment at definition time, making them proper closures.

### Syntax

```
fn <name> <param1> <param2> ... = <body>
```

The function name follows identifier syntax rules. Parameters are identifier patterns. The body is any expression.

### Semantics

When a `fn` expression is evaluated:

1. Create a function value containing:
   - The function name (for debugging and recursion)
   - The parameter list
   - The body expression (unevaluated)
   - A capture of the current environment
2. Bind the function to its name in the environment
3. Return `Unit`

The function captures the environment by reference, making it a true closure. The body is not evaluated until the function is called.

### Type

Function definition returns `Unit`. The function itself has type `fn(T1, T2, ...) -> R` where `T1, T2, ...` are parameter types and `R` is the return type.

### Test: Simple function definition

**Input:**

```cadenza
fn double x = x * 2
```

**Output:**

```repl
() : Unit
```

**Notes:** Function definitions return Unit, not the function value

### Test: Function definition and call

**Input:**

```cadenza
fn square x = x * x
square 5
```

**Output:**

```repl
() : Unit
25 : Integer
```

**Notes:** After defining the function, we can call it with an argument

### Test: Multi-parameter function

**Input:**

```cadenza
fn add x y = x + y
add 3 7
```

**Output:**

```repl
() : Unit
10 : Integer
```

**Notes:** Functions can take multiple parameters

### Test: Zero-parameter function

**Input:**

```cadenza
fn get_value = 42
get_value
```

**Output:**

```repl
() : Unit
42 : Integer
```

**Notes:** Functions with no parameters are automatically called when their name is referenced

### Test: Function with expression body

**Input:**

```cadenza
fn compute = 10 * 5 + 2
compute
```

**Output:**

```repl
() : Unit
52 : Integer
```

**Notes:** The body can be any expression, not just using parameters

---

## Function Application

Function application calls a function with arguments. Functions are called by writing the function name (or expression) followed by arguments.

### Syntax

```
<function> <arg1> <arg2> ...
```

The function can be an identifier, an expression that evaluates to a function, or an operator.

### Semantics

When a function application is evaluated:

1. Evaluate the function expression to get a function value
2. Evaluate each argument expression (left to right)
3. Create a new environment extending the function's captured environment
4. Bind each parameter to its corresponding argument
5. Evaluate the function body in the new environment
6. Return the result

### Test: Simple application

**Input:**

```cadenza
fn triple x = x * 3
triple 7
```

**Output:**

```repl
() : Unit
21 : Integer
```

### Test: Application with multiple arguments

**Input:**

```cadenza
fn multiply x y = x * y
multiply 6 7
```

**Output:**

```repl
() : Unit
42 : Integer
```

### Test: Nested application

**Input:**

```cadenza
fn add x y = x + y
fn double x = x * 2
add (double 3) (double 4)
```

**Output:**

```repl
() : Unit
() : Unit
14 : Integer
```

**Notes:** Arguments are expressions that can include function calls

### Test: Application as argument

**Input:**

```cadenza
fn square x = x * x
fn apply_twice f x = f (f x)
apply_twice square 3
```

**Output:**

```repl
() : Unit
() : Unit
81 : Integer
```

**Notes:** Functions can be passed as arguments (higher-order functions)

### Test: ERROR - Arity mismatch (too few arguments)

**Input:**

```cadenza
fn add x y = x + y
add 5
```

**Output:**

```
error: arity mismatch
 --> test.cdz:2:1
  |
2 | add 5
  | ^^^ function expects 2 arguments, got 1
  |
  = note: function `add` is defined with parameters: x, y
  = help: provide the missing argument(s)
```

### Test: ERROR - Arity mismatch (too many arguments)

**Input:**

```cadenza
fn double x = x * 2
double 5 10
```

**Output:**

```
error: arity mismatch
 --> test.cdz:2:1
  |
2 | double 5 10
  | ^^^^^^ function expects 1 argument, got 2
  |
  = note: function `double` is defined with parameter: x
  = help: remove the extra argument(s)
```

### Test: ERROR - Not a function

**Input:**

```cadenza
let x = 42
x 10
```

**Output:**

```
error: not callable
 --> test.cdz:2:1
  |
2 | x 10
  | ^ value of type Integer is not a function
  |
  = note: only functions can be called with arguments
```

---

## Closures

Functions capture their lexical environment at definition time. This means they can access variables from outer scopes even when called in a different context.

### Semantics

When a function is defined, it captures a reference to the current environment. When called, the function evaluates its body using this captured environment (extended with parameter bindings), not the caller's environment.

This enables functions to "close over" variables from their definition site.

### Test: Simple closure

**Input:**

```cadenza
let factor = 10
fn multiply_by_factor x = x * factor
multiply_by_factor 5
```

**Output:**

```repl
() : Unit
() : Unit
50 : Integer
```

**Notes:** The function captures `factor` from the outer scope

### Test: Closure with multiple captures

**Input:**

```cadenza
let a = 10
let b = 20
fn sum_with_context x = x + a + b
sum_with_context 5
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
35 : Integer
```

**Notes:** Functions can capture multiple variables

### Test: Closure preserves captured value

**Input:**

```cadenza
let x = 10
fn capture_fn = x
let x = 20
capture_fn
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
10 : Integer
```

**Notes:** The function captures the value at definition time. Later reassignments don't affect it.

### Test: Nested function (closure in closure)

**Input:**

```cadenza
fn outer x =
    fn inner y = x + y
    inner
let add_5 = outer 5
add_5 3
```

**Output:**

```repl
() : Unit
() : Unit
8 : Integer
```

**Notes:** Functions can return other functions, and the returned function captures variables from the outer function

---

## Closures and Reassignment

Closures capture variables by reference, not by value. This means if a captured variable is reassigned, all closures that captured it see the new value.

### Semantics

When a closure captures a variable:

1. It stores a reference to the variable's binding in the environment
2. When the closure is called, it reads the current value of that binding
3. If the binding is reassigned, the closure sees the new value
4. Multiple closures can share the same captured variable

This is different from capturing by value (copying), which would freeze the value at definition time.

### Test: Closure sees reassignment

**Input:**

```cadenza
let x = 10
fn get_x = x
get_x
x = 20
get_x
```

**Output:**

```repl
() : Unit
() : Unit
10 : Integer
() : Unit
20 : Integer
```

**Notes:** The function captures `x` by reference, so it sees the reassignment

### Test: Multiple closures share captured variable

**Input:**

```cadenza
let counter = 0
fn increment =
  counter += 1
fn get_counter = counter
increment
increment
get_counter
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
() : Unit
() : Unit
2 : Integer
```

**Notes:** Both functions capture the same `counter` variable, so `increment` affects what `get_counter` sees

### Test: Closure as mutable state

**Input:**

```cadenza
let state = 0
fn get_state = state
fn set_state new_val =
  state = new_val
set_state 42
get_state
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
() : Unit
42 : Integer
```

**Notes:** Closures can be used to create objects with mutable state

### Test: Reassignment in nested closure

**Input:**

```cadenza
let x = 1
fn outer =
    x = 10
    fn inner = x
    inner
let get_x = outer
get_x
x
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
10 : Integer
10 : Integer
```

**Notes:** The reassignment in `outer` affects the captured variable, visible to both `inner` and the outer scope

### Capture by Reference and Linear Types

With linear memory management, captured variables must be handled carefully:

- If a closure captures a linear type (String, List, etc.), it captures a reference
- Reassigning the captured variable transfers ownership and deletes the old value
- The closure then refers to the new value
- Multiple closures sharing a captured variable means they all share the same reference

### Test: Closure captures reference to linear type

**Input:**

```cadenza
let name = "Alice"
fn get_name = name
name = "Bob"
get_name
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
"Bob" : String
```

**Notes:** The old string "Alice" is freed when `name` is reassigned. The closure sees the new value.

---

## Recursion

Functions can call themselves by name, enabling recursive algorithms. The function name is bound in the environment before the body is evaluated, making recursion possible.

### Semantics

Function definitions are "hoisted" - the function name is available in its own body, allowing self-reference.

### Test: Simple recursion

**Input:**

```cadenza
fn factorial n =
    match n
        0 => 1
        n => n * factorial n - 1
factorial 5
```

**Output:**

```repl
() : Unit
120 : Integer
```

**Notes:** The function can call itself by name

### Test: Mutual recursion

**Input:**

```cadenza
fn is_even n =
    match n
        0 => true
        n => is_odd n - 1

fn is_odd n =
    match n
        0 => false
        n => is_even n - 1

is_even 4
```

**Output:**

```repl
() : Unit
() : Unit
true : Bool
```

**Notes:** Functions can call each other recursively due to hoisting

---

## Higher-Order Functions

Functions are first-class values, which means they can be passed as arguments, returned as results, and stored in data structures.

### Test: Function as argument

**Input:**

```cadenza
fn apply f x = f x
fn double x = x * 2
apply double 5
```

**Output:**

```repl
() : Unit
() : Unit
10 : Integer
```

**Notes:** Functions can accept other functions as parameters

### Test: Function as return value

**Input:**

```cadenza
fn make_adder n =
    fn adder x = x + n
    adder
let add_10 = make_adder 10
add_10 5
```

**Output:**

```repl
() : Unit
() : Unit
15 : Integer
```

**Notes:** Functions can return functions, creating closures with captured state

### Test: Function composition

**Input:**

```cadenza
fn compose f g =
    fn composed x = f (g x)
    composed

fn double x = x * 2
fn square x = x * x
let double_then_square = compose square double

double_then_square 3
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
() : Unit
36 : Integer
```

**Notes:** Functions can be composed to create new functions. Result: (3 \* 2)² = 36

---

## Function Values as Expressions

Functions can be referenced without calling them, treated as values.

### Test: Function value in variable

**Input:**

```cadenza
fn add x y = x + y
let op = add
op 1 2
```

**Output:**

```repl
() : Unit
() : Unit
3 : Integer
```

**Notes:** Functions can be assigned to variables and called through them

### Test: Operator as function value

**Input:**

```cadenza
let add_op = +
add_op 1 2
```

**Output:**

```repl
() : Unit
3 : Integer
```

**Notes:** Operators are functions and can be used as values

---

## Anonymous Functions

Functions without names can be defined with the following syntax.

### Syntax

```
\<param1> <param2> ... -> <body>
```

### Test: Anonymous function

**Input:**

```cadenza
let double = \x -> x * 2
double 5
```

**Output:**

```repl
() : Unit
10 : Integer
```

**Notes:** Anonymous functions allow defining functions inline without naming them

---

## Mutable Cells for Shared State

To enable shared mutable state in closures while maintaining linear memory safety, Cadenza uses explicit `Cell` types. A Cell is a reference-counted container that allows mutation. Conceptually it is similar to a `Arc<Mutex<T>>` in Rust.

### Syntax

```
Cell.new <expression>       # Create a Cell
Cell.get <cell>             # Read cell value
Cell.set <cell> <value>     # Write cell value
Cell.update <cell> fn(<value>) -> <value>
```

### Semantics

- `Cell.new expr` evaluates the expression and wraps the result in a reference-counted Cell
- `Cell.get c` reads the current value from the Cell (returns a reference)
- `Cell.set c v` replaces the Cell's value with a new value (frees the old value)
- Multiple closures can share a Cell, enabling shared mutable state
- The Cell is freed when all references to it are dropped

### Type

`Cell<T>` where T is the type of the contained value

### Test: Cell for mutable counter

**Input:**

```cadenza
let counter = Cell.new 0
fn increment = Cell.set counter (Cell.get counter) + 1
fn get_counter = cell_get counter
increment
increment
get_counter
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
() : Unit
() : Unit
2 : Integer
```

**Notes:** Multiple closures share the same Cell, enabling mutable state

### Test: Cell with linear type

**Input:**

```cadenza
let name_cell = Cell.new "Alice"
fn get_name = Cell.get name_cell
fn set_name new_name = Cell.set name_cell new_name
set_name "Bob"
get_name
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
() : Unit
"Bob" : String
```

**Notes:** The old string "Alice" is freed when the Cell is updated. Reference counting manages the Cell's lifetime.

### Benefits of Explicit Cells

1. **Opt-in mutability** - Regular variables are immutable unless wrapped in a Cell
2. **Clear ownership** - Cells use reference counting, other types use linear ownership
3. **No implicit sharing** - You must explicitly create a Cell to share mutable state
4. **Type system tracks** - `Cell<T>` vs `T` makes sharing visible in types

### Without Cells

Regular variable capture is immutable (capture by value):

**Input:**

```cadenza
let x = 10
fn get_x = x
x = 20
get_x
```

**Output:**

```repl
() : Unit
() : Unit
10 : Integer
() : Unit
10 : Integer
```

**Notes:** Without Cells, reassignment creates a new binding (shadowing). The closure captures the original value, not a reference. However, with each invocation the value is borrowed so it can be invoked multiple times.

### Borrowed Captures

**Input:**

```cadenza
let x = "hello"
fn len_of_x =
  String.len x
len_of_x
len_of_x
```

**Output:**

```repl
() : Unit
() : Unit
5 : Integer
5 : Integer
```

### Cloned Captures

**Input:**

```cadenza
let x = "hello"
fn cloned_x = *x
cloned_x
cloned_x
```

**Output:**

```repl
() : Unit
() : Unit
"hello" : String
"hello" : String
```

---

## Compiler Queries

Functions require:

- `eval(fn)` - Capture environment, create function value, bind name, return Unit
- `eval(apply)` - Evaluate function and arguments, extend captured environment, evaluate body
- `typeof(fn)` - Infer function type from parameters and body
- `typeof(apply)` - Check function type, check argument types, return result type
- `closure_capture(fn, env)` - Determine which variables to capture (by value)
- `lifetime(function)` - Track lifetime of captured variables
- `check_arity(fn, args)` - Verify argument count matches parameters
- `eval(cell)` - Create reference-counted cell
- `eval(cell_get)` - Read from cell
- `eval(cell_set)` - Write to cell, free old value

## Implementation Notes

- Functions are values containing: name, parameters, body, captured values (not references)
- Function application creates a new scope with parameter bindings
- Zero-arity functions are auto-applied when their name is referenced
- Recursion works because function name is bound before body evaluation (hoisting)
- Closures capture by value (immutable capture) - use Cells for mutable sharing
- Cells use reference counting for shared ownership
- Higher-order functions work naturally since functions are values
