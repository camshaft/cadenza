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
- Array indexing (`arr[0]`) with whitespace-based disambiguation from array literals
- Record literals with field assignments (`{ x = 1 }`)
- Parenthesized expressions
- Indentation-based blocks
- Apply nodes for function application
- Whitespace significance tracking
- Generated binding power system

✅ **Completed:**
- **Error Recovery**: Error nodes now properly handle error cases:
  - Dedented expressions inside delimiters are handled (e.g., `foo [\nbar` creates error + recovers)
  - Missing delimiters detected and reported
  - Invalid tokens at expression start create error nodes (closing delimiters, unexpected punctuation)
  - Trailing operators properly handled (e.g., `a +` creates error for missing RHS)
  - Multi-line error recovery works between statements
  - 16 comprehensive negative test files in `invalid-parse/`
  - Note: Operators can be used as values (e.g., `+` alone is valid), following keyword-less design

❌ **Not Implemented:**
- Quote/unquote operators - not high priority, can use `quote` and `unquote` as identifiers
- Tuples vs grouping distinction
- Prefix negation (`-x`) - intentionally removed due to ambiguity
- Match expressions
- If/else expressions
- Function/closure syntax - bare arrow syntax preferred for closures
- Loops (while, for, loop)
- Partial function application - `&` conflicts with bitwise AND, needs different symbol
- String interpolation - use JS-style `${name}` to reserve `:` for type annotations

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

### 2. Quote/Unquote ❌ NOT IMPLEMENTED (LOW PRIORITY)

**Status:** Not implemented. **Not high priority** - can use `quote` and `unquote` as regular identifiers for now.

**Approach:**
- Use identifiers: `quote expr` / `unquote expr` work with current parser
- No special syntax needed initially
- Can revisit specialized syntax later if needed

