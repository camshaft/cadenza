# Cadenza Syntax Parser - Status Document

This document tracks the current state of the `cadenza-syntax` crate and remaining parser implementation work.

## Overview

The syntax crate implements a whitespace-significant, keyword-less parser for Cadenza using Pratt parsing with binding powers. The parser produces a lossless CST (Concrete Syntax Tree) using the rowan library.

## Current State

The parser implements most core syntax features:

✅ **Completed:**
- Basic lexer and parser infrastructure
- All operator categories (arithmetic, comparison, logical, bitwise, shift, assignment, range)
- Field access (`.`) and path access (`::`)
- Prefix operators (`@`, `!`, `~`, `$`, `...`)
- Postfix operators (`?`, `|?`)
- Array literals (`[1, 2, 3]`)
- Record literals - **shorthand only** (`{ x, y }`)
- Parenthesized expressions
- Indentation-based blocks
- Apply nodes for function application
- Whitespace significance tracking
- Generated binding power system

⚠️ **Partially Complete:**
- **Records**: Shorthand syntax works (`{ x, y }`), but field assignments (`{ x = 1 }`) fail due to parser marker propagation issue
- **Error Recovery**: Basic error nodes exist but recovery is weak

❌ **Not Implemented:**
- Array indexing (`arr[0]`)
- Quote/unquote operators (syntax not finalized)
- Tuples vs grouping distinction
- Prefix negation (`-x`) - intentionally removed due to ambiguity
- Match expressions
- If/else expressions
- Function/closure syntax
- Loops (while, for, loop)
- Partial function application (`&foo arg`)
- String interpolation

## Detailed Status by Feature

### 1. Operators ✅ MOSTLY COMPLETE

**Status:** All operator categories implemented with proper binding powers.

**Completed:**
- Generated binding power system with enums
- All infix operators (pipe, range, assignment, logical, bitwise, shift, arithmetic, exponentiation, field/path access)
- Postfix operators (`?`, `|?`)
- Prefix operators (`@`, `!`, `~`, `$`, `...`)

**Known Issues:**
- Prefix negation (`-x`) removed due to ambiguity with infix minus
- Requires context-aware parsing to distinguish `-x` (negation) from `a - x` (subtraction)

**References:** `PARSER_ISSUES.md` Issue 1

---

### 2. Quote/Unquote ❌ NOT IMPLEMENTED

**Status:** Infrastructure missing, syntax not finalized.

**Design Decision Needed:**
- Current tokens: `'` (SingleQuote), `~` (Tilde)
- Single quote conflicts with potential character literals
- Alternative syntaxes to consider:
  - Backtick for quote: `` `expr ``
  - Dollar for unquote: `$expr`
  - Keyword-like: `quote expr` / `unquote expr`

**What's Needed:**
1. Finalize syntax decision
2. Add as prefix operators in parser
3. Represent as `Apply(__quote__, [expr])` and `Apply(__unquote__, [expr])`
4. Handle nested quotes and unquotes
5. Support block quotes with indentation

**References:** `PARSER_ISSUES.md` Issue 2

---

### 3. Array Literals ✅ COMPLETE

**Status:** Fully implemented with comma-separated elements.

**Working:**
- Empty arrays: `[]`
- Single/multiple elements: `[1]`, `[1, 2, 3]`
- Nested arrays: `[[1, 2], [3, 4]]`
- Trailing commas: `[1, 2,]`
- Complex expressions: `[a + b, c * d]`

**Representation:** `Apply(__list__, [elements...])`

**Test Files:** `array-*.cdz` in test-data/

---

### 4. Array Indexing ❌ NOT IMPLEMENTED

**Status:** No postfix bracket handling.

**What's Needed:**
1. Add `LBracket` as postfix operator in binding power system
2. Parse expression inside brackets as index
3. Represent as `Apply(__index__, [array, index])`
4. Handle chained indexing: `arr[0][1]`
5. Handle complex expressions: `arr[i + 1]`

**Design Challenge:** Distinguish `[` as array literal prefix vs indexing postfix

**References:** `PARSER_ISSUES.md` Issue 4

---

### 5. Record Creation ⚠️ PARTIAL

**Status:** Shorthand works, field assignments blocked.

**Working:**
- Empty records: `{}`
- Shorthand fields: `{ x, y }` → `[__record__, x, y]`

**Not Working:**
- Field assignments: `{ x = 1 }` - **BLOCKED**
- Mixed shorthand and assignments: `{ x, y = 10 }`
- Nested records with assignments

**Root Cause:** Parser marker propagation issue with low binding power operators inside delimiters.

When parsing `{ x = 1 }`:
1. Parser enters `parse_record` with `BraceMarker`
2. Parses field with `CommaMarker(BraceMarker)`
3. Sees `=` operator (binding power 5,4)
4. Creates new `WhitespaceMarker` for RHS, **loses delimiter context**
5. After parsing `1`, sees `}` on same line
6. Whitespace marker says "continue" 
7. Juxtaposition (BP 6) >= 4, tries to consume `}` as argument
8. Error: closing brace consumed incorrectly

**Solution:** Propagate parent delimiter markers through recursive parsing so child contexts know when to stop.

**Workaround:** Shorthand expansion (`{ x }` → `{ x = x }`) deferred to macro expansion time.

**References:** `PARSER_ISSUES.md` Issue 5

---

### 6. Record Field Access ✅ COMPLETE

**Status:** Dot notation fully implemented.

**Working:**
- Simple access: `point.x`
- Chained access: `obj.field.subfield`
- After function calls: `get_point().x`
- With indexing: `arr[0].name`
- In expressions: `point.x + point.y`

**Representation:** `Apply(., [record, field])`

**Binding Power:** Field access (30, 31) - high precedence, left-associative

**Test Files:** `op-field-*.cdz` in test-data/

**References:** `PARSER_ISSUES.md` Issue 6

---

### 7. Tuples ❌ NOT IMPLEMENTED

**Status:** Parentheses currently only used for grouping.

**What's Needed:**
1. Detect comma in parentheses to distinguish tuple from grouping
2. Single element without comma: `(x)` is grouping
3. Multiple elements or trailing comma: `(x, y)` or `(x,)` is tuple
4. Represent as `Apply(__tuple__, [elements...])`
5. Empty tuple: `()` → `Apply(__tuple__, [])`

**Design Decision:** Should `()` be empty tuple or unit type?

**References:** `PARSER_ISSUES.md` Issue 7

---

### 8. Match Expressions ❌ NOT IMPLEMENTED

**Status:** No pattern matching support.

**What's Needed:**
1. Parse match arms with patterns and bodies
2. Support `|` as pattern separator or indentation-based
3. Represent as `Apply(match, [scrutinee, arm1, arm2, ...])`
4. Each arm: `Apply(__arm__, [pattern, body])`
5. Support pattern syntax: literals, identifiers, constructors
6. Handle guard clauses: `pattern if condition -> body`

**Syntax Examples:**
```cadenza
match x
    0 -> "zero"
    1 -> "one"
    _ -> "other"
