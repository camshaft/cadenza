# Compound Types

Compound types group multiple values together into structured data. Cadenza provides tuples, records, structs, and enums for different use cases.

## Tuples

Tuples are ordered collections of values accessed by position. They're useful for grouping related values temporarily.

### Syntax

```
(<expr1>, <expr2>, ...)
()                          # Empty tuple (Unit)
(<expr>,)                   # Single-element tuple (trailing comma required)
```

### Semantics

Tuple expressions evaluate each element in order and construct a tuple value containing the results.

### Type

`(<T1>, <T2>, ...)` where each `T` is the type of the corresponding element.

The empty tuple `()` has type `Unit` and serves as the "no value" type.

### Test: Simple tuple

**Input:**

```cadenza
(1, 2, 3)
```

**Output:**

```repl
(1, 2, 3) : (Integer, Integer, Integer)
```

### Test: Empty tuple (Unit)

**Input:**

```cadenza
()
```

**Output:**

```repl
() : Unit
```

**Notes:** The empty tuple is Unit, the type of expressions that return nothing meaningful

### Test: Single-element tuple

**Input:**

```cadenza
(42,)
```

**Output:**

```repl
(42,) : (Integer,)
```

**Notes:** Trailing comma required to distinguish from parenthesized expressions

### Test: Tuple with different types

**Input:**

```cadenza
("Alice", 30, true)
```

**Output:**

```repl
("Alice", 30, true) : (String, Integer, Bool)
```

**Notes:** Tuples can contain values of different types (heterogeneous)

### Test: Nested tuples

**Input:**

```cadenza
((1, 2), (3, 4))
```

**Output:**

```repl
((1, 2), (3, 4)) : ((Integer, Integer), (Integer, Integer))
```

### Test: Tuple from expressions

**Input:**

```cadenza
let x = 10
let y = 20
(x, y, x + y)
```

**Output:**

```repl
() : Unit
() : Unit
(10, 20, 30) : (Integer, Integer, Integer)
```

---

## Records

Records are collections of named fields. They use structural typing - two records with the same fields have the same type.

### Syntax

```
{ <field1> = <expr1>, <field2> = <expr2>, ... }
{ <field1>, <field2>, ... }                      # Field shorthand
{}                                               # Empty record
```

Field shorthand `{ x, y }` is equivalent to `{ x = x, y = y }`.

### Semantics

Record expressions evaluate each field value and construct a record. Field order is preserved but doesn't affect equality.

### Type

`{ <field1>: <T1>, <field2>: <T2>, ... }` where each `T` is the field's type.

### Test: Simple record

**Input:**

```cadenza
{ x = 10, y = 20 }
```

**Output:**

```repl
{ x = 10, y = 20 } : { x: Integer, y: Integer }
```

### Test: Empty record

**Input:**

```cadenza
{}
```

**Output:**

```repl
{} : {}
```

### Test: Record with field shorthand

**Input:**

```cadenza
let x = 1
let y = 2
{ x, y }
```

**Output:**

```repl
() : Unit
() : Unit
{ x = 1, y = 2 } : { x: Integer, y: Integer }
```

**Notes:** Field shorthand automatically uses variable names as field names

### Test: Record with mixed notation

**Input:**

```cadenza
let name = "Alice"
{ name, age = 30, active = true }
```

**Output:**

```repl
() : Unit
{ name = "Alice", age = 30, active = true } : { name: String, age: Integer, active: Bool }
```

**Notes:** Can mix shorthand and explicit field assignment

### Test: Record with computed values

**Input:**

```cadenza
{ x = 2 + 3, y = 10 * 5 }
```

**Output:**

```repl
{ x = 5, y = 50 } : { x: Integer, y: Integer }
```

### Test: Nested records

**Input:**

```cadenza
{ point = { x = 10, y = 20 }, label = "origin" }
```

**Output:**

```repl
{ point = { x = 10, y = 20 }, label = "origin" } : { point: { x: Integer, y: Integer }, label: String }
```

---

## Field Access

Fields are accessed using dot notation.

### Syntax

```
<record>.<field_name>
```

