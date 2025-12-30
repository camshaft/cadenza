# Documentation and Doctests

Documentation is a first-class feature in Cadenza. Every definition can have documentation that's accessible at compile time and runtime, and documentation can include executable examples that become part of the test suite.

## Documentation Comments

Documentation is written using special comment syntax that attaches to the following definition.

### Syntax

```
## <documentation text>
<definition>
```

Double-hash comments (`##`) attach documentation to the next definition. Multiple consecutive doc comments are concatenated.

### Semantics

When a documented definition is created:

1. Parse the documentation text (Markdown format)
2. Extract any code examples (doctests)
3. Attach the documentation to the definition
4. Register doctests in the test suite
5. Make documentation queryable via `docof`

### Test: Function with documentation

**Input:**

```cadenza
## Doubles the input value.
##
## Returns the input multiplied by 2.
fn double x = x * 2
```

**Output:**

```repl
() : Unit
```

**Notes:** Documentation is attached to the function definition

### Test: Struct with documentation

**Input:**

```cadenza
## A 2D point with x and y coordinates.
##
## Points use integer coordinates for precision.
struct Point {
    ## Point in the `x` field
    x = Integer,
    ## Point in the `y` field
    y = Integer,
}
```

**Output:**

```repl
() : Unit
```

---

## Reading Documentation at Compile Time

The `docof` operator returns the documentation of a definition as a syntax tree. Documentation is parsed from Markdown into normal expressions, making it easy to traverse, manipulate, and evaluate.

### Syntax

```
docof <identifier>
```

### Semantics

`docof` looks up the definition and returns its documentation as Cadenza syntax. Markdown elements become function calls:

- Paragraphs: `(p "text content")`
- Headings: `(h1 "title")`, `(h2 "title")`, etc.
- Code blocks: `(code "language" "content")`
- Lists: `(ul (li "item 1") (li "item 2"))`
- Inline code: `(code_inline "text")`
- Bold: `(strong "text")`
- Emphasis: `(em "text")`

If no documentation exists, returns `()`.

### Type

`docof` returns a value of type `Syntax` - an expression tree.

### Test: docof returns syntax tree

**Input:**

```cadenza
## Adds two numbers together.
fn add x y = x + y
docof add
```

**Output:**

```repl
() : Unit
(p "Adds two numbers together.") : Syntax
```

**Notes:** The paragraph becomes a regular function call `(p "...")`

### Test: docof with heading

**Input:**

```cadenza
## Summary line.
##
## # Details
##
## More information here.
fn example = 42
docof example
```

**Output:**

```repl
() : Unit
(__block__
  (p "Summary line.")
  (h1 "Details")
  (p "More information here.")) : Syntax
```

**Notes:** Multiple elements wrapped in a block

### Test: docof with code block

**Input:**

````cadenza
## Example function.
##
## ```cadenza
## example 42
## ```
fn example x = x
docof example
````

**Output:**

```repl
() : Unit
(__block__
  (p "Example function.")
  (code "cadenza" "example 42")) : Syntax
```

**Notes:** Code blocks preserve language and content

### Test: Check for undocumented items

**Input:**

```cadenza
## Documented function.
fn documented x = x

fn undocumented x = x

for def in Module.exports
    let doc = docof def
    if doc == (quote ()) then
        println $"Warning: ${def} is missing documentation"
        break
```

**Output:**

```repl
() : Unit
() : Unit
Warning: undocumented is missing documentation
```

**Notes:** Missing documentation returns `quote ()`, which is easy to check

---

## Doctests

Doctests are executable code examples embedded in documentation comments. They're extracted and added to the test suite automatically.

### Syntax

````
## Documentation text.
##
## ```
## <test code>
## ```
fn <name> ...
````

Code blocks automatically become executable tests. The test code can reference the item being documented.

### Semantics

When documentation with doctests is processed:

1. Extract all code blocks marked with `test`
2. Generate test functions from the examples
3. Add tests to the module's test suite
4. Tests run with `cadenza test`

Doctests are compiled as part of the module, not as separate compilation units. This makes them fast and allows them to access private definitions.

### Test: Function with doctest

**Input:**

````cadenza
## Squares a number.
##
## ```
## assert (square 5) == 25
## ```
fn square x = x * x
````

**Output:**

```repl
() : Unit
```