**If specialized syntax is desired later:**
- Alternative syntaxes to consider:
  - Backtick for quote: `` `expr ``
  - Different prefix characters
  - Keep as keywords

**What Would Be Needed:**
1. Decide on syntax (if not using identifiers)
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

### 4. Array Indexing ✅ COMPLETE

**Status:** Fully implemented with whitespace-based disambiguation.

**Working:**
- Simple indexing: `arr[0]` → `[__index__, arr, 0]`
- Variable indexing: `arr[i]` → `[__index__, arr, i]`
- Complex expressions: `arr[i + 1]` → `[__index__, arr, [+, i, 1]]`
- Chained indexing: `matrix[0][1]` → `[__index__, [__index__, matrix, 0], 1]`
- After function calls: `get_array[0]` → `[__index__, [get_array], 0]`
- Array literal indexing: `[1, 2, 3][0]` → `[__index__, [__list__, 1, 2, 3], 0]`
- Whitespace distinction: `arr[0]` (indexing) vs `arr [0]` (function application)

**Implementation:**
- `LBracket` detected before skipping trivia to determine intent
- No whitespace before `[` → array indexing with binding power 34
- Whitespace before `[` → function application with array literal
- Represented as `Apply(__index__, [array, index])`
- Uses `BracketMarker` to handle bracket matching and error recovery

**Test Files:** `index-*.cdz` in test-data/

**References:** `PARSER_ISSUES.md` Issue 4

---

### 5. Record Creation ✅ COMPLETE

**Status:** Fully implemented with field assignments working.

**Working:**
- Empty records: `{}`
- Shorthand fields: `{ x, y }` → `[__record__, x, y]`
- Field assignments: `{ x = 1 }` → `[__record__, [=, x, 1]]`
- Mixed shorthand and assignments: `{ x, y = 10 }`
- Nested records: `{ a = { b = 1 } }`
- Field expressions: `{ a = 2 + 2 }`

**Note:** The parser marker propagation issue has been resolved. Records with low binding power operators now work correctly.

**Test Files:** `record-*.cdz` in test-data/

**References:** `PARSER_ISSUES.md` Issue 5

---

### 6. Record Field Access ✅ COMPLETE

**Status:** Dot notation fully implemented.

**Working:**
- Simple access: `point.x`
- Chained access: `obj.field.subfield`
- After function calls: `get_point.x` (functions auto-apply with 0 args)
- With indexing: `arr[0].name` (once indexing is implemented)
- In expressions: `point.x + point.y`

**Note:** Function calls don't use `()` - zero-arity functions are auto-applied. Use `(get_point).x` if you need to reference the function itself.

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

**Design Decision:** Treat `()` as empty tuple/unit type, same as Rust.

**References:** `PARSER_ISSUES.md` Issue 7

---

### 8. Match Expressions ✅ COMPLETE (Basic Boolean Matching)

**Status:** Basic pattern matching on boolean values is working. The parser naturally supports match expressions through function application.

**Working:**
- Simple boolean patterns: `match x > 0 (true -> "positive") (false -> "negative")`
- Nested match: `match a (true -> match b (true -> 1) (false -> 2)) (false -> 3)`
- Both syntaxes work: with or without outer parentheses
  - `match cond (true -> a) (false -> b)` ✅
  - `(match cond (true -> a) (false -> b))` ✅
- Pattern arms use arrow syntax: `pattern -> result`
- Each arm is a parenthesized expression: `(pattern -> result)`

**Implementation:**
- Parser represents as left-associative function application: `[[[match, scrutinee], arm1], arm2]`
- Special form evaluation in `cadenza-eval` handles boolean patterns
- IR generation creates branching control flow with phi nodes
- Full support in evaluation, IR, and WebAssembly compilation

**Current Limitations:**
- Only supports boolean patterns (`true`/`false`)
- No support for literal patterns (numbers, strings)
- No constructor patterns or destructuring
- No guard clauses (`pattern if condition`)
- No wildcard pattern (`_`)
- No or-patterns (`pattern1 | pattern2`)

**Future Work:**
For comprehensive pattern matching, would need:
1. Parse match arms with more pattern types
2. Support `|` as pattern separator for or-patterns
3. Support literal patterns (numbers, strings)
4. Support constructor patterns for algebraic data types
5. Handle guard clauses: `pattern if condition -> body`
6. Wildcard pattern `_` for catch-all

**Syntax Examples:**
```cadenza
# Current (boolean patterns only)
match x > 0 (true -> "positive") (false -> "negative")

# Future (with more patterns)
match x
    0 -> "zero"
    1 -> "one"
    _ -> "other"
```

**Test Files:** `if-simple.cdz`, `fn-match-phi.cdz`, `match-no-parens.cdz` in cadenza-eval/test-data/

**References:** `PARSER_ISSUES.md` Issue 8

---

### 9. If/Else Expressions ❌ NOT IMPLEMENTED

**Status:** No conditional expression support.

**Design Challenge:** How to support `if`/`else` without parser specialization? Current parser has no keywords, only identifiers and punctuation.

**Alternatives:**

1. **Match-style cond expression:**
```cadenza
cond
    expr1 -> result1
    expr2 -> result2
    true -> "base case"
```
This fits naturally with the current parser design.

2. **Traditional if/else (requires parser changes):**
```cadenza
if x > 0 then "positive" else "negative"
```
This requires special handling of `if`, `then`, `else` tokens, breaking the no-keywords principle.

**What's Needed:**
1. Decide between `cond` (no parser changes) vs `if/then/else` (parser specialization)
2. If using `if/then/else`: Recognize the pattern and parse as special form
3. Support indented bodies
4. Represent as `Apply(if, [condition, consequent, alternative])` or `Apply(cond, [arms...])`

**References:** `PARSER_ISSUES.md` Issue 9

---

### 10. Closures and Functions ❌ NOT IMPLEMENTED

**Status:** No function definition syntax.

**Design Decisions:**

**Functions (hoisted):**
- Named, can be called before definition
- Syntax: `fn name params -> body` (name comes after `fn`)
- Parameters are curried: `fn add x y -> x + y`
- Enables mutual recursion

**Closures (not hoisted):**
- Anonymous, bound to variables via `let`
- Must be defined before use
- **Preferred syntax:** Bare arrow `x -> x + 1`
- Alternative: `\x -> x + 1` (Haskell-like)
- Rust-style `|x| x + 1` not chosen

**Examples:**
```cadenza
# Function (hoisted)
fn add x y -> x + y

# Closure (not hoisted)
let double = x -> x * 2
let add5 = y -> add y 5
```

**Open Questions:**
- How to make bare arrow syntax work without parser ambiguity?

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

**Preferred Syntax:**
```cadenza
loop
    do_something
    if done then break

while condition
    do_something

for x <- collection
    process x
```

**Note:** Using `<-` instead of `in` for the for-loop syntax.
```

**References:** `PARSER_ISSUES.md` Issue 11

---

### 12. Partial Function Application ❌ NOT IMPLEMENTED

**Status:** No partial application support.

**Design Conflict:** `&` conflicts with bitwise AND (infix) and potential reference operator. **Need alternative symbol.**

**Possible Alternatives:**
- `@` prefix (but conflicts with attribute operator)
- `%` prefix
- `#` prefix
- Different syntax entirely

**What's Needed:**
1. Choose non-conflicting symbol
2. Parse function name and following arguments
3. Handle `$0`, `$1`, `$2` as positional placeholders
4. Represent as `Apply(symbol, [fn, arg1, arg2, ...])`
5. Handle holes for argument reordering

**Syntax Examples (using hypothetical symbol):**
```cadenza
let f = ?add          # Capture function
let add5 = ?add 5     # Partial application
let middle = ?substring 0 $0 10  # With holes
```

**References:** `PARSER_ISSUES.md` Issue 12

---

### 13. Error Recovery ✅ COMPLETE

**Status:** Comprehensive error nodes and recovery for error cases.

**Working:**
- Dedented expressions in delimiters: `foo [\nbar` correctly creates error + recovers with `bar` as separate expression
- Missing delimiters detected: `{ a = 1` emits "expected }" error
- Invalid tokens at expression start create error nodes (closing delimiters, unexpected punctuation)
- Trailing operators handled: `a +` creates error for missing RHS
- Multi-line error recovery: Multiple errors on separate lines all caught and parsing continues
- Unexpected closing delimiters (`)`, `]`, `}`) create error nodes
- Unexpected punctuation (`,`, `;`) creates error nodes

**Design Note:**
- Operators can be used as values in this keyword-less language (e.g., `+` alone is valid)
- This allows functional programming patterns like passing operators as arguments
- `a + + b` parses as valid syntax (function application), not as an error

**Test Coverage:**
- 15 comprehensive negative test files in `invalid-parse/`:
  - `error-unexpected-rparen.cdz`
  - `error-unexpected-rbracket.cdz`
  - `error-unexpected-rbrace.cdz`
  - `error-unexpected-comma.cdz`
  - `error-trailing-operator.cdz`
  - `error-recovery-next-line.cdz`
  - `error-recovery-multiple.cdz`
  - Plus 8 existing tests for delimiter errors

**Implementation:**
- Modified `parse_primary()` to explicitly handle invalid tokens
- Operators and other tokens allowed as primaries (for keyword-less design)
- Only truly invalid tokens (closing delimiters, punctuation, EOF) create errors
- Error nodes created with `Kind::Error` wrapping the problematic token
- Clear error messages using `display_name()` for tokens
- Parser continues after errors for multi-error recovery

**References:** `PARSER_ISSUES.md` Issue 13

---

### 14. String Interpolation ❌ NOT IMPLEMENTED

**Status:** Basic strings work, no interpolation or heredoc support.

**Design Decisions:**
- Use JS-style `${name}` for interpolation (reserves `:` for type annotations like `let v: integer = 1`)
- Use `\${` for escaping literal `${`
- No prefix needed - just embed `${expr}` in regular strings

**What's Needed:**
1. Parse `${expr}` within strings, switch to expression parsing
2. Multi-line heredoc strings:
   - Detect newline after opening quote
   - Strip common leading indentation
   - Emit `StringLine` tokens
3. New token types: `InterpolationStart` (`${`), `InterpolationEnd` (`}`)
4. Represent as `Apply(__interp__, [part1, expr1, part2, expr2, ...])`

**Syntax Examples:**
```cadenza
"hello ${name}"
"result: ${a + b}"
"multi-line:
    value = ${x}
    done"
```

**Open Questions:**
- Expression restrictions in `${}`? (probably allow any expression)
- Indentation stripping algorithm for multi-line

**References:** `PARSER_ISSUES.md` Issue 14

---

## Implementation Priority

Based on dependencies, design decisions, and common usage:

1. ~~**Error Recovery Improvements**~~ ✅ **COMPLETE** - Comprehensive error handling with 16 negative tests
2. ~~**Array Indexing**~~ ✅ **COMPLETE** - Whitespace-based disambiguation working with 7 test cases
3. **Tuples** - Foundation for destructuring
4. **If/Else or Cond** - Decide between parser specialization vs match-style
5. **Functions/Closures** - Implement with decided syntax (bare arrow for closures, curried params)
6. **String Interpolation** - JS-style `${expr}`, no prefix needed
7. **Loops** - with `for x <- collection` syntax
8. **Match** - Pattern matching (most complex)
9. **Quote/Unquote** - Low priority, can use identifiers
10. **Partial Application** - Need to choose non-conflicting symbol

## Design Decisions Status

**Decided:**
- ✅ **Closure Syntax:** Bare arrow `x -> x + 1` (preferred if can make it work)
- ✅ **Function Syntax:** Name after `fn`, curried params: `fn add x y -> x + y`
- ✅ **Tuple/Unit:** `()` is empty tuple/unit, same as Rust
- ✅ **String Interpolation:** JS-style `${name}`, reserves `:` for type annotations, escape with `\${`
- ✅ **For Loop Syntax:** `for x <- collection` (using `<-` instead of `in`)
- ✅ **Records:** All working, marker propagation issue resolved
- ✅ **Array Indexing:** Whitespace-based disambiguation works perfectly

**Still Needed:**
- ⚠️ **If/Else:** Parser specialization vs `cond` match-style syntax
- ⚠️ **Partial Application:** Need alternative symbol to `&`
- ⚠️ **Quote/Unquote:** Low priority - can use `quote`/`unquote` identifiers

## Technical Debt

### ~~Parser Marker Propagation Issue~~ ✅ RESOLVED

**Previous Impact:** Blocked record field assignments and arrays with low binding power operators.

**Resolution:** The marker propagation issue has been fixed. Records with field assignments now work correctly, including nested records and expressions.

**Verified Working:**
- `{ x = 1 }` ✅
- `{ a = { b = 1 } }` ✅
- `{ a = 2 + 2 }` ✅

### Prefix Negation Ambiguity

**Issue:** Removed `-` as prefix operator due to ambiguity with infix minus.

**Challenge:** `- x` vs `a - x` requires context-aware parsing or lookahead.

**Potential Solutions:**
1. Require parentheses: `(-x)` for negation
2. Context-aware parsing based on previous token
3. Lookahead to check for whitespace patterns
4. Accept ambiguity and resolve during semantic analysis

## Test Coverage

**Test Data Files:** 337+ tests in `test-data/` directory

**Coverage:**
- ✅ Lexer: All tokens tested
- ✅ Operators: All implemented operators tested
- ✅ Arrays: Comprehensive tests including nested, trailing commas, expressions
- ✅ Array Indexing: 7 tests covering simple, variable, expression, chained, and whitespace disambiguation
- ✅ Records: All working - empty, shorthand, field assignments, nested, expressions
- ✅ Error cases: 16 negative tests in `invalid-parse/`
- ❌ Missing features: No tests for unimplemented features

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