### Semantics

Evaluate the record expression, then extract the named field's value.

### Test: Simple field access

**Input:**

```cadenza
let point = { x = 10, y = 20 }
point.x
```

**Output:**

```repl
() : Unit
10 : Integer
```

### Test: Field access on expression

**Input:**

```cadenza
fn make_point x y = { x, y }
(make_point 5 10).x
```

**Output:**

```repl
() : Unit
5 : Integer
```

**Notes:** Can access fields on any expression that evaluates to a record

### Test: Chained field access

**Input:**

```cadenza
let nested = { inner = { value = 42 } }
nested.inner.value
```

**Output:**

```repl
() : Unit
42 : Integer
```

### Test: ERROR - Missing field

**Input:**

```cadenza
let point = { x = 10, y = 20 }
point.z
```

**Output:**

```
error: field not found
 --> test.cdz:2:7
  |
2 | point.z
  |       ^ field `z` does not exist
  |
  = help: available fields are: x, y
```

### Test: ERROR - Field access on non-record

**Input:**

```cadenza
let x = 42
x.field
```

**Output:**

```
error: cannot access field on non-record type
 --> test.cdz:2:1
  |
2 | x.field
  | ^ value of type Integer has no fields
  |
  = note: field access requires a record or struct type
```

---

## Structs

Structs are nominally-typed records. Unlike records (which use structural typing), two structs with the same fields but different names are different types.

### Syntax

```
struct <Name> { <field1> = <Type1>, <field2> = <Type2>, ... }
```

### Semantics

A struct definition:

1. Creates a new nominal type with the given name
2. Registers a constructor function with that name
3. Returns `Unit`

The constructor takes a record of field values and creates a struct instance.

### Type

Struct definitions return `Unit`. The struct itself is a new nominal type, and the constructor has type `fn({ fields... }) -> StructName`.

### Test: Struct definition

**Input:**

```cadenza
struct Point { x = Integer, y = Integer }
```

**Output:**

```repl
() : Unit
```

### Test: Struct instance creation

**Input:**

```cadenza
struct Point { x = Integer, y = Integer }
let p = Point { x = 10, y = 20 }
p.x
```

**Output:**

```repl
() : Unit
() : Unit
10 : Integer
```

### Test: Structs are nominally typed

**Input:**

```cadenza
struct Point { x = Integer, y = Integer }
struct Vector { x = Integer, y = Integer }
let p = Point { x = 1, y = 2 }
let v = Vector { x = 1, y = 2 }
p == v
```

**Output:**

```
error: type mismatch
 --> test.cdz:5:1
  |
5 | p == v
  | ^^^^^^ cannot compare Point with Vector
  |
  = note: Point and Vector are different nominal types
  = note: even though they have the same fields, nominal typing makes them distinct
```

**Notes:** Structural similarity doesn't make types equal with nominal typing

---

## Converting Structs to Records

Structs can be converted to their underlying structural record type, removing the nominal wrapper. This is useful when you want structural typing for a specific operation.

### Syntax

```
Record.from_struct <struct_value>
```

### Semantics

`Record.from_struct` takes a struct instance and returns a record with the same field values, but with structural typing instead of nominal typing.

### Test: Convert struct to record

**Input:**

```cadenza
struct Point { x = Integer, y = Integer }
let p = Point { x = 10, y = 20 }
let r = Record.from_struct p
typeof r
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
{ x: Integer, y: Integer } : Type
```

**Notes:** The nominal Point type becomes a structural record type

### Test: Structural equality after conversion

**Input:**

```cadenza
struct Point { x = Integer, y = Integer }
struct Vector { x = Integer, y = Integer }
let p = Point { x = 1, y = 2 }
let v = Vector { x = 1, y = 2 }
(Record.from_struct p) == (Record.from_struct v)
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
() : Unit
true : Bool
```

**Notes:** After converting to records, they're structurally equal

---

## Record Splitting

Records can be split into selected fields and a "rest" record containing the remaining fields. This is useful for extracting specific fields while preserving others.

### Syntax in Patterns

```
{ <field1>, ...rest }
{ <field1>, <field2>, ...rest }
```

