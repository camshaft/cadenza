# Macros and Metaprogramming

Macros are compile-time functions that receive unevaluated syntax and return new syntax. They enable code generation, domain-specific languages, and compile-time computation.

## Macro Definition

Macros are defined like functions but operate on syntax trees rather than values. They receive expressions as arguments without evaluating them, and return expressions to be evaluated in place of the macro call.

### Syntax

```
macro <name> <param1> <param2> ... = <body>
```

The body receives syntax tree nodes and can construct new syntax to return.

### Semantics

When a `macro` expression is evaluated:

1. Create a macro value containing the name, parameters, and body
2. Register the macro in the compiler
3. Return `Unit`

Macros are expanded during evaluation - when a macro call is encountered, the macro is invoked with unevaluated arguments to produce new syntax.

### Type

Macro definitions return `Unit`. The macro itself has type `macro(Syntax, Syntax, ...) -> Syntax`.

### Test: Simple macro definition

**Input:**

```cadenza
macro double_it expr =
    quote 2 * ${expr}
```

**Output:**

```repl
() : Unit
() : Unit
10 : Integer
```

**Notes:** Macro definitions return Unit and register the macro for use

---

## Macro Invocation

Macros are called like functions, but their arguments are not evaluated before the call. The macro receives raw syntax trees.

### Semantics

When a macro is invoked:

1. Identify that the callee is a macro (not a function)
2. Pass unevaluated argument expressions to the macro
3. Execute the macro body (which is compile-time code)
4. The macro returns new syntax
5. Evaluate the returned syntax in place of the macro call

### Test: Macro invocation

**Input:**

```cadenza
macro double_it expr =
    quote 2 * ${expr}

double_it 10 + 5
```

**Output:**

```repl
() : Unit
30 : Integer
```

**Notes:** The macro receives `10 + 5` as syntax, returns `(* 2 (10 + 5))`, which evaluates to 30

---

## Quasiquotation

Quasiquotation provides a way to construct syntax trees with embedded expressions. The `quote` macro creates quoted syntax, `${expr}` splices syntax into the location.

### Syntax

```
quote <syntax>      # Quote: return syntax as-is
${<expression>}     # Unquote: evaluate and splice into quoted syntax
${...<expression>}  # Unquote-splicing: evaluate list and splice elements
```

### Semantics

- `quote` prevents evaluation and returns the syntax tree
- `${expr}` within a quasiquote evaluates an expression and splices its result
- `${...expr}` splices a list of syntax elements into the surrounding syntax

### Test: Simple quasiquotation

**Input:**

```cadenza
let x = 42
quote 1 + ${x}
```

**Output:**

```repl
() : Unit
Syntax (+ 1 42) : Syntax
```

**Notes:** The quasiquote constructs syntax, splicing in the value of `x`

### Test: Unquote-splicing

**Input:**

```cadenza
let args = [1, 2, 3]
quote (+ ${...args})
```

**Output:**

```repl
() : Unit
Syntax (+ 1 2 3) : Syntax
```

**Notes:** Unquote-splicing inserts list elements directly into the syntax

---

## Type Queries at Compile Time

The `typeof` operator returns the type of an expression as a first-class value. This enables compile-time branching based on types.

### Syntax

```
typeof <expression>
```

### Semantics

`typeof` evaluates to a `Type` value representing the expression's type. This happens at compile time - the expression is type-checked but not evaluated.

### Type

`typeof` returns a value of type `Type`.

### Test: typeof with literal

**Input:**

```cadenza
typeof 42
```

**Output:**

```repl
Integer : Type
```

**Notes:** Returns the type as a value

### Test: typeof with variable

**Input:**

```cadenza
let x = "hello"
typeof x
```

**Output:**

```repl
() : Unit
String : Type
```

### Test: typeof with function

**Input:**

```cadenza
fn double x = x * 2
typeof double
```

**Output:**

```repl
() : Unit
fn(Integer) -> Integer : Type
```

**Notes:** Returns the function's type signature

---

## Compile-Time Branching

Since types are first-class values and macros execute at compile time, you can branch on types to generate different code.

### Test: Type-based code generation

**Input:**

```cadenza
macro make_show ty =
    if (typeof ty) == Integer then
        quote (fn show_value x = String.from_int x)
    else
        quote (fn show_value x = "unknown")

make_show 42
show_value 100
```

**Output:**

```repl
() : Unit
() : Unit
"100" : String
```

**Notes:** The macro checks the type and generates appropriate code