**Notes:** The doctest is extracted and becomes a test named `test_square_doctest_1`

### Test: Multiple doctests

**Input:**

````cadenza
## Adds two numbers.
##
## ```
## assert (add 2 3) == 5
## ```
##
## ```
## assert (add 0 10) == 10
## ```
fn add x y = x + y
````

**Output:**

```repl
() : Unit
```

**Notes:** Each example becomes a separate test

---

## Doctest Placeholders

Doctests use placeholders to avoid hardcoding module paths, making them resilient to refactoring.

### Syntax

```
$pkg        # Current package name
$module     # Current module path
$item       # The documented item's name
```

### Semantics

When generating doctests, placeholders are replaced:

- `$pkg` → the package name (e.g., `"math"`)
- `$module` → the full module path (e.g., `"math.geometry"`)
- `$item` → the item being documented (e.g., `"Point"`)

This makes doctests portable and refactoring-safe.

### Test: Doctest with placeholders

**Input:**

````cadenza
## Creates a new point.
##
## ```
## let p = $module.Point { x = 10, y = 20 }
## assert p.x == 10
## ```
struct Point { x = Integer, y = Integer }
````

**Output:**

```repl
() : Unit
```

**Notes:** `$module` is replaced with the actual module path when generating the test

### Test: Doctest using $item

**Input:**

````cadenza
## Doubles a number.
##
## ```
## assert ($item 5) == 10
## ```
fn double x = x * 2
````

**Output:**

```repl
() : Unit
```

**Notes:** `$item` is replaced with `double`, making the test code more maintainable

---

### Multi-line Tests

Multiple expressions can be tested in sequence.

### Test: Multi-line doctest

**Input:**

````cadenza
## Counter with state.
##
## ```
## let c = make_counter
## Cell.set c 5
## let v = Cell.get c
## assert v == 5
## ```
fn make_counter = Cell.new 0
````

**Notes:** Each line is part of the same test, sharing bindings

### Error Tests

Doctests can verify that errors occur.

### Test: Doctest expecting error

**Input:**

````cadenza
## Divides two numbers.
##
## ```
## assert (divide 10 2) == 5
## ```
##
## ```must_panic
## divide 10 0
## ```
fn divide a b =
    assert b != 0 "division by zero"
    a / b
````

**Notes:** The `must_panic` marker indicates the test should produce an error

---

## Documentation Format

Documentation uses Markdown with special conventions.

### Structure

````
## <One-line summary>
##
## <Detailed description with multiple paragraphs>
##
## # Examples
##
## ```
## <example code>
## ```
##
## # Parameters
##
## - `x` - <description>
## - `y` - <description>
##
## # Returns
##
## <description of return value>
````

### Test: Structured documentation

**Input:**

````cadenza
## Calculates the distance between two points.
##
## Uses the Euclidean distance formula: √((x₂-x₁)² + (y₂-y₁)²)
##
## # Examples
##
## ```
## let p1 = Point { x = 0, y = 0 }
## let p2 = Point { x = 3, y = 4 }
## let d = distance p1 p2
## assert d == 5.0
## ```
##
## # Parameters
##
## - `p1` - The first point
## - `p2` - The second point
##
## # Returns
##
## The distance as a Float value.
fn distance p1 p2 =
    let dx = p2.x - p1.x
    let dy = p2.y - p1.y
    Float.sqrt (dx * dx + dy * dy)
````

**Output:**

```repl
() : Unit
```

**Notes:** Well-structured documentation improves code clarity and IDE experience

## Compiler Queries

Documentation requires:

- `eval(doc_comment)` - Parse documentation, extract doctests, attach to definition
- `docof(item)` - Return documentation string for item
- `extract_doctests(doc)` - Extract test blocks from documentation
- `generate_test(doctest, context)` - Create test function from doctest
- `resolve_placeholder(placeholder, context)` - Replace $crate, $module, $item

## Implementation Notes

- Documentation is stored as attributes on definitions
- Doctests are generated during compilation, not as separate units
- `docof` performs a compile-time lookup in the definition table
- Markdown in documentation can include any formatting
- Code blocks can be unmarked (`test`), `test`, `must_panic`, or `no_test`
- Placeholders are resolved based on the item's location in the module hierarchy
- Tests are named: `test_<item>_doctest_<n>`