This was shown in the variables section for patterns. Here we show it with values.

### Record.split Function

Split a record programmatically at runtime.

#### Syntax

```
Record.split <record> [<field_names>]
```

#### Returns

A tuple: `(<extracted_record>, <rest_record>)`

### Test: Split record with pattern

**Input:**

```cadenza
let person = { name = "Alice", age = 30, email = "alice@example.com", active = true }
let { name, age, ...rest } = person
name
rest
```

**Output:**

```repl
() : Unit
() : Unit
"Alice" : String
{ email = "alice@example.com", active = true } : { email: String, active: Bool }
```

**Notes:** Pattern matching splits the record, binding selected fields and the rest

### Test: Programmatic record split

**Input:**

```cadenza
let person = { name = "Alice", age = 30, email = "alice@example.com" }
let (selected, rest) = Record.split person [:"name", :"age"]
selected
rest
```

**Output:**

```repl
() : Unit
() : Unit
{ name = "Alice", age = 30 } : { name: String, age: Integer }
{ email = "alice@example.com" } : { email: String }
```

**Notes:** Programmatic split allows compile-time field selection. The fields must be known at compile time.

### Test: Record merge

**Input:**

```cadenza
let base = { x = 10, y = 20 }
let extra = { z = 30, label = "point" }
{ ...base, ...extra }
```

**Output:**

```repl
() : Unit
() : Unit
{ x = 10, y = 20, z = 30, label = "point" } : { x: Integer, y: Integer, z: Integer, label: String }
```

**Notes:** Merging records combines their fields

### Test: Record merge with merge function

**Input:**

```cadenza
let base = { x = 10, y = 20 }
let extra = { z = 30, label = "point" }
Record.merge base extra
```

**Output:**

```repl
() : Unit
() : Unit
{ x = 10, y = 20, z = 30, label = "point" } : { x: Integer, y: Integer, z: Integer, label: String }
```

---

## Enum Variant Qualification

Enum variants are accessed through the enum name to avoid conflicts and make code clearer. This follows Rust's approach rather than ML's unqualified variants.

### Qualified Variants

Variants are always qualified with the enum name: `EnumName.Variant`

**Benefits:**

- No name conflicts between different enums
- Clear which enum a variant belongs to
- Easier to refactor (rename enums without changing variant names)
- Better IDE support (autocomplete shows variants)

### Test: Qualified variant construction

**Input:**

```cadenza
enum Status {
    Pending,
    Active,
}
let s = Status.Pending
```

**Output:**

```repl
() : Unit
() : Unit
s : Status
```

**Notes:** Variants accessed as `Status.Pending`, not just `Pending`

### Test: Multiple enums with similar variants

**Input:**

```cadenza
enum HttpStatus { Ok, Error }
enum DbStatus { Ok, Error }
let http = HttpStatus.Ok
let db = DbStatus.Ok
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
() : Unit
Ok : HttpStatus
Ok : DbStatus
```

**Notes:** No conflict because variants are qualified

### Variant Imports

Variants can be imported into the scope

**Input:**

```cadenza
enum Status { Pending, Active }

use { Pending, Active } = Status

Pending
Active
```

**Output:**

```repl
() : Unit
() : Unit
Pending : Status
Active : Status
```

### Variant Wildcard Imports

Variants can be imported into the scope

**Input:**

```cadenza
enum Status { Pending, Active }

use Status

Pending
Active
```

**Output:**

```repl
() : Unit
() : Unit
Pending : Status
Active : Status
```

---

### Test: ERROR - Missing required field

**Input:**

```cadenza
struct Point { x = Integer, y = Integer }
Point { x = 10 }
```

**Output:**

```
error: missing struct field
 --> test.cdz:2:7
  |
2 | Point { x = 10 }
  |       ^^^^^^^^^ missing field `y`
  |
  = note: struct Point requires fields: x, y
```

### Test: ERROR - Wrong field type

**Input:**

```cadenza
struct Point { x = Integer, y = Integer }
Point { x = "hello", y = 20 }
```

**Output:**