```

**References:** `PARSER_ISSUES.md` Issue 8

---

### 9. If/Else Expressions ❌ NOT IMPLEMENTED

**Status:** No conditional expression support.

**What's Needed:**
1. Recognize `if condition then consequent else alternative` pattern
2. Support indented bodies after `then` and `else`
3. Represent as `Apply(if, [condition, consequent, alternative])`
4. Support `elif` chains by nesting
5. Handle if without else (if language design permits)

**Syntax Examples:**
```cadenza
if x > 0 then "positive" else "negative"

if condition then
    do_something
else
    do_other
```

**References:** `PARSER_ISSUES.md` Issue 9

---

### 10. Closures and Functions ❌ NOT IMPLEMENTED

**Status:** No function definition syntax.

**Design Decisions Needed:**

**Functions (hoisted):**
- Named, can be called before definition
- Syntax: `fn name params -> body`
- Enables mutual recursion

**Closures (not hoisted):**
- Anonymous, bound to variables via `let`
- Must be defined before use
- Several syntax options:
  - Bare arrow: `x -> x + 1`
  - Backslash: `\x -> x + 1` (Haskell-like)
  - Pipes: `|x| x + 1` (Rust-like)
  - `fn` keyword: `fn x -> x + 1`

**Open Questions:**
1. Closure syntax choice
2. Function name position (`fn double x` vs `double = fn x`)
3. Parameter syntax (curried `fn add x y` vs tuple `fn add (x, y)`)

**References:** `PARSER_ISSUES.md` Issue 10

---

### 11. Loops ❌ NOT IMPLEMENTED

**Status:** No loop constructs.

**What's Needed:**
1. Parse `loop` with block body
2. Parse `while` with condition and block
3. Parse `for pattern in iterable` with block
4. Add `break` and `continue` for loop control
5. Optional: loop labels for nested loop control

**Design Implications:** Supporting loops implies supporting mutability.

**Syntax Examples:**
```cadenza
loop
    do_something
    if done then break

while condition
    do_something

for x in collection
    process x
