# Variables and Bindings

Variables bind names to values, allowing computed results to be stored and reused. Cadenza uses lexical scoping where visibility is determined by source code structure.

## Identifiers

Identifiers are names that refer to values. They form the vocabulary for naming bindings, functions, types, and other entities in the language.

### Syntax

```
[a-zA-Z_][a-zA-Z0-9_-]*
```

An identifier must:

- Start with a letter (a-z, A-Z) or underscore (\_)
- Continue with letters, digits (0-9), underscores (\_), or hyphens (-)
- Be case-sensitive: `foo`, `Foo`, and `FOO` are distinct

### Evaluation

When an identifier is evaluated, its name is looked up in the environment. The search proceeds from the innermost scope to the outermost scope, and the first matching binding is returned.

### Type

An identifier has the same type as the value it refers to.

### Test: Simple identifier lookup

**Input:**

```cadenza
let name = "Alice"
name
```

**Output:**

```repl
() : Unit
"Alice" : String
```

**Notes:** The identifier `name` evaluates to the value bound to it

### Test: Identifier with underscores

**Input:**

```cadenza
let _private_var = 42
_private_var
```

**Output:**

```repl
() : Unit
42 : Integer
```

**Notes:** Identifiers can start with underscore, following Rust convention for unused or private bindings

### Test: Case sensitivity

**Input:**

```cadenza
let foo = 1
let Foo = 2
let FOO = 3
foo
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
1 : Integer
```

**Notes:** Each differently-cased name creates a separate binding

### Test: ERROR - Undefined variable

**Input:**

```cadenza
undefined_variable
```

**Output:**

```
error: undefined variable: `undefined_variable`
 --> test.cdz:1:1
  |
1 | undefined_variable
  | ^^^^^^^^^^^^^^^^^^ not found in this scope
  |
  = help: use `let undefined_variable = ...` to define it
```

### Test: ERROR - Typo in variable name

**Input:**

```cadenza
let value = 10
vlaue
```

**Output:**

```
error: undefined variable: `vlaue`
 --> test.cdz:2:1
  |
2 | vlaue
  | ^^^^^ not found in this scope
  |
  = help: did you mean `value`?
```

**Notes:** The compiler can suggest similar names for typos

---

## Patterns

Patterns appear on the left-hand side of bindings and describe how to decompose values and bind their parts to names.

### Simple Identifier Pattern

The simplest pattern is just an identifier, which matches any value and binds it to that name.

#### Syntax

```
<identifier>
```

#### Semantics

An identifier pattern always succeeds and binds the value to the given name.

### Test: Identifier pattern

**Input:**

```cadenza
let x = 42
x
```

**Output:**

```repl
() : Unit
42 : Integer
```

**Notes:** The pattern `x` matches the value `42` and binds it

### Tuple Pattern

A tuple pattern destructures a tuple value into its elements.

#### Syntax

```
(<pattern1>, <pattern2>, ...)
```

#### Semantics

A tuple pattern matches a tuple value if:

- The tuple has the same number of elements as the pattern
- Each element matches its corresponding sub-pattern
- All sub-patterns succeed

When matched, each sub-pattern creates its bindings.

### Test: Tuple destructuring

**Input:**

```cadenza
let (x, y) = (10, 20)
x
y
```

**Output:**

```repl
() : Unit
10 : Integer
20 : Integer
```

**Notes:** The pattern `(x, y)` destructures the tuple and binds `x` to 10 and `y` to 20

### Test: Nested tuple pattern

**Input:**

```cadenza
let (a, (b, c)) = (1, (2, 3))
a + b + c
```

**Output:**

```repl
() : Unit
6 : Integer
```

**Notes:** Patterns can be nested to destructure nested tuples

### Test: ERROR - Tuple pattern arity mismatch

**Input:**

```cadenza
let (x, y) = (1, 2, 3)
```

**Output:**

```
error: pattern match failed
 --> test.cdz:1:5
  |
1 | let (x, y) = (1, 2, 3)
  |     ^^^^^^   --------- this tuple has 3 elements
  |     |
  |     this pattern expects 2 elements
  |
  = note: tuple patterns must match the number of elements
```

### Record Pattern

A record pattern destructures a record value into its fields.

#### Syntax

```
{<field1>, <field2>, ...}              # Field shorthand
{<field1> = <pattern1>, ...}           # Full form
{<field1>, <field2>, ...}              # Partial destructuring (ignores other fields)
```

#### Semantics

A record pattern matches a record value if all named fields exist. The pattern can be partial - it doesn't need to name all fields in the record.