```
error: type mismatch in struct field
 --> test.cdz:2:13
  |
2 | Point { x = "hello", y = 20 }
  |             ^^^^^^^ expected Integer, found String
  |
  = note: field `x` requires type Integer
```

---

## Enums

Enums define a type that can be one of several named variants, each potentially carrying associated data.

### Syntax

```
enum <Name> {
    <Variant1>,
    <Variant2> <Type>,
    <Variant3> { <field>: <Type>, ... },
    ...
}
```

### Semantics

An enum definition:

1. Creates a new nominal type
2. Registers constructor functions for each variant
3. Returns `Unit`

Each variant becomes a constructor function.

### Test: Simple enum definition

**Input:**

```cadenza
enum Status {
    Pending,
    Active,
    Completed,
}
```

**Output:**

```repl
() : Unit
```

### Test: Enum with associated data

**Input:**

```cadenza
enum Result {
    Ok Integer,
    Err String,
}
let success = Result.Ok 42
let failure = Result.Err "something went wrong"
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
success : Result
failure : Result
```

**Notes:** Variants can carry different types of data

### Test: Enum with record variants

**Input:**

```cadenza
enum Shape {
    Circle { radius: Float },
    Rectangle { width: Float, height: Float }
}
let c = Shape.Circle { radius = 5.0 }
let r = Shape.Rectangle { width = 10.0, height = 20.0 }
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
c : Shape
r : Shape
```

**Notes:** Variants can have named fields like structs

---

## Lists

Lists are dynamically-sized sequences of values of the same type. They're implemented as contiguous arrays in memory.

### Syntax

```
[<expr1>, <expr2>, ...]
[]                       # Empty list
```

### Semantics

List expressions evaluate each element and construct a list. All elements must have the same type.

### Type

`[<T>]` where `T` is the element type.

### Test: Simple list

**Input:**

```cadenza
[1, 2, 3, 4, 5]
```

**Output:**

```repl
[1, 2, 3, 4, 5] : [Integer]
```

### Test: Empty list

**Input:**

```cadenza
[]
```

**Output:**

```repl
[] : [Unknown]
```

**Notes:** Empty lists have unknown element type until used in context

### Test: List from variables

**Input:**

```cadenza
let x = 10
let y = 20
[x, y, x + y]
```

**Output:**

```repl
() : Unit
() : Unit
[10, 20, 30] : [Integer]
```

### Test: Nested lists

**Input:**

```cadenza
[[1, 2], [3, 4], [5, 6]]
```

**Output:**

```repl
[[1, 2], [3, 4], [5, 6]] : [[Integer]]
```

### Test: ERROR - Heterogeneous list

**Input:**

```cadenza
[1, "hello", true]
```

**Output:**

```
error: type mismatch in list
 --> test.cdz:1:4
  |
1 | [1, "hello", true]
  |     ^^^^^^^ expected Integer, found String
  |
  = note: all list elements must have the same type
  = note: first element has type Integer
```

**Notes:** Lists are homogeneous - all elements must have the same type

---

## List Indexing

Elements are accessed by zero-based index.

### Syntax

```
<list>[<index>]
```

### Semantics

Evaluate the list and index expressions, then return the element at that position. Indexing out of bounds is a runtime error.

### Test: Simple indexing

**Input:**

```cadenza
let nums = [10, 20, 30]
nums[0]
nums[2]
```

**Output:**

```repl
() : Unit
10 : Integer
30 : Integer
```

**Notes:** Lists use zero-based indexing

### Test: ERROR - Index out of bounds

**Input:**

```cadenza
let nums = [1, 2, 3]
nums[5]
```

**Output:**

```
error: index out of bounds
 --> test.cdz:2:6
  |
2 | nums[5]
  |      ^ index 5 is out of bounds
  |
  = note: list has length 3
  = note: valid indices are 0..2
```

### Test: ERROR - Negative index

**Input:**

```cadenza
let nums = [1, 2, 3]
nums[-1]
```

**Output:**

```
error: negative index
 --> test.cdz:2:6
  |
2 | nums[-1]
  |      ^^ indices must be non-negative
  |
  = help: use `List.last` to access the last element
```