```

**References:** `PARSER_ISSUES.md` Issue 11

---

### 12. Partial Function Application ❌ NOT IMPLEMENTED

**Status:** No partial application support.

**What's Needed:**
1. Add `&` as prefix operator for partial application
2. Parse function name and following arguments
3. Handle `$0`, `$1`, `$2` as positional placeholders
4. Represent as `Apply(&, [fn, arg1, arg2, ...])`
5. Handle holes for argument reordering

**Design Conflict:** `&` also used for bitwise AND (infix) and potential reference operator.

**Syntax Examples:**
```cadenza
let f = &add          # Capture function
let add5 = &add 5     # Partial application
let middle = &substring 0 $0 10  # With holes
```

**References:** `PARSER_ISSUES.md` Issue 12

---

### 13. Error Recovery ⚠️ PARTIAL

**Status:** Basic error nodes exist, recovery is weak.

**Issues:**
1. `parse_primary()` has catch-all that accepts invalid tokens
2. Missing synchronization strategy after errors
3. Limited negative test coverage
4. Errors in delimited contexts don't propagate correctly

**What's Needed:**
1. Restrict valid primary tokens (identifiers, literals, delimiters, prefix ops)
2. Create Error nodes for unexpected tokens
3. Implement synchronization (skip to newline, delimiter, etc.)
4. Add comprehensive negative tests
5. Fix marker propagation to handle dedented expressions in delimiters

**Example Issue:** `foo [\nbar` where `bar` is at root indentation should not become an argument of `foo`.

**References:** `PARSER_ISSUES.md` Issue 13

---

### 14. String Interpolation ❌ NOT IMPLEMENTED

**Status:** Basic strings work, no interpolation or heredoc support.

**What's Needed:**
1. Add `:` prefix for interpolated strings: `:"hello ${name}"`
2. Parse `${expr}` within strings, switch to expression parsing
3. Multi-line heredoc strings:
   - Detect newline after opening quote
   - Strip common leading indentation
   - Emit `StringLine` tokens
4. New token types: `InterpolatedStringStart`, `InterpolationStart`, `InterpolationEnd`, `StringLine`
5. Represent as `Apply(__interp__, [part1, expr1, part2, expr2, ...])`

**Open Questions:**
1. Interpolation syntax: `:"..."` vs `$"..."` vs `f"..."` vs `` `...` ``
2. Escape syntax: `\${` vs `$${`
3. Expression restrictions in `${}`
4. Indentation stripping algorithm for multi-line

**References:** `PARSER_ISSUES.md` Issue 14

---

## Implementation Priority

Based on dependencies and common usage:

1. **Error Recovery** ⚠️ - Critical for usability
2. **Fix Record Field Assignments** ⚠️ - Core data structure
3. **Array Indexing** - Basic data structure access
4. **Tuples** - Foundation for destructuring
5. **If/Else** - Basic control flow
6. **Functions/Closures** - Core language feature (needs design decision)
7. **String Interpolation** - Common feature
8. **Quote/Unquote** - Metaprogramming (needs design decision)
9. **Loops** - Control flow
10. **Match** - Pattern matching (most complex)
11. **Partial Application** - Advanced feature

## Design Decisions Needed

Several features are blocked on design decisions:

1. **Quote/Unquote Syntax:** `'` vs `` ` `` vs `quote`
2. **Closure Syntax:** `->` vs `\` vs `|x|` vs `fn`
3. **Function Syntax:** Name position and parameter style
4. **Partial Application:** Conflict with `&` operator
5. **Tuple vs Unit:** What is `()`?
6. **String Interpolation:** Which prefix syntax?

## Technical Debt

### Parser Marker Propagation Issue

**Impact:** Blocks record field assignments and potentially other features.

**Root Cause:** When parsing RHS of low binding power operators inside delimiters, the parser creates a new `WhitespaceMarker` that doesn't know about the outer delimiter context.

**Solution:** Refactor `parse_expression_bp` to accept and propagate parent delimiter markers through recursive calls. Child markers should compose with parent markers to check both whitespace rules AND delimiter boundaries.

**Affected Features:**
- Record field assignments (primary blocker)
- Arrays with assignment operators (e.g., `[x = 1]`)
- Any low binding power operator inside delimiters

### Prefix Negation Ambiguity

**Issue:** Removed `-` as prefix operator due to ambiguity with infix minus.

**Challenge:** `- x` vs `a - x` requires context-aware parsing or lookahead.

**Potential Solutions:**
1. Require parentheses: `(-x)` for negation
2. Context-aware parsing based on previous token
3. Lookahead to check for whitespace patterns
4. Accept ambiguity and resolve during semantic analysis

## Test Coverage

**Test Data Files:** 330+ tests in `test-data/` directory

**Coverage:**
- ✅ Lexer: All tokens tested
- ✅ Operators: All implemented operators tested
- ✅ Arrays: Comprehensive tests
- ✅ Records: Shorthand tested, assignments fail
- ⚠️ Error cases: Limited negative tests in `invalid-parse/`
- ❌ Missing features: No tests (can't test what doesn't exist)

## References

- **PARSER_ISSUES.md** - Detailed implementation issues with test cases
- **design-doc.md** - High-level syntax design
- **build/token.rs** - Generated binding power system
- **src/parse.rs** - Main parser implementation (838 lines)

## Notes

- Parser uses Pratt parsing with generated binding powers
- All constructs represented as `Apply` nodes
- No keywords - everything is function application
- Whitespace significant - indentation creates blocks
- Lossless CST with rowan library
- Tests use snapshot testing with insta