### Test: Conditional function generation

**Input:**

```cadenza
let use_fast_path = true

macro make_compute =
    if use_fast_path then
        quote (fn compute x = x * 2)
    else
        quote (fn compute x = x + x)

make_compute
compute 10
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
20 : Integer
```

**Notes:** Macros can generate different implementations based on compile-time conditions

---

## Iteration and Multiple Definitions

Macros can iterate over lists to generate multiple definitions. This is powerful for eliminating boilerplate.

### Test: Generate multiple similar functions

**Input:**

```cadenza
for num in [2, 3, 5, 10]
    let name = :"mul_by_${String.from_int num}"
    fn ${name} x = x * num

mul_by_2 7
mul_by_5 4
```

**Output:**

```repl
() : Unit
14 : Integer
20 : Integer
```

**Notes:** The loop generates `mul_by_2`, `mul_by_3`, `mul_by_5`, and `mul_by_10` functions

---

## Type Reflection

Types are first-class values that can be inspected and decomposed at compile time. This enables generating code based on type structure.

### Type.fields

Get the list of fields from a record or struct type.

#### Syntax

```
Type.fields <type>
```

#### Returns

A list of field names and their types: `[(Symbol, Type)]`

### Test: Get fields from record type

**Input:**

```cadenza
let person_ty = typeof { name = "Alice", age = 30 }
Type.fields person_ty
```

**Output:**

```repl
() : Unit
[(:"name", String), (:"age", Integer)] : [(Symbol, Type)]
```

**Notes:** Returns a list of (field_name, field_type) tuples

### Test: Generate accessor functions from type

**Input:**

```cadenza
let record_ty = typeof { name = "Alice", age = 30, email = "alice@example.com" }
for (field_name, field_ty) <- Type.fields record_ty
    let accessor_name = :"get_${field_name}"
    fn ${accessor_name} record = record.${field_name}

let person = { name = "Bob", age = 25, email = "bob@example.com" }
get_name person
get_age person
get_email person
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
() : Unit
"Bob" : String
25 : Integer
"bob@example.com" : String
```

**Notes:** Iterating over type fields generates accessor functions automatically

### Type.variant_of

Check if a type is a specific variant (for enums/unions).

#### Syntax

```
Type.variant_of <type> <variant_name>
```

#### Returns

`Bool` - true if the type is that variant

### Test: Check type variant

**Input:**

```cadenza
let num_ty = typeof 42
Type.variant_of num_ty :"Integer"
```

**Output:**

```repl
() : Unit
true : Bool
```

### Type.name

Get the name of a nominal type (struct).

#### Syntax

```
Type.name <type>
```

#### Returns

`String` - the type's name, or error if not a nominal type

### Test: Get struct type name

**Input:**

```cadenza
struct Point { x = Integer, y = Integer }
let p = Point { x = 10, y = 20 }
Type.name (typeof p)
```

**Output:**

```repl
() : Unit
() : Unit
:"Point" : Symbol
```

---

## Syntax Construction

Beyond quasiquotation, macros can construct syntax programmatically using explicit construction functions.

### Syntax.apply

Construct a function application.

#### Syntax

```
Syntax.apply <function_syntax> [<arg_syntax>, ...]
```

### Test: Build syntax programmatically

**Input:**

```cadenza
let fn_name = :"double"
let arg = Syntax.literal 5
Syntax.apply fn_name [arg]
```

**Output:**

```repl
() : Unit
() : Unit
Syntax (double 5) : Syntax
```

**Notes:** Constructs syntax trees without quasiquotation

### Test: Build nested syntax

**Input:**

```cadenza
macro make_pipeline funcs init =
    List.fold funcs init \acc fn_syntax ->
        Syntax.apply fn_syntax [acc]

make_pipeline [double, square, double] 3
```

**Output:**

```repl
() : Unit
72 : Integer
```

**Notes:** Constructs `double (square (double 3))` from the list. Result: 3 _ 2 = 6, 6² = 36, 36 _ 2 = 72

---

## Compile-Time Computation

Macros execute arbitrary Cadenza code at compile time. This includes calling functions, computing values, and building data structures.

### Test: Compile-time calculation

**Input:**

```cadenza
macro make_constants =
    for i in [1, 2, 3, 4, 5]
        let name = :"CONST_${String.from_int i}"
        let value = i * i
        fn ${name} = ${value}

make_constants
CONST_3
CONST_5
```

**Output:**