---

## Structural vs Nominal Typing

Cadenza uses both structural and nominal typing depending on the data type.

### Structural Types

Records and tuples use structural typing - types are equal if their structure matches.

### Test: Structural record equality

**Input:**

```cadenza
let point1 = { x = 10, y = 20 }
let point2 = { x = 10, y = 20 }
typeof point1 == typeof point2
```

**Output:**

```repl
() : Unit
() : Unit
true : Bool
```

**Notes:** Both have the same structure, so same type

### Nominal Types

Structs and enums use nominal typing - types are equal only if they have the same name.

### Test: Nominal struct types differ

**Input:**

```cadenza
struct Point { x = Integer, y = Integer }
struct Vector { x = Integer, y = Integer }
typeof (Point { x = 0, y = 0 }) == typeof (Vector { x = 0, y = 0 })
```

**Output:**

```repl
() : Unit
() : Unit
false : Bool
```

**Notes:** Despite identical structure, Point and Vector are different types

---

## Field Assignment

Record and struct fields can be updated using assignment syntax.

### Syntax

```
<record>.<field> = <expression>
```

### Semantics

Field assignment:

1. Evaluates the record expression
2. Evaluates the new value expression
3. Updates the field in place
4. Frees the old value (for linear types)
5. Returns `Unit`

### Test: Simple field assignment

**Input:**

```cadenza
let point = { x = 10, y = 20 }
point.x = 30
point
```

**Output:**

```repl
() : Unit
() : Unit
{ x = 30, y = 20 } : { x: Integer, y: Integer }
```

### Test: Field assignment with expression

**Input:**

```cadenza
let point = { x = 10, y = 20 }
point.x = point.x + 5
point.x
```

**Output:**

```repl
() : Unit
() : Unit
15 : Integer
```

**Notes:** Can use current field value in the new value expression

### Test: ERROR - Assign to missing field

**Input:**

```cadenza
let point = { x = 10, y = 20 }
point.z = 30
```

**Output:**

```
error: field not found
 --> test.cdz:2:7
  |
2 | point.z = 30
  |       ^ field `z` does not exist
  |
  = note: record has fields: x, y
```

---

## Type Aliases

Type aliases create new names for existing types without creating new nominal types.

### Syntax

```
type <Name> = <Type>
```

### Semantics

A type alias:

1. Registers the name as an alias for the type
2. Returns `Unit`

The alias and original type are completely interchangeable.

### Test: Simple type alias

**Input:**

```cadenza
type Point = { x: Integer, y: Integer }
let p: Point = { x = 10, y = 20 }
```

**Output:**

```repl
() : Unit
() : Unit
p : Point
```

**Notes:** Type aliases provide descriptive names for complex types

### Test: Alias is transparent

**Input:**

```cadenza
type UserId = Integer
let id: UserId = 42
let num: Integer = id
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
num : Integer
```

**Notes:** Type aliases don't create new nominal types, just new names

---

## Compiler Queries

Compound types require:

- `eval(tuple)` - Evaluate elements, construct tuple
- `eval(record)` - Evaluate fields, construct record
- `eval(struct_def)` - Register nominal type and constructor
- `eval(enum_def)` - Register nominal type and variant constructors
- `eval(field_access)` - Extract field from record/struct
- `eval(field_assign)` - Update field, free old value
- `eval(index)` - Extract element from list, check bounds
- `typeof(tuple)` - Infer tuple type from elements
- `typeof(record)` - Infer record type from fields
- `typeof(struct)` - Look up nominal type
- `typeof(field_access)` - Look up field type
- `check_nominal(type1, type2)` - Check nominal type equality
- `check_structural(type1, type2)` - Check structural type equality

## Implementation Notes

- Records use structural typing: same fields = same type
- Structs use nominal typing: same name required for equality
- Tuples are structural but often used for temporary grouping
- Lists are homogeneous and dynamically sized
- Field access is compile-time checked against type
- Field assignment updates in place, freeing old value for linear types
- Empty list type is inferred from usage context
- Type aliases are compile-time only, erased in generated code