### Test: Record destructuring

**Input:**

```cadenza
let { x, y } = { x = 10, y = 20 }
x + y
```

**Output:**

```repl
() : Unit
30 : Integer
```

**Notes:** Field shorthand `{ x, y }` is equivalent to `{ x = x, y = y }`

### Test: Record destructuring with nested patterns

**Input:**

```cadenza
let { x = a, y = b } = { x = 10, y = 20 }
a + b
```

**Output:**

```repl
() : Unit
30 : Integer
```

**Notes:** Fields can be bound to different names using the full form

### Test: Partial record destructuring

**Input:**

```cadenza
let { x, ... } = { x = 10, y = 20, z = 30 }
x
```

**Output:**

```repl
() : Unit
10 : Integer
```

### Test: Partial record splitting

**Input:**

```cadenza
let { x, ...rest } = { x = 10, y = 20, z = 30 }
x
rest
```

**Output:**

```repl
() : Unit
10 : Integer
{ y = 20, z = 30 } : { y = Integer, z = Integer }
```

**Notes:** The pattern only needs to match the fields it names; other fields are ignored

### Test: ERROR - Record pattern missing field

**Input:**

```cadenza
let { x, y, z } = { x = 10, y = 20 }
```

**Output:**

```
error: pattern match failed
 --> test.cdz:1:5
  |
1 | let {x, y, z} = {x = 10, y = 20}
  |            ^    ---------------- this record has fields: x, y
  |            |
  |            field `z` not found in record
  |
  = note: record patterns can only destructure fields that exist
```

### Wildcard Pattern

The wildcard pattern `_` matches any value and discards it without binding.

#### Syntax

```
_
```

#### Semantics

The wildcard always matches and creates no bindings. It's used when you need to match a value but don't need to use it.

### Test: Wildcard in tuple

**Input:**

```cadenza
let (x, _) = (10, 20)
x
```

**Output:**

```repl
() : Unit
10 : Integer
```

**Notes:** The second element is matched but not bound

### Test: Multiple wildcards

**Input:**

```cadenza
let (_, x, _) = (1, 2, 3)
x
```

**Output:**

```repl
() : Unit
2 : Integer
```

**Notes:** Only the middle element is bound

---

## Infallible vs Fallible Patterns

Patterns are classified into two categories based on whether they can fail to match.

### Infallible Patterns

**Infallible patterns** always succeed and can be used in `let` bindings. These include:

- Identifier patterns: `x`
- Tuple patterns: `(x, y)` - fails at compile time if sizes don't match
- Record patterns: `{ x, y }` - fails at compile time if fields don't exist
- Wildcard patterns: `_`

These patterns are safe because any mismatch is detected at compile time through type checking.

### Fallible Patterns

**Fallible patterns** can fail to match at runtime and are **only allowed in `match` expressions**, not in `let` bindings. These include:

- List patterns: `[x, y]` - runtime length may not match
- Literal patterns: `42`, `"hello"` - runtime value may differ

The distinction exists to prevent runtime panics. If you need to destructure a list or match against specific values, use a `match` expression which handles failure by trying alternative patterns.

### Test: ERROR - List pattern in let

**Input:**

```cadenza
let [x, y] = my_list
```

**Output:**

```
error: fallible pattern in let binding
 --> test.cdz:1:5
  |
1 | let [x, y] = my_list
  |     ^^^^^^ this pattern can fail to match
  |
  = note: list patterns are fallible because list length is not known at compile time
  = note: let bindings require infallible patterns to prevent runtime panics
  = help: use a match expression to handle different list lengths:
  = help:   match my_list
  = help:     [x, y] => ...
  = help:     _ => ...
```

**Notes:** List patterns can only appear in `match` expressions where failure can be handled

### Test: ERROR - Literal pattern in let

**Input:**

```cadenza
let 42 = x
```

**Output:**

```
error: fallible pattern in let binding
 --> test.cdz:1:5
  |
1 | let 42 = x
  |     ^^ this pattern can fail to match
  |
  = note: literal patterns only match specific values
  = note: let bindings require infallible patterns to prevent runtime panics
  = help: use an identifier pattern: `let n = x`
  = help: then test the value: `n == 42`
```

**Notes:** Literal patterns are for matching in control flow, not for binding

### Why This Distinction?

This design choice prevents runtime panics while keeping the language predictable:

1. **No implicit failure**: `let` bindings always succeed (or fail at compile time)
2. **Explicit error handling**: Use `match` when patterns might not match
3. **Clear intent**: `let` is for binding, `match` is for conditional logic