```repl
() : Unit
9 : Integer
25 : Integer
```

**Notes:** The squares are computed at compile time, not runtime

---

## Macro Hygiene

Cadenza macros are hygienic by default, preventing accidental variable capture. Variables introduced by macros are automatically scoped to avoid conflicts with user code.

### Automatic Hygiene

The compiler automatically namespaces variables introduced by macros to ensure they don't conflict with variables at the use site. This happens transparently without any extra syntax.

#### Semantics

When a macro introduces a variable:

1. The compiler generates a unique name for the variable
2. All references to that variable within the macro are namespaced
3. The unique name is guaranteed not to conflict with user code
4. User variables at the use site are unaffected

### Test: Automatic variable scoping

**Input:**

```cadenza
macro swap_and_add x y =
    quote
        let temp = ${x}
        ${x} = ${y}
        ${y} = temp
        ${x} + ${y}

let a = 10
let b = 20
swap_and_add a b
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
30 : Integer
```

**Notes:** The `temp` variable is automatically scoped to prevent conflicts

### Test: Macro variable doesn't leak

**Input:**

```cadenza
macro double_with_temp x =
    quote
        let temp = ${x} * 2
        temp

double_with_temp 5
temp
```

**Output:**

```
() : Unit
10 : Integer
error: undefined variable: `temp`
 --> test.cdz:7:1
  |
7 | temp
  | ^^^^ not found in this scope
  |
  = note: `temp` is a macro-internal variable and not visible here
```

**Notes:** Variables introduced by macros are scoped to the macro expansion

### Test: Multiple macro calls don't conflict

**Input:**

```cadenza
macro with_counter body =
    quote
        let count = 0
        ${body}
        count

let result1 = with_counter (quote count = count + 1)
let result2 = with_counter (quote count = count + 5)
result1
result2
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
1 : Integer
5 : Integer
```

**Notes:** Each macro invocation gets its own unique `count` variable

### Opting Out of Hygiene

Sometimes you want a macro to deliberately introduce variables into the calling scope. Use the `unhygienic` attribute to opt out.

#### Syntax

```
@unhygienic
macro <name> <params> = <body>
```

### Test: Unhygienic macro introduces binding

**Input:**

```cadenza
@unhygienic
macro with_result body =
    quote
        let result = ${body}
        result

with_result (10 + 5)
result
```

**Output:**

```repl
() : Unit
15 : Integer
15 : Integer
```

**Notes:** The `unhygienic` attribute makes `result` visible at the use site

### Captured vs Use-Site Variables

Macros can distinguish between variables from their definition site (captured) and variables at the use site (passed as arguments).

### Test: Macro using captured variable

**Input:**

```cadenza
let factor = 10
macro multiply_by_factor expr =
    quote ${factor} * ${expr}

multiply_by_factor 5
let factor = 20
multiply_by_factor 5
```

**Output:**

```repl
() : Unit
() : Unit
50 : Integer
() : Unit
50 : Integer
```

**Notes:** The macro captures `factor` from its definition site. Redefining `factor` doesn't affect the macro.

### Test: Macro receives use-site syntax

**Input:**

```cadenza
macro use_var var_name =
    quote ${var_name} + 1

let x = 10
use_var x
let x = 20
use_var x
```

**Output:**

```repl
() : Unit
() : Unit
11 : Integer
() : Unit
21 : Integer
```

**Notes:** The macro receives the identifier `x` as syntax, which resolves at the use site

---

## Compiler Queries

Macros and metaprogramming require:

- `eval(macro)` - Register macro in compiler, return Unit
- `eval(macro_call)` - Invoke macro with unevaluated args, evaluate result
- `eval(quote)` - Construct syntax tree with splicing
- `eval(typeof)` - Return type of expression as Type value
- `typeof_compile_time(expr)` - Type-check without evaluating
- `Type.fields(ty)` - Get record/struct fields
- `Type.variant_of(ty, name)` - Check type variant
- `Type.name(ty)` - Get nominal type name
- `Syntax.apply(fn, args)` - Construct function application
- `Syntax.ident(name)` - Construct identifier
- `Syntax.literal(value)` - Construct literal

## Implementation Notes

- Macro expansion happens during evaluation phase
- Macros return syntax trees that are then evaluated
- `typeof` performs type inference without evaluation
- Quasiquotation constructs Expr nodes directly
- Type reflection accesses type structure at compile time
- Recursive macros are allowed (macro can invoke itself)
- All Cadenza features available at compile time (it's the same language)