---

## Let Bindings

The `let` keyword creates a binding between a pattern and a value. It evaluates the expression, matches the result against the pattern, and creates bindings for the pattern's identifiers.

### Syntax

```
let <pattern> = <expression>
```

### Semantics

When a `let` expression is evaluated:

1. Evaluate the right-hand expression to produce a value
2. Match the value against the pattern
3. Bind pattern identifiers to (parts of) the value
4. Return `Unit` (the empty tuple `()`)

The `let` form returns `Unit` rather than the bound value because in a language with linear memory management, returning the value would require cloning it (since it's already bound to the identifier). By returning `Unit`, we avoid implicit copies.

### Type

A `let` expression always has type `Unit`, regardless of the bound expression's type.

### Test: Simple let binding returns Unit

**Input:**

```cadenza
let x = 42
```

**Output:**

```repl
() : Unit
```

**Notes:** The `let` evaluates to Unit, not the bound value. Use the identifier to access the value.

### Test: Let binding with identifier usage

**Input:**

```cadenza
let x = 42
x
```

**Output:**

```repl
() : Unit
42 : Integer
```

**Notes:** After binding, use the identifier to access the value

### Test: Let with arithmetic expression

**Input:**

```cadenza
let result = 10 + 5
result
```

**Output:**

```repl
() : Unit
15 : Integer
```

### Test: Multiple sequential bindings

**Input:**

```cadenza
let x = 1
let y = 2
x + y
```

**Output:**

```repl
() : Unit
() : Unit
3 : Integer
```

**Notes:** Each `let` returns Unit, only the final expression returns the sum

### Test: Let binding using previous binding

**Input:**

```cadenza
let x = 10
let y = x * 2
y
```

**Output:**

```repl
() : Unit
() : Unit
20 : Integer
```

**Notes:** The expression in a `let` can reference earlier bindings

### Test: ERROR - Pattern mismatch

**Input:**

```cadenza
let 42 = x
```

**Output:**

```
error: expected pattern, found literal
 --> test.cdz:1:5
  |
1 | let 42 = x
  |     ^^ literals cannot be used as patterns
  |
  = note: patterns must be identifiers or destructuring forms
  = help: use `x == 42` to test equality instead
```

**Notes:** Currently only identifier patterns are supported; literals and other forms are errors

---

## Shadowing

Shadowing occurs when a new binding is created with the same name as an existing binding. The new binding hides the old one within its scope.

### Semantics

When a binding shadows another:

1. A new binding is created in the current scope
2. The identifier now refers to the new binding
3. The old binding is unchanged but inaccessible
4. When the shadowing scope ends, the old binding becomes visible again

Shadowing is distinct from mutation - it creates a new binding rather than modifying an existing one.

### Test: Basic shadowing

**Input:**

```cadenza
let x = 1
let x = 2
x
```

**Output:**

```repl
() : Unit
() : Unit
2 : Integer
```

**Notes:** The second `let x` creates a new binding that shadows the first

### Test: Shadowing with different types

**Input:**

```cadenza
let x = 42
let x = "hello"
x
```

**Output:**

```repl
() : Unit
() : Unit
"hello" : String
```

**Notes:** Shadowing can change the type since it creates an entirely new binding

### Test: Shadowing preserves outer binding

**Input:**

```cadenza
let x = 1
let outer_result =
    let x = 2
    x
outer_result
x
```

**Output:**

```repl
() : Unit
() : Unit
2 : Integer
1 : Integer
```

**Notes:** After the inner scope ends, the original `x = 1` binding is visible again

### Test: Shadowing uses previous value

**Input:**

```cadenza
let counter = 0
let counter = counter + 1
let counter = counter + 1
counter
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
2 : Integer
```

**Notes:** Each new binding can reference the previous binding with the same name

---

## Lexical Scoping

Cadenza uses lexical (static) scoping where binding visibility is determined by source code structure, not execution flow.

### Semantics

The environment is a stack of scopes:

- Global scope contains top-level bindings
- Each block or function creates a new scope
- Lookup searches from innermost to outermost scope
- Inner scopes can access outer bindings
- Outer scopes cannot access inner bindings

### Test: Inner scope accesses outer

**Input:**

```cadenza
let outer = 100
let result =
    let inner = 200
    inner + outer
result
```

**Output:**

```repl
() : Unit
300 : Integer
300 : Integer
```

**Notes:** The inner block can read `outer` from the enclosing scope

### Test: Multiple nesting levels

**Input:**

```cadenza
let a = 1
let level1 =
    let b = 2
    let level2 =
        let c = 3
        a + b + c
    level2
level1
```

**Output:**

```repl
() : Unit
() : Unit
6 : Integer
```

**Notes:** Each nested scope can access all outer bindings

### Test: ERROR - Inner binding not visible outside

**Input:**

```cadenza
let outer =
    let inner = 10
    inner
inner
```

**Output:**

```
error: undefined variable: `inner`
 --> test.cdz:4:1
  |
4 | inner
  | ^^^^^ not found in this scope
  |
  = note: `inner` is defined inside a block and not visible here
  = help: move the binding outside the block if you need it here
```

---

## Reassignment

The `=` operator without `let` modifies an existing binding's value. Unlike shadowing (which creates a new binding), reassignment mutates the existing binding in place.

### Syntax

```
<identifier> = <expression>
```

### Semantics

When a reassignment is evaluated:

1. Look up the existing binding for the identifier
2. Evaluate the right-hand expression to produce a new value
3. Replace the binding's current value with the new value
4. Return `Unit`

Reassignment requires that the identifier already exists - you cannot create new bindings with `=` alone.

### Type

Reassignment always returns `Unit`. The new value's type should be compatible with the variable's declared or inferred type (when type system is implemented).

### Test: Simple reassignment

**Input:**

```cadenza
let x = 1
x = 2
x
```

**Output:**

```repl
() : Unit
() : Unit
2 : Integer
```

**Notes:** Reassignment modifies the existing binding created by `let`

### Test: Reassignment with expression

**Input:**

```cadenza
let counter = 0
counter = counter + 1
counter
```

**Output:**

```repl
() : Unit
() : Unit
1 : Integer
```

**Notes:** The right-hand side can reference the current value of the variable being reassigned

### Test: Multiple reassignments

**Input:**

```cadenza
let x = 1
x = 2
x = 3
x = 4
x
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
() : Unit
4 : Integer
```

**Notes:** A variable can be reassigned multiple times

### Test: Reassignment in nested scope

**Input:**

```cadenza
let x = 1
let result =
    x = 2
    x
x
```

**Output:**

```repl
() : Unit
2 : Integer
2 : Integer
```

**Notes:** Reassignment modifies the binding from the outer scope. Unlike shadowing, the modification persists after the inner scope ends.

### Test: Reassignment vs shadowing

**Input:**

```cadenza
let x = 1
let outer =
    let x = 10
    x
let inner =
    x = 2
    x
outer
inner
x
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
10 : Integer
2 : Integer
2 : Integer
```

**Notes:** The shadowing (`let x = 10`) doesn't affect the outer `x`, but reassignment (`x = 2`) does modify it

### Test: ERROR - Reassignment of undefined variable

**Input:**

```cadenza
x = 42
```

**Output:**

```
error: undefined variable: `x`
 --> test.cdz:1:1
  |
1 | x = 42
  | ^ not found in this scope
  |
  = note: reassignment requires an existing binding
  = help: use `let x = 42` to create a new binding
```

**Notes:** You must use `let` to create a binding before you can reassign it

### Reassignment and Linear Types

Reassignment interacts with linear memory management in important ways:

When reassigning a variable that holds a linear type (String, List, etc.):

1. The old value is automatically freed (delete called)
2. The new value is moved into the binding
3. No implicit cloning occurs

This makes reassignment efficient and memory-safe without requiring explicit cleanup.

### Test: Reassignment frees old value

**Input:**

```cadenza
let s = "hello"
s = "world"
s
```

**Output:**

```repl
() : Unit
() : Unit
"world" : String
```

**Notes:** The old string "hello" is automatically freed when `s` is reassigned. This prevents memory leaks.

---

## Compiler Queries

Variables and bindings require:

- `eval(let)` - Evaluate expression, match pattern, extend environment, return Unit
- `eval(ident)` - Look up identifier in environment
- `eval(assign)` - Find binding, evaluate expression, update binding, return Unit
- `lookup_var(name, env)` - Search scope stack for binding
- `typeof(let)` - Always returns Unit type
- `typeof(ident)` - Look up identifier type in type environment
- `pattern_match(pattern, value)` - Match value against pattern, extract bindings
- `lifetime(binding)` - Track lifetime for memory management

## Implementation Notes

- Environment is a stack of maps (scope → name → value)
- `let` adds a binding to the current (top) scope
- Identifier lookup is a linear search from top to bottom of scope stack
- Shadowing works automatically - lookup finds innermost binding first
- Assignment searches for existing binding and modifies its value
- Returning Unit from `let` avoids implicit cloning for linear types
