# Parser Implementation Issues

This document tracks the remaining parser implementation work for Cadenza. Each section describes a feature that needs to be implemented, the current state, what needs to be done, and test cases with expected outputs.

Based on the design document, all constructs are represented as `Apply` nodes. The parser uses Pratt parsing for operator precedence.

---

## Issue 1: Complete Operator Support ✅ MOSTLY COMPLETE

### Summary

**Status:** Infrastructure complete, most operators implemented. Binding power generation system successfully deployed.

**Completed:**

- ✅ Generated binding power system with automatic precedence calculation
- ✅ All infix operators (arithmetic, comparison, logical, bitwise, shift, assignment, range)
- ✅ Postfix operators (`?`, `|?`)
- ✅ Prefix operators (`@`, `!`, `~`, `$` - but not `-` due to ambiguity)
- ✅ Missing tokens added to lexer (`**`, `..=`, `<<=`, `>>=`, `->`, `<-`, `${`)
- ✅ Parser integration with generated methods

**Remaining Work:**

- ⚠️ Prefix negation (`-x`) - removed due to infix/prefix ambiguity, needs context-aware parsing
- ✅ Field access (`.`) - implemented with binding power (30, 31)
- ✅ Path separator (`::`) - implemented with binding power (32, 33)
- ⚠️ Array indexing (`[]`) - needs special postfix handling

### Current State

The parser now uses a **generated binding power system** defined in `build/token.rs`. All operator precedence is centrally defined using enums and automatically calculated with proper associativity.

**Architecture:**

- Binding power enums: `PrefixBindingPower`, `InfixBindingPower`, `PostfixBindingPower`
- Each enum variant represents a precedence group
- Binding powers are calculated: `base = enum_discriminant * 2`
- Associativity determines left/right BP: Left = `(base, base+1)`, Right = `(base+1, base)`
- Generated methods on `Kind`: `prefix_binding_power()`, `infix_binding_power()`, `postfix_binding_power()`, `juxtaposition_binding_power()`

**Postfix operators:**
| Operator | Token | Binding Power | Status |
|----------|-------|---------------|--------|
| `?` | Question | 32 | ✅ Implemented |
| `\|?` | PipeQuestion | 0 | ✅ Implemented |

**Prefix operators:**
| Operator | Token | Binding Power | Status |
|----------|-------|---------------|--------|
| `@` | At | 0 | ✅ Implemented |
| `!` | Bang | 26 | ✅ Implemented |
| `~` | Tilde | 26 | ✅ Implemented |
| `$` | Dollar | 26 | ✅ Implemented |
| `-` | Minus | - | ❌ Removed (ambiguity with infix) |

**Infix operators (by precedence group, low to high):**
| Group | Operators | Left BP | Right BP | Assoc | Status |
|-------|-----------|---------|----------|-------|--------|
| Pipe | `\|>` | 0 | 1 | Left | ✅ Implemented |
| Range | `..`, `..=` | 2 | 3 | Left | ✅ Implemented |
| Assignment | `=`, `+=`, `-=`, `*=`, `/=`, `%=`, `&=`, `\|=`, `^=`, `<<=`, `>>=`, `->`, `<-` | 5 | 4 | Right | ✅ Implemented |
| Juxtaposition | (function application) | 6 | 7 | Left | ✅ Implemented |
| Logical OR | `\|\|` | 8 | 9 | Left | ✅ Implemented |
| Logical AND | `&&` | 10 | 11 | Left | ✅ Implemented |
| Equality | `==`, `!=` | 12 | 13 | Left | ✅ Implemented |
| Comparison | `<`, `<=`, `>`, `>=` | 14 | 15 | Left | ✅ Implemented |
| Bitwise OR | `\|` | 16 | 17 | Left | ✅ Implemented |
| Bitwise XOR | `^` | 18 | 19 | Left | ✅ Implemented |
| Bitwise AND | `&` | 20 | 21 | Left | ✅ Implemented |
| Shift | `<<`, `>>` | 22 | 23 | Left | ✅ Implemented |
| Additive | `+`, `-` | 24 | 25 | Left | ✅ Implemented |
| Multiplicative | `*`, `/`, `%` | 26 | 27 | Left | ✅ Implemented |
| Exponentiation | `**` | 29 | 28 | Right | ✅ Implemented |
| Field Access | `.` | 30 | 31 | Left | ✅ Implemented |
| Path Access | `::` | 32 | 33 | Left | ✅ Implemented |

### Rust Operators Missing from Cadenza

Comparing against [Rust's operator precedence](https://doc.rust-lang.org/reference/expressions.html#expression-precedence):

**Missing Infix Operators:**
| Operator | Description | Rust Precedence | Suggested BP |
|----------|-------------|-----------------|--------------|
| `**` | Exponentiation | N/A in Rust | (14, 13) right-associative |
| `^` | Bitwise XOR | Between \|\| and && | (5, 6) |
| `\|` | Bitwise OR | Below ^ | (4, 5) |
| `&` | Bitwise AND | Above ^ | (7, 8) |
| `<<` | Left shift | Above + - | (9, 10) |
| `>>` | Right shift | Above + - | (9, 10) |
| `..` | Range | Low precedence | (2, 3) |
| `..=` | Inclusive range | Low precedence | (2, 3) |
| `+=` | Add assign | Same as = | (3, 2) right-associative |
| `-=` | Sub assign | Same as = | (3, 2) right-associative |
| `*=` | Mul assign | Same as = | (3, 2) right-associative |
| `/=` | Div assign | Same as = | (3, 2) right-associative |
| `%=` | Mod assign | Same as = | (3, 2) right-associative |
| `&=` | And assign | Same as = | (3, 2) right-associative |
| `\|=` | Or assign | Same as = | (3, 2) right-associative |
| `^=` | Xor assign | Same as = | (3, 2) right-associative |
| `<<=` | Shl assign | Same as = | (3, 2) right-associative |
| `>>=` | Shr assign | Same as = | (3, 2) right-associative |
| `->` | Arrow (fn types) | Special | N/A (type syntax) |
| `::` | Path separator | Highest | (17, 18) |
| `.` | Field access | Highest | (16, 17) |

**Missing Prefix Operators:**
| Operator | Description | Status |
|----------|-------------|--------|
| `-` | Negation | ❌ Not implemented |
| `!` | Logical NOT | ❌ Not implemented |
| `*` | Dereference | ❌ Not implemented |
| `&` | Reference | ❌ Not implemented |

**Tokens Available but No Binding Power:**
The lexer already recognizes these tokens but they have no binding power assigned:

- `Caret` (`^`) - Bitwise XOR
- `Pipe` (`|`) - Bitwise OR
- `Ampersand` (`&`) - Bitwise AND / Reference
- `LessLess` (`<<`) - Left shift
- `GreaterGreater` (`>>`) - Right shift
- `DotDot` (`..`) - Range
- `Arrow` (`->`) - Function arrow
- `ColonColon` (`::`) - Path separator
- `Dot` (`.`) - Field access
- `Bang` (`!`) - Logical NOT
- `Tilde` (`~`) - Bitwise NOT / Unquote
- All compound assignment operators (`+=`, `-=`, etc.)

**Tokens Missing from Lexer (need to be added):**

- `StarStar` (`**`) - Exponentiation
- `DotDotEqual` (`..=`) - Inclusive range
- `LessLessEqual` (`<<=`) - Left shift assign
- `GreaterGreaterEqual` (`>>=`) - Right shift assign

### What Needs To Be Done

1. **Add missing tokens to lexer** (`lexer.rs` and `build/token.rs`):

   - `**` → `StarStar`
   - `..=` → `DotDotEqual`
   - `<<=` → `LessLessEqual`
   - `>>=` → `GreaterGreaterEqual`

2. **Add exponentiation operator** - Using `**` since `^` is reserved for bitwise XOR. Add with right-associativity (highest arithmetic precedence).

3. **Add bitwise operators** to `infix_binding_power` (see complete binding power table below for exact values):

   ```rust
   // Bitwise OR (between comparison and logical OR)
   Pipe => (15, 16),

   // Bitwise XOR (between bitwise OR and bitwise AND)
   Caret => (17, 18),

   // Bitwise AND (between bitwise XOR and shifts)
   Ampersand => (19, 20),

   // Shifts (between bitwise AND and additive)
   LessLess | GreaterGreater => (21, 22),
   ```

   Note: Exact binding power values will need to be adjusted to fit between existing operators. The complete table below shows the final relative ordering.

4. **Add prefix operators** - implement `prefix_binding_power`:

   ```rust
   fn prefix_binding_power(op: Kind) -> Option<u8> {
       Some(match op {
           Minus => 14,  // Negation
           Bang => 14,   // Logical NOT
           Tilde => 14,  // Bitwise NOT
           Ampersand => 14, // Reference
           Star => 14,   // Dereference
           _ => return None,
       })
   }
   ```

5. **Add field access** (`.`) as high-precedence left-associative infix operator

6. **Add path separator** (`::`) as highest-precedence left-associative infix operator

7. **Add range operators** (`..`, `..=`) as low-precedence operators

8. **Add compound assignment operators** with same precedence as `=`

### Suggested Complete Binding Power Table

From lowest to highest precedence:

| Level | Operators                                                          | Associativity | Type           |
| ----- | ------------------------------------------------------------------ | ------------- | -------------- |
| 1     | `\|>`, `\|?`                                                       | Left          | Pipe/PipeTry   |
| 2     | `..`, `..=`                                                        | Left          | Range          |
| 3     | `=`, `+=`, `-=`, `*=`, `/=`, `%=`, `&=`, `\|=`, `^=`, `<<=`, `>>=` | Right         | Assignment     |
| 4     | `\|\|`                                                             | Left          | Logical OR     |
| 5     | `&&`                                                               | Left          | Logical AND    |
| 6     | `==`, `!=`                                                         | Left          | Equality       |
| 7     | `<`, `<=`, `>`, `>=`                                               | Left          | Comparison     |
| 8     | `\|`                                                               | Left          | Bitwise OR     |
| 9     | `^`                                                                | Left          | Bitwise XOR    |
| 10    | `&`                                                                | Left          | Bitwise AND    |
| 11    | `<<`, `>>`                                                         | Left          | Shift          |
| 12    | `+`, `-`                                                           | Left          | Additive       |
| 13    | `*`, `/`, `%`                                                      | Left          | Multiplicative |
| 14    | `**`                                                               | Right         | Exponentiation |
| 15    | `-`, `!`, `~`, `&`, `*`                                            | Right         | Prefix unary   |
| 16    | `?`                                                                | Left          | Postfix try    |
| 17    | `.`                                                                | Left          | Field access   |
| 18    | `::`                                                               | Left          | Path           |
| 19    | `[]`                                                               | Left          | Indexing       |
| 20    | `()`                                                               | Left          | Call           |

### Test Cases

#### Test: op-exponent.cdz

```cadenza
2 ** 3 ** 2
```

**Expected AST (right-associative, so 2^(3^2) = 2^9 = 512):**

```
[
    [**, 2, [**, 3, 2]],
]
```

#### Test: op-exponent-with-mul.cdz

```cadenza
2 * 3 ** 2
```

**Expected AST (** binds tighter than \*):\*\*

```
[
    [*, 2, [**, 3, 2]],
]
```

#### Test: op-bitwise-and.cdz

```cadenza
a & b
```

**Expected AST:**

```
[
    [&, a, b],
]
```

#### Test: op-bitwise-or.cdz

```cadenza
a | b
```

**Expected AST:**

```
[
    [|, a, b],
]
```

#### Test: op-bitwise-xor.cdz

```cadenza
a ^ b
```

**Expected AST:**

```
[
    [^, a, b],
]
```

#### Test: op-bitwise-precedence.cdz

```cadenza
a | b ^ c & d
```

**Expected AST (& > ^ > |):**

```
[
    [|, a, [^, b, [&, c, d]]],
]
```

#### Test: op-shift-left.cdz

```cadenza
x << 2
```

**Expected AST:**

```
[
    [<<, x, 2],
]
```

#### Test: op-shift-right.cdz

```cadenza
x >> 2
```

**Expected AST:**

```
[
    [>>, x, 2],
]
```

#### Test: op-shift-with-add.cdz

```cadenza
a + b << c
```

**Expected AST (<< lower than +):**

```
[
    [<<, [+, a, b], c],
]
```

#### Test: op-range.cdz

```cadenza
1..10
```

**Expected AST:**

```
[
    [.., 1, 10],
]
```

#### Test: op-range-inclusive.cdz

```cadenza
1..=10
```

**Expected AST:**

```
[
    [..=, 1, 10],
]
```

#### Test: op-negate.cdz

```cadenza
-x
```

**Expected AST:**

```
[
    [-, x],
]
```

#### Test: op-negate-in-expr.cdz

```cadenza
a + -b
```

**Expected AST:**

```
[
    [+, a, [-, b]],
]
```

#### Test: op-not.cdz

```cadenza
!x
```

**Expected AST:**

```
[
    [!, x],
]
```

#### Test: op-not-in-expr.cdz

```cadenza
!a && !b
```

**Expected AST:**

```
[
    [&&, [!, a], [!, b]],
]
```

#### Test: op-compound-assign.cdz

```cadenza
x += 1
```

**Expected AST:**

```
[
    [+=, x, 1],
]
```

#### Test: op-all-compound-assign.cdz

```cadenza
a -= 1
b *= 2
c /= 3
d %= 4
e &= 5
f |= 6
g ^= 7
h <<= 8
i >>= 9
```

**Expected AST:**

```
[
    [-=, a, 1],
    [*=, b, 2],
    [/=, c, 3],
    [%=, d, 4],
    [&=, e, 5],
    [|=, f, 6],
    [^=, g, 7],
    [<<=, h, 8],
    [>>=, i, 9],
]
```

#### Test: op-path.cdz

```cadenza
std::io::Read
```

**Expected AST:**

```
[
    [::, [::, std, io], Read],
]
```

---

## Issue 2: Quote/Unquote (Syntax Metaprogramming)

### Current State

The lexer recognizes `'` (`SingleQuote`) and `~` (`Tilde`) tokens. According to the design doc:

- `'expr` → `Apply(__quote__, [expr])` - capture syntax as a value
- `~expr` → `Apply(__unquote__, [expr])` - splice syntax back

However, the parser does not currently handle these as prefix operators. Additionally, the single quote character (`'`) may conflict with character literals or other language constructs, so **an alternative syntax needs to be decided**.

### Syntax Alternatives to Consider

1. **Backtick for quote**: `` `expr `` (backtick is already a token: `Backtick`)
2. **Dollar for unquote**: `$expr` (dollar is already a token: `Dollar`)
3. **Keyword-like**: `quote expr` / `unquote expr` (still function application)
4. **Bracket-based**: `'[expr]` / `~[expr]`
5. **Hash-based**: `#'expr` / `#~expr`
6. **Lisp-style with backtick**: `` `expr `` for quote, `,expr` for unquote

### What Needs To Be Done

1. **Decide on final syntax** for quote and unquote operators
2. Add quote operator as a prefix operator in `parse_primary`
3. Add unquote operator as a prefix operator in `parse_primary`
4. Represent as: `Apply(__quote__, [expr])` and `Apply(__unquote__, [expr])`
5. Handle nested quotes: `'('x)` and unquotes within quotes
6. Handle block quotes with indentation

### Test Cases

_Note: These examples use `'` and `~` but the actual syntax may change._

#### Test: quote-simple.cdz

```cadenza
'x
```

**Expected AST:**

```
[
    [__quote__, x],
]
```

#### Test: quote-expr.cdz

```cadenza
'(a + b)
```

**Expected AST:**

```
[
    [__quote__, [+, a, b]],
]
```

#### Test: quote-call.cdz

```cadenza
'(foo bar baz)
```

**Expected AST:**

```
[
    [__quote__, [[foo, bar], baz]],
]
```

#### Test: unquote-simple.cdz

```cadenza
~x
```

**Expected AST:**

```
[
    [__unquote__, x],
]
```

#### Test: quote-with-unquote.cdz

```cadenza
'(let x = ~value)
```

**Expected AST:**

```
[
    [__quote__, [=, [let, x], [__unquote__, value]]],
]
```

#### Test: quote-block.cdz

```cadenza
let ast = '
    let out = ~var
    out + 1
```

**Expected AST:**

```
[
    [=, [let, ast], [__quote__,
        [__block__,
            [=, [let, out], [__unquote__, var]],
            [+, out, 1]]]],
]
```

#### Test: unquote-splice.cdz

```cadenza
~ast
```

**Expected AST:**

```
[
    [__unquote__, ast],
]
```

#### Test: nested-quote.cdz

```cadenza
''x
```

**Expected AST:**

```
[
    [__quote__, [__quote__, x]],
]
```

#### Test: quote-in-function.cdz

```cadenza
let make_let = fn name value -> '(let ~name = ~value)
```

**Expected AST:**

```
[
    [=, [let, make_let],
        [[fn, name, value],
            [__quote__, [=, [let, [__unquote__, name]], [__unquote__, value]]]]],
]
```

---

## Issue 3: Array Literals

### Current State

The lexer recognizes `[` (`LBracket`) and `]` (`RBracket`) tokens, but the parser does not handle array literal syntax. Currently, `[` is consumed as a standalone token without proper array literal parsing.

### What Needs To Be Done

1. Add a `parse_array` function that handles `[` as a prefix to start an array literal
2. Parse comma-separated elements within the brackets
3. Represent the array as: `Apply(__list__, [element1, element2, ...])`
4. Handle empty arrays: `[]` → `Apply(__list__, [])`
5. Handle trailing commas: `[1, 2,]`
6. Handle nested arrays: `[[1, 2], [3, 4]]`

### Test Cases

#### Test: array-empty.cdz

```cadenza
[]
```

**Expected AST:**

```
[
    [__list__],
]
```

#### Test: array-single.cdz

```cadenza
[1]
```

**Expected AST:**

```
[
    [__list__, 1],
]
```

#### Test: array-simple.cdz

```cadenza
[1, 2, 3]
```

**Expected AST:**

```
[
    [__list__, 1, 2, 3],
]
```

#### Test: array-with-exprs.cdz

```cadenza
[a + b, c * d]
```

**Expected AST:**

```
[
    [__list__, [+, a, b], [*, c, d]],
]
```

#### Test: array-nested.cdz

```cadenza
[[1, 2], [3, 4]]
```

**Expected AST:**

```
[
    [__list__, [__list__, 1, 2], [__list__, 3, 4]],
]
```

#### Test: array-trailing-comma.cdz

```cadenza
[1, 2, 3,]
```

**Expected AST:**

```
[
    [__list__, 1, 2, 3],
]
```

---

## Issue 4: Array Indexing

### Current State

The lexer recognizes `[` and `]` tokens, but there is no postfix indexing support. The parser cannot handle `arr[0]` syntax.

### What Needs To Be Done

1. Add `LBracket` as a postfix operator in the parser's binding power system
2. Parse the expression inside the brackets as the index
3. Represent indexing as: `Apply(__index__, [array, index])`
4. Handle chained indexing: `arr[0][1]`
5. Handle complex index expressions: `arr[i + 1]`

### Test Cases

#### Test: index-simple.cdz

```cadenza
arr[0]
```

**Expected AST:**

```
[
    [__index__, arr, 0],
]
```

#### Test: index-variable.cdz

```cadenza
arr[i]
```

**Expected AST:**

```
[
    [__index__, arr, i],
]
```

#### Test: index-expr.cdz

```cadenza
arr[i + 1]
```

**Expected AST:**

```
[
    [__index__, arr, [+, i, 1]],
]
```

#### Test: index-chained.cdz

```cadenza
matrix[0][1]
```

**Expected AST:**

```
[
    [__index__, [__index__, matrix, 0], 1],
]
```

#### Test: index-after-call.cdz

```cadenza
get_array()[0]
```

**Expected AST:**

```
[
    [__index__, [get_array], 0],
]
```

#### Test: index-with-array-literal.cdz

```cadenza
[1, 2, 3][0]
```

**Expected AST:**

```
[
    [__index__, [__list__, 1, 2, 3], 0],
]
```

---

## Issue 5: Record Creation ⚠️ PARTIAL

### Summary

**Status:** Basic record syntax implemented, but field assignments don't work.

**Completed:**
- ✅ `SyntheticRecord` token with `__record__` identifier added
- ✅ `parse_record` function implemented (modeled on `parse_array`)
- ✅ `BraceMarker` type alias for brace-delimited expressions
- ✅ Empty records: `{}` → `[__record__]`
- ✅ Shorthand fields: `{ x, y }` → `[__record__, x, y]`

**Not Working:**
- ❌ Field assignments: `{ x = 1 }` - fails due to parser limitation
- ❌ Nested records with assignments
- ❌ Records with expressions in field values

**Note:** Shorthand expansion (`{ x }` → `{ x = x }`) is deferred to macro expansion time. The parser only captures the identifiers; the expansion is done later.

### Implementation Complexity: Assignment Operator Issue

Records with `=` assignments (like `{ x = 1 }`) don't work due to a fundamental limitation in how the parser handles low binding power operators inside delimiters. This same issue affects arrays with assignments (e.g., `[x = 1]` also fails).

**Root Cause:** When parsing the RHS of an infix operator, the parser creates a new `WhitespaceMarker` that doesn't know about the outer delimiter context (`}`, `]`, etc.). For operators with low binding power like `=` (binding power 5,4), the parser continues past the closing delimiter and tries to treat it as a juxtaposition argument.

**Example trace for `{ x = 1 }`:**
1. Parser enters `parse_record` with `BraceMarker`
2. `parse_expression_bp(0, CommaMarker(BraceMarker))` called for field
3. Parses `x` as primary
4. Sees `=` infix operator, binding power (5, 4), continues
5. Parse RHS with `self.whitespace.marker()` (loses delimiter context!)
6. Parses `1` as primary
7. Sees `}` - whitespace marker says "continue" (same line)
8. `}` has no binding power, falls to juxtaposition (binding power 6, 7)
9. 6 >= 4 (RHS min_bp), so parser tries to apply `1` to `}`
10. Error: closing brace consumed as argument

**Why arrays with `+` work but not `=`:** The `+` operator has high binding power (24, 25), so when parsing its RHS with min_bp=25, juxtaposition (6) < 25 and the parser stops correctly. With `=` (min_bp=4), juxtaposition (6) >= 4 continues incorrectly.

### Proposed Solution

The recommended fix is to have markers passed from the top-most call through all recursive parsing, allowing parent delimiters to influence child parsing. This would require:

1. Refactoring `parse_expression_bp` to accept an optional parent delimiter marker
2. When creating child markers for RHS parsing, compose them with the parent marker
3. The child marker's `should_continue` would check both whitespace rules AND parent delimiter boundaries

This is a significant architectural change that would benefit both record and array parsing for any low binding power operators.

### What Needs To Be Done

1. ~~Add a `parse_record` function that handles `{` as a prefix to start a record literal~~ ✅
2. ❌ Parse field definitions as `name = value` pairs (blocked by marker issue)
3. ❌ Handle shorthand field syntax expansion (deferred to macro expansion time)
4. Represent records as: `Apply(__record__, [field1, field2, ...])`
   - Currently: shorthand only captures identifiers
   - Goal: full form with `Apply(=, [field, value])` for each field
5. ~~Handle empty records: `{}`~~ ✅
6. ❌ Handle trailing commas in field lists (blocked by marker issue)

### Test Cases

#### Test: record-empty.cdz ✅ WORKING

```cadenza
{}
```

**Expected AST:**

```
[
    [__record__],
]
```

#### Test: record-shorthand.cdz ✅ WORKING (without expansion)

```cadenza
{ x, y }
```

**Current AST (shorthand not expanded - expansion deferred to macro time):**

```
[
    [__record__, x, y],
]
```

**Future AST (after macro expansion):**

```
[
    [__record__, [=, x, x], [=, y, y]],
]
```

#### Test: record-single-field.cdz ❌ NOT WORKING

```cadenza
{ x = 1 }
```

**Expected AST:**

```
[
    [__record__, [=, x, 1]],
]
```

**Current Status:** Fails with "expected }" error due to marker propagation issue.

#### Test: record-multi-field.cdz ❌ NOT WORKING

```cadenza
{ x = 1, y = 2, z = 3 }
```

**Expected AST:**

```
[
    [__record__, [=, x, 1], [=, y, 2], [=, z, 3]],
]
```

**Current Status:** Fails due to marker propagation issue.

#### Test: record-mixed.cdz ❌ NOT WORKING

```cadenza
{ x, y = 10 }
```

**Expected AST:**

```
[
    [__record__, [=, x, x], [=, y, 10]],
]
```

**Current Status:** Fails due to marker propagation issue.

#### Test: record-nested.cdz ❌ NOT WORKING

```cadenza
{ point = { x = 0, y = 0 } }
```

**Expected AST:**

```
[
    [__record__, [=, point, [__record__, [=, x, 0], [=, y, 0]]]],
]
```

**Current Status:** Fails due to marker propagation issue.

#### Test: record-with-exprs.cdz ❌ NOT WORKING

```cadenza
{ sum = a + b, product = a * b }
```

**Expected AST:**

```
[
    [__record__, [=, sum, [+, a, b]], [=, product, [*, a, b]]],
]
```

**Current Status:** Fails due to marker propagation issue.

---

## Issue 6: Record Field Access (Dot Notation)

### Current State

The lexer recognizes `.` (`Dot`) token, but the parser does not handle it as a field access operator.

### What Needs To Be Done

1. Add `.` as an infix operator with high binding power (between function application and postfix operators)
2. Parse the right-hand side as an identifier (field name)
3. Represent field access as: `Apply(., [record, field])`
4. Handle chained access: `a.b.c`
5. Handle method-like syntax: `obj.method arg` → `Apply(method, [obj, arg])`

### Test Cases

#### Test: field-simple.cdz

```cadenza
point.x
```

**Expected AST:**

```
[
    [., point, x],
]
```

#### Test: field-chained.cdz

```cadenza
obj.field.subfield
```

**Expected AST:**

```
[
    [., [., obj, field], subfield],
]
```

#### Test: field-after-call.cdz

```cadenza
get_point().x
```

**Expected AST:**

```
[
    [., [get_point], x],
]
```

#### Test: field-with-index.cdz

```cadenza
arr[0].name
```

**Expected AST:**

```
[
    [., [__index__, arr, 0], name],
]
```

#### Test: field-method-call.cdz

```cadenza
list.map fn x -> x * 2
```

**Expected AST:**

```
[
    [[., list, map], [fn, x, [*, x, 2]]],
]
```

#### Test: field-in-expr.cdz

```cadenza
point.x + point.y
```

**Expected AST:**

```
[
    [+, [., point, x], [., point, y]],
]
```

---

## Issue 7: Tuples

### Current State

Parentheses are currently used for grouping expressions. The parser handles `(expr)` but does not distinguish between grouping and tuple creation. According to the design doc, `(foo, bar)` should parse as a list.

### What Needs To Be Done

1. Modify `parse_expression` within parentheses to check for commas
2. Single element without comma: `(x)` remains grouping
3. Multiple elements or trailing comma: `(x, y)` or `(x,)` creates a tuple
4. Represent tuples as: `Apply(__tuple__, [element1, element2, ...])`
5. Handle empty tuple: `()` → `Apply(__tuple__, [])`
6. Support tuple patterns for destructuring (future semantic analysis)

### Test Cases

#### Test: tuple-empty.cdz

```cadenza
()
```

**Expected AST:**

```
[
    [__tuple__],
]
```

#### Test: tuple-single.cdz

```cadenza
(1,)
```

**Expected AST:**

```
[
    [__tuple__, 1],
]
```

#### Test: tuple-pair.cdz

```cadenza
(1, 2)
```

**Expected AST:**

```
[
    [__tuple__, 1, 2],
]
```

#### Test: tuple-triple.cdz

```cadenza
(a, b, c)
```

**Expected AST:**

```
[
    [__tuple__, a, b, c],
]
```

#### Test: tuple-nested.cdz

```cadenza
((1, 2), (3, 4))
```

**Expected AST:**

```
[
    [__tuple__, [__tuple__, 1, 2], [__tuple__, 3, 4]],
]
```

#### Test: tuple-with-exprs.cdz

```cadenza
(a + b, c * d)
```

**Expected AST:**

```
[
    [__tuple__, [+, a, b], [*, c, d]],
]
```

#### Test: grouping-vs-tuple.cdz

```cadenza
(a + b)
```

**Expected AST (grouping, not tuple):**

```
[
    [+, a, b],
]
```

---

## Issue 8: Match Expressions

### Current State

**Basic boolean pattern matching is now working!** The parser naturally supports match expressions through function application.

Working syntaxes:

Single-line:
```cadenza
match x > 0 (true -> "positive") (false -> "negative")
```

Indented (preferred):
```cadenza
match x > 0
    (true -> "positive")
    (false -> "negative")
```

The parser represents this as left-associative function application:
```
[[[match, [>, x, 0]], [->, true, "positive"]], [->, false, "negative"]]
```

**Important:** Each pattern arm MUST be wrapped in parentheses. This is required because
the `->` operator has lower binding power (Assignment) than function application (Juxtaposition).
Without parentheses, `match cond true -> 42` would incorrectly parse as `(match cond true) -> 42`.

Current capabilities:
- Boolean patterns (`true`/`false`)
- Arrow syntax for pattern arms: `(pattern -> result)`
- Single-line and indented syntax
- Nested match expressions
- Full evaluation, IR generation, and WebAssembly compilation support

### What Still Needs To Be Done

For comprehensive pattern matching with more pattern types:

1. Support more pattern types beyond booleans:
   - Literal patterns: numbers, strings
   - Wildcard pattern: `_`
   - Constructor patterns for algebraic data types
   - Tuple/record destructuring patterns

2. Consider syntax improvements:
   - Could adjust operator precedence to allow `pattern -> result` without parentheses
   - Or introduce special syntax like `match x: pattern -> result` to signal match context
   - Or use a different separator like `|` between pattern and body

3. Advanced features:
   - Or-patterns: `pattern1 | pattern2 -> result`
   - Guard clauses: `pattern if condition -> result`
   - Nested patterns: `Some (Just x) -> ...`

### Test Cases

#### Working Now: match-boolean.cdz

Single-line:
```cadenza
match x > 0 (true -> "positive") (false -> "negative")
```

Indented:
```cadenza
match x > 0
    (true -> "positive")
    (false -> "negative")
```

**Actual AST:**

```
[[[match, [>, x, 0]], [->, true, "positive"]], [->, false, "negative"]]
```

#### Future: match-simple.cdz

```cadenza
match x
    0 -> "zero"
    1 -> "one"
    _ -> "other"
```

**Expected AST:**

```
[
    [match, x,
        [__arm__, 0, "zero"],
        [__arm__, 1, "one"],
        [__arm__, _, "other"]],
]
```

#### Test: match-inline.cdz

```cadenza
match x | 0 -> "zero" | 1 -> "one" | _ -> "other"
```

**Expected AST:**

```
[
    [match, x,
        [__arm__, 0, "zero"],
        [__arm__, 1, "one"],
        [__arm__, _, "other"]],
]
```

#### Test: match-with-binding.cdz

```cadenza
match opt
    Some x -> x
    None -> 0
```

**Expected AST:**

```
[
    [match, opt,
        [__arm__, [Some, x], x],
        [__arm__, None, 0]],
]
```

#### Test: match-tuple-pattern.cdz

```cadenza
match pair
    (0, y) -> y
    (x, 0) -> x
    (x, y) -> x + y
```

**Expected AST:**

```
[
    [match, pair,
        [__arm__, [__tuple__, 0, y], y],
        [__arm__, [__tuple__, x, 0], x],
        [__arm__, [__tuple__, x, y], [+, x, y]]],
]
```

#### Test: match-with-guard.cdz

```cadenza
match n
    x if x > 0 -> "positive"
    x if x < 0 -> "negative"
    _ -> "zero"
```

**Expected AST:**

```
[
    [match, n,
        [__arm__, x, [if, [>, x, 0]], "positive"],
        [__arm__, x, [if, [<, x, 0]], "negative"],
        [__arm__, _, "zero"]],
]
```

#### Test: match-nested.cdz

```cadenza
match result
    Ok value ->
        match value
            Some x -> x
            None -> default
    Err e -> handle_error e
```

**Expected AST:**

```
[
    [match, result,
        [__arm__, [Ok, value],
            [match, value,
                [__arm__, [Some, x], x],
                [__arm__, None, default]]],
        [__arm__, [Err, e], [handle_error, e]]],
]
```

---

## Issue 9: If/Else Expressions

### Current State

There is no special handling for `if`/`then`/`else` expressions. Since Cadenza has no keywords, these would be parsed as identifiers with function application, but the ternary conditional structure is not recognized.

### What Needs To Be Done

1. Recognize `if condition then consequent else alternative` as a ternary pattern
2. Support indented bodies after `then` and `else`
3. Represent if/else as: `Apply(if, [condition, consequent, alternative])`
4. Support `elif` chains by nesting: `if c1 then e1 elif c2 then e2 else e3`
5. Handle if without else (optional, depends on language design)

### Test Cases

#### Test: if-simple.cdz

```cadenza
if x > 0 then "positive" else "non-positive"
```

**Expected AST:**

```
[
    [if, [>, x, 0], "positive", "non-positive"],
]
```

#### Test: if-with-blocks.cdz

```cadenza
if condition then
    do_something
    result
else
    do_other
    other_result
```

**Expected AST:**

```
[
    [if, condition,
        [__block__, do_something, result],
        [__block__, do_other, other_result]],
]
```

#### Test: if-nested.cdz

```cadenza
if a then if b then 1 else 2 else 3
```

**Expected AST:**

```
[
    [if, a, [if, b, 1, 2], 3],
]
```

#### Test: if-elif-else.cdz

```cadenza
if x > 0 then "positive" elif x < 0 then "negative" else "zero"
```

**Expected AST:**

```
[
    [if, [>, x, 0], "positive",
        [if, [<, x, 0], "negative", "zero"]],
]
```

#### Test: if-in-expr.cdz

```cadenza
let result = if x > 0 then x else 0 - x
```

**Expected AST:**

```
[
    [=, [let, result], [if, [>, x, 0], x, [-, 0, x]]],
]
```

#### Test: if-with-complex-condition.cdz

```cadenza
if a && b || c then x else y
```

**Expected AST:**

```
[
    [if, [||, [&&, a, b], c], x, y],
]
```

#### Test: if-multiline.cdz

```cadenza
if condition
    then result1
    else result2
```

**Expected AST:**

```
[
    [if, condition, result1, result2],
]
```

---

## Issue 10: Closures and Functions

### Current State

The lexer recognizes `->` (`Arrow`) and `fn` would be parsed as an identifier. The parser does not currently handle closure or function syntax.

### Design Considerations

**Key distinction: Functions vs Closures**

1. **Functions** (`fn`) - Named, hoisted like Rust

   - Can be called before they are defined in the source
   - No need for pre-declaration
   - Top-level or nested definitions

2. **Closures** - Anonymous, not hoisted
   - Bound to variables via `let`
   - Must be defined before use (follows `let` ordering rules)
   - Can capture surrounding scope

This distinction allows Rust-like ergonomics where you can organize code freely (functions), while maintaining clear data flow for closures.

### What Needs To Be Done

1. Add `fn` as the prefix for function definitions (hoisted)
2. Design closure syntax (not hoisted, follows `let` rules)
3. Use `->` to separate parameters from body in both cases
4. Parse parameter lists (with optional type annotations)
5. Handle whitespace-based blocks for multi-line bodies
6. Represent functions as: `Apply(fn, [name, params, body])`
7. Represent closures differently to indicate no hoisting

### Syntax Design (Pending)

**Functions (hoisted):**

```cadenza
# Named function - can be called anywhere in scope
fn double x -> x * 2

# Multiple parameters
fn add x y -> x + y

# Or with tuple-style parameters
fn add (x, y) -> x + y

# Multi-line function body
fn complex x y ->
    let sum = x + y
    let product = x * y
    (sum, product)

# Functions can call each other in any order
fn is_even n -> if n == 0 then true else is_odd (n - 1)
fn is_odd n -> if n == 0 then false else is_even (n - 1)
```

**Closures (not hoisted, anonymous):**

```cadenza
# Option A: No prefix, just arrow (like Haskell/ML lambdas)
let double = x -> x * 2

# Option B: Use a different symbol/keyword
let double = \x -> x * 2      # Haskell-style backslash
let double = |x| x * 2        # Rust-style pipes
let double = fn x -> x * 2    # Reuse fn for consistency

# Closure capturing scope
let multiplier = 3
let triple = x -> x * multiplier

# Closure in higher-order function
map (x -> x * 2) list
filter (x -> x > 0) numbers
```

### Open Questions

1. **Closure syntax**: Should closures use:

   - Bare arrow: `x -> x + 1` (clean but may be ambiguous)
   - Backslash: `\x -> x + 1` (Haskell-like, `\` looks odd)
   - Pipes: `|x| x + 1` (Rust-like)
   - `fn` keyword: `fn x -> x + 1` (consistent with functions)

2. **Function name position**: Should the name come before or after `fn`?

   - `fn double x -> ...` (name first, like current examples)
   - `double = fn x -> ...` (assignment style, but then hoisting is weird)

3. **Parameter syntax**: Whitespace-separated vs tuple-style?
   - `fn add x y -> x + y` (curried by default)
   - `fn add (x, y) -> x + y` (explicit tuple)

### Test Cases

_Note: Syntax is pending design decisions. Examples use current best guess._

#### Test: fn-simple.cdz

```cadenza
fn double x -> x * 2
```

**Expected AST:**

```
[
    [fn, double, x, [*, x, 2]],
]
```

#### Test: fn-multi-param.cdz

```cadenza
fn add x y -> x + y
```

**Expected AST:**

```
[
    [fn, add, x, y, [+, x, y]],
]
```

#### Test: fn-tuple-param.cdz

```cadenza
fn swap (x, y) -> (y, x)
```

**Expected AST:**

```
[
    [fn, swap, [__tuple__, x, y], [__tuple__, y, x]],
]
```

#### Test: fn-block.cdz

```cadenza
fn complex x y ->
    let sum = x + y
    let product = x * y
    (sum, product)
```

**Expected AST:**

```
[
    [fn, complex, x, y, [__block__,
        [=, [let, sum], [+, x, y]],
        [=, [let, product], [*, x, y]],
        [__tuple__, sum, product]]],
]
```

#### Test: fn-mutual-recursion.cdz

```cadenza
# These can call each other because fn is hoisted
fn is_even n -> if n == 0 then true else is_odd (n - 1)
fn is_odd n -> if n == 0 then false else is_even (n - 1)
```

**Expected AST:**

```
[
    [fn, is_even, n, [if, [==, n, 0], true, [is_odd, [-, n, 1]]]],
    [fn, is_odd, n, [if, [==, n, 0], false, [is_even, [-, n, 1]]]],
]
```

#### Test: closure-simple.cdz

```cadenza
let double = x -> x * 2
```

**Expected AST:**

```
[
    [=, [let, double], [->, x, [*, x, 2]]],
]
```

#### Test: closure-in-call.cdz

```cadenza
map (x -> x * 2) list
```

**Expected AST:**

```
[
    [[map, [->, x, [*, x, 2]]], list],
]
```

#### Test: closure-capturing.cdz

```cadenza
let multiplier = 3
let triple = x -> x * multiplier
```

**Expected AST:**

```
[
    [=, [let, multiplier], 3],
    [=, [let, triple], [->, x, [*, x, multiplier]]],
]
```

#### Test: fn-curried.cdz

```cadenza
fn add x -> y -> x + y
```

**Expected AST:**

```
[
    [fn, add, x, [->, y, [+, x, y]]],
]
```

---

## Issue 11: Loops (while, for, loop)

### Current State

The parser does not handle any loop constructs. Since Cadenza has no keywords, `while`, `for`, and `loop` would be parsed as identifiers.

### Design Considerations

**Mutability vs Immutability:**
Supporting loops implies supporting mutability, as pure functional loops via recursion can be cumbersome for practical use. The decision is to support mutability for ergonomics.

**Rust-style loop constructs:**

1. `loop` - Infinite loop, exits via `break`
2. `while condition` - Loop while condition is true
3. `for pattern in iterable` - Iterator-based loop

### What Needs To Be Done

1. Parse `loop` with a block body
2. Parse `while` with condition and block body
3. Parse `for` with pattern, `in` keyword, iterable expression, and block body
4. Add `break` and `continue` for loop control
5. Optionally support loop labels for nested loop control
6. Represent loops using Apply nodes

### Syntax Design

```cadenza
# Infinite loop
loop
    do_something
    if done then break

# While loop
while condition
    do_something
    update_state

# For loop over iterator
for x in collection
    process x

# For loop with pattern destructuring
for (key, value) in map
    print key value

# Loop with label (for nested loops)
outer: loop
    inner: for x in items
        if x == target then break outer

# Break with value (loop as expression)
let result = loop
    let x = try_something
    if x.is_ok then break x.value
```

### Test Cases

_Note: Syntax follows Rust conventions but without explicit keywords._

#### Test: loop-simple.cdz

```cadenza
loop
    do_something
```

**Expected AST:**

```
[
    [loop, [__block__, do_something]],
]
```

#### Test: loop-break.cdz

```cadenza
loop
    if done then break
    do_something
```

**Expected AST:**

```
[
    [loop, [__block__,
        [if, done, break],
        do_something]],
]
```

#### Test: while-simple.cdz

```cadenza
while x > 0
    x = x - 1
```

**Expected AST:**

```
[
    [while, [>, x, 0], [__block__,
        [=, x, [-, x, 1]]]],
]
```

#### Test: for-simple.cdz

```cadenza
for x in items
    process x
```

**Expected AST:**

```
[
    [for, x, in, items, [__block__,
        [process, x]]],
]
```

#### Test: for-pattern.cdz

```cadenza
for (i, x) in enumerate items
    print i x
```

**Expected AST:**

```
[
    [for, [__tuple__, i, x], in, [enumerate, items], [__block__,
        [[print, i], x]]],
]
```

#### Test: loop-break-value.cdz

```cadenza
let result = loop
    let x = try_get
    if x.is_some then break x.value
```

**Expected AST:**

```
[
    [=, [let, result], [loop, [__block__,
        [=, [let, x], try_get],
        [if, [., x, is_some], [break, [., x, value]]]]]],
]
```

#### Test: while-with-continue.cdz

```cadenza
while has_more
    let item = get_next
    if skip item then continue
    process item
```

**Expected AST:**

```
[
    [while, has_more, [__block__,
        [=, [let, item], get_next],
        [if, [skip, item], continue],
        [process, item]]],
]
```

### Open Questions

1. **Loop labels**: Should we support labels for breaking out of nested loops? Syntax could be `label: loop` or `loop @label`.

2. **Break with value**: Should `loop` be an expression that can return a value via `break value`?

3. **For loop syntax**: Should `in` be a keyword or can it be parsed contextually as part of the `for` construct?

4. **Range-based for**: Should `for i in 0..10` be supported natively or rely on range iterators?

---

## Issue 12: Partial Function Application (`&`)

### Current State

The lexer recognizes `&` (`Ampersand`) token, but it's currently designated for bitwise AND and reference operations. This feature proposes using `&` as a prefix operator for partial function application.

### Design Overview

Partial function application allows capturing a function with some arguments pre-applied, creating a new function that takes the remaining arguments.

**Syntax forms:**

1. `&foo` - Capture function `foo` as a value
2. `&foo arg1 arg2` - Partially apply `arg1` and `arg2` to `foo`
3. `&foo arg1 $0 arg3 $1` - Partial application with positional holes for remaining arguments

### What Needs To Be Done

1. Add `&` as a prefix operator for partial application (distinct from bitwise AND which is infix)
2. Parse the function name and any following arguments
3. Handle `$0`, `$1`, `$2`, etc. as positional placeholders for argument holes (allows reordering)
4. Represent as: `Apply(__partial__, [fn, arg1, arg2, ...])` or `Apply(&, [fn, arg1, arg2, ...])`
5. Handle holes: `Apply(__partial__, [fn, arg1, $0, arg3, $1])`

### Syntax Design

```cadenza
# Simple function capture
let f = &add
# f is now a callable that behaves like add

# Partial application with arguments
let add5 = &add 5
# add5 is now a function that takes one arg and adds 5 to it

# Multiple partial arguments
let result = &foo 1 2 3
# Applies 1, 2, 3 to foo, returns partially applied function if foo takes more args

# Argument holes with $0, $1, etc.
let middle = &substring 0 $0 10
# Creates a function that takes one arg (the string) in the middle position

# Multiple holes with explicit ordering
let between = &clamp $0 0 100 $1
# Creates a function taking 2 args: $0 is the value, $1 is the high bound
# Usage: between value high  =>  clamp value 0 100 high

# Reordering arguments
let flipped = &foo $1 $0
# Swaps the order of arguments

# Use in higher-order functions
map (&add 1) list
# Adds 1 to each element

filter (&greater_than $0 0) numbers
# Filter numbers greater than 0
```

### Test Cases

#### Test: partial-simple.cdz

```cadenza
&foo
```

**Expected AST:**

```
[
    [&, foo],
]
```

#### Test: partial-with-arg.cdz

```cadenza
&add 5
```

**Expected AST:**

```
[
    [&, add, 5],
]
```

#### Test: partial-multi-args.cdz

```cadenza
&foo 1 2 3
```

**Expected AST:**

```
[
    [&, foo, 1, 2, 3],
]
```

#### Test: partial-with-hole.cdz

```cadenza
&foo 1 $0 3
```

**Expected AST:**

```
[
    [&, foo, 1, $0, 3],
]
```

#### Test: partial-multi-holes.cdz

```cadenza
&clamp $0 0 100 $1
```

**Expected AST:**

```
[
    [&, clamp, $0, 0, 100, $1],
]
```

#### Test: partial-reorder.cdz

```cadenza
&foo $1 $0
```

**Expected AST:**

```
[
    [&, foo, $1, $0],
]
```

#### Test: partial-in-call.cdz

```cadenza
map (&add 1) list
```

**Expected AST:**

```
[
    [[map, [&, add, 1]], list],
]
```

#### Test: partial-in-pipeline.cdz

```cadenza
numbers |> filter (&gt $0 0) |> map (&mul 2)
```

**Expected AST:**

```
[
    [|>, [|>, numbers, [filter, [&, gt, $0, 0]]], [map, [&, mul, 2]]],
]
```

#### Test: partial-with-expr.cdz

```cadenza
&foo (a + b) bar (c * d)
```

**Expected AST:**

```
[
    [&, foo, [+, a, b], bar, [*, c, d]],
]
```

### Open Questions

1. **Precedence**: How does `&` interact with other operators? Should `&foo 1 + 2` parse as `(&foo 1) + 2` or `&foo (1 + 2)`?

2. **Distinction from reference**: Since `&` is also used for references/bitwise AND, should partial application use a different syntax like `&>`, `@`, or `\`?

3. **Evaluation**: Are arguments to `&foo arg` evaluated eagerly or lazily?

4. **Hole validation**: Should the parser validate that holes are sequential ($0, $1, $2) or allow gaps?

---

## Issue 13: Error Recovery and Parser Robustness

### Current State

The parser currently has weak error handling:

1. **Overly permissive token acceptance**: In `parse_primary()`, the catch-all `_ => { self.bump(); }` accepts any token, including invalid ones like `)`, `]`, `}`, `,`, etc.

2. **Missing error nodes**: When invalid syntax is encountered, the parser doesn't consistently create `Error` nodes to mark problematic regions.

3. **No synchronization**: After encountering an error, the parser doesn't have a strategy to recover and continue parsing subsequent valid code.

4. **Limited negative tests**: There are no tests verifying that invalid syntax is properly rejected and reported.

5. **Missing error propagation in delimited contexts**: When a dedented expression is encountered inside an array/bracket (e.g., `foo [\nbar` where `bar` is at root indentation), the parser correctly bails out of the array but `bar` ends up as an ApplyArgument of `foo` instead of being a separate top-level expression. The error should propagate up through the expression tree to properly recover. See `test-data/invalid-parse/array-dedent-recovery.cdz` for the test case.

### What Needs To Be Done

1. **Restrict valid primary tokens**: Only accept tokens that can start an expression:

   - Identifiers
   - Literals (Integer, Float, StringStart)
   - Opening delimiters (`(`, `[`, `{`)
   - Prefix operators (`-`, `!`, `~`, `&`, `*`)
   - Quote/unquote operators

2. **Create Error nodes**: When an unexpected token is encountered, wrap it in an `Error` node and record a parse error.

3. **Implement synchronization**: After an error, skip tokens until a synchronization point (newline at base indentation, closing delimiter, etc.)

4. **Add negative test cases**: Comprehensive tests for invalid syntax that verify proper error reporting.

### Implementation Details

```rust
fn parse_primary(&mut self) {
    match self.current() {
        Kind::Identifier => {
            self.bump();
        }
        Kind::Integer | Kind::Float => {
            self.parse_literal();
        }
        Kind::StringStart => {
            self.parse_string();
        }
        Kind::LParen => {
            self.parse_expression(LParenMarker::new(self));
        }
        Kind::LBracket => {
            self.parse_array();
        }
        Kind::LBrace => {
            self.parse_record();
        }
        // Prefix operators
        Kind::Minus | Kind::Bang | Kind::Tilde | Kind::Ampersand | Kind::Star => {
            self.parse_prefix();
        }
        // Invalid tokens - create error node
        Kind::RParen | Kind::RBracket | Kind::RBrace | Kind::Comma | Kind::Semicolon => {
            self.error_unexpected_token("expression");
        }
        Kind::Eof => {
            self.error("unexpected end of file");
        }
        _ => {
            // Unknown token - try to recover
            self.error_unexpected_token("expression");
            self.bump(); // consume the invalid token
        }
    }
}

fn error_unexpected_token(&mut self, expected: &str) {
    self.builder.start_node(Kind::Error.into());
    let token = self.current();
    self.error(&format!("expected {}, found {:?}", expected, token));
    self.bump();
    self.builder.finish_node();
}
```

### Test Cases - Negative Tests

These tests verify that invalid syntax produces appropriate errors.

#### Test: error-unexpected-rparen.cdz

```cadenza
)
```

**Expected:** Parse error "expected expression, found RParen"
**Expected AST:**

```
[
    Error(")"),
]
```

#### Test: error-unexpected-rbracket.cdz

```cadenza
]
```

**Expected:** Parse error "expected expression, found RBracket"
**Expected AST:**

```
[
    Error("]"),
]
```

#### Test: error-unexpected-rbrace.cdz

```cadenza
}
```

**Expected:** Parse error "expected expression, found RBrace"
**Expected AST:**

```
[
    Error("}"),
]
```

#### Test: error-unexpected-comma.cdz

```cadenza
,
```

**Expected:** Parse error "expected expression, found Comma"
**Expected AST:**

```
[
    Error(","),
]
```

#### Test: error-unclosed-paren.cdz

```cadenza
(a + b
```

**Expected:** Parse error "expected closing parenthesis"
**Expected AST:**

```
[
    [+, a, b],
]
```

#### Test: error-unclosed-bracket.cdz

```cadenza
[1, 2, 3
```

**Expected:** Parse error "expected closing bracket"
**Expected AST:**

```
[
    [__list__, 1, 2, 3],
]
```

#### Test: error-extra-rparen.cdz

```cadenza
(a + b))
```

**Expected:** Parse error on second `)`
**Expected AST:**

```
[
    [+, a, b],
    Error(")"),
]
```

#### Test: error-mismatched-delimiters.cdz

```cadenza
(a + b]
```

**Expected:** Parse error "expected ), found ]"
**Expected AST:**

```
[
    [+, a, b],
]
```

#### Test: error-recovery-next-line.cdz

```cadenza
let x = )
let y = 5
```

**Expected:** Error on first line, successful parse of second line
**Expected AST:**

```
[
    [=, [let, x], Error(")")],
    [=, [let, y], 5],
]
```

#### Test: error-recovery-multiple.cdz

```cadenza
let a = 1
)
let b = 2
]
let c = 3
```

**Expected:** Errors on lines 2 and 4, successful parse of other lines
**Expected AST:**

```
[
    [=, [let, a], 1],
    Error(")"),
    [=, [let, b], 2],
    Error("]"),
    [=, [let, c], 3],
]
```

#### Test: error-empty-parens-call.cdz

```cadenza
foo()
```

**Expected:** Either valid (if empty args allowed) or error
**Note:** This depends on design decision about empty argument lists

#### Test: error-double-operator.cdz

```cadenza
a + + b
```

**Expected:** Parse error "expected expression, found +"
**Expected AST:**

```
[
    [+, a, Error("+"), b],
]
```

#### Test: error-trailing-operator.cdz

```cadenza
a +
```

**Expected:** Parse error "expected expression after operator"
**Expected AST:**

```
[
    [+, a, Error],
]
```

#### Test: error-leading-binary-operator.cdz

```cadenza
* a
```

**Expected:** Error if `*` is not a valid prefix operator in this context
**Note:** Depends on whether `*` is used for dereference

### Open Questions

1. **Error recovery strategy**: Should we use panic mode (skip until sync point) or phrase-level recovery (insert/delete tokens)?

2. **Sync points**: What tokens should be synchronization points? Options:

   - Newline at base indentation level
   - Keywords like `let`, `fn`, `if`
   - Closing delimiters

3. **Error message quality**: How detailed should error messages be? Should we suggest fixes?

4. **Multiple errors**: Should we try to report as many errors as possible in one pass, or stop at the first error?

5. **Error node granularity**: Should each invalid token get its own `Error` node, or should we group consecutive errors?

---

## Issue 14: String Interpolation and Multi-line Strings

### Current State

The lexer handles basic strings with `StringStart`, `StringContent`/`StringContentWithEscape`, and `StringEnd` tokens. However, there is no support for:

1. String interpolation (embedding expressions in strings)
2. Heredoc-style multi-line strings with automatic indentation stripping

### Design Overview

**String Interpolation:**
Use `:` prefix before a string to enable interpolation with `${expr}` syntax:

```cadenza
:"hello ${name}"
```

**Multi-line Strings:**
When a string contains a newline after the opening quote, treat it as a heredoc:

- Remove the initial newline
- Strip common leading indentation from all lines
- The lexer emits a token per line, parser handles indentation stripping

### What Needs To Be Done

1. **Add interpolated string prefix** - `:` before `"` triggers interpolation mode
2. **Parse `${expr}` within strings** - Switch to expression parsing inside `${...}`
3. **Multi-line string handling**:
   - Detect newline after opening quote
   - Emit `StringLine` tokens instead of `StringContent`
   - Parser strips leading indentation based on its indentation tracking
4. **New token types**:
   - `InterpolatedStringStart` (`:"` )
   - `InterpolationStart` (`${`)
   - `InterpolationEnd` (`}`)
   - `StringLine` (for multi-line content)

### Syntax Design

**String Interpolation:**

```cadenza
# Basic interpolation
let greeting = :"hello ${name}"

# Expression in interpolation
let result = :"the answer is ${x + y}"

# Multiple interpolations
let message = :"${greeting}, you have ${count} messages"

# Nested expressions
let complex = :"value: ${if condition then a else b}"

# Interpolation with method calls
let info = :"user: ${user.name}, age: ${user.age}"
```

**Multi-line Strings (Heredoc style):**

```cadenza
# Multi-line string - initial newline is removed
let text = "
    This is line 1
    This is line 2
    This is line 3
"
# Result: "This is line 1\nThis is line 2\nThis is line 3\n"
# (indentation stripped based on first content line)

# Multi-line with interpolation
let html = :"
    <div>
        <h1>${title}</h1>
        <p>${content}</p>
    </div>
"

# Indentation is preserved relative to first line
let code = "
    fn main() {
        println!("hello")
    }
"
# Preserves the relative indentation of the code
```

### Test Cases

#### Test: interp-simple.cdz

```cadenza
:"hello ${name}"
```

**Expected AST:**

```
[
    [__interp__, "hello ", name],
]
```

#### Test: interp-expr.cdz

```cadenza
:"result: ${a + b}"
```

**Expected AST:**

```
[
    [__interp__, "result: ", [+, a, b]],
]
```

#### Test: interp-multiple.cdz

```cadenza
:"${x} and ${y}"
```

**Expected AST:**

```
[
    [__interp__, "", x, " and ", y],
]
```

#### Test: interp-nested.cdz

```cadenza
:"value: ${if cond then a else b}"
```

**Expected AST:**

```
[
    [__interp__, "value: ", [if, cond, a, b]],
]
```

#### Test: multiline-simple.cdz

```cadenza
"
    line 1
    line 2
"
```

**Expected AST:**

```
[
    "line 1\nline 2\n",
]
```

#### Test: multiline-indent-preserved.cdz

```cadenza
"
    outer
        inner
    outer again
"
```

**Expected AST:**

```
[
    "outer\n    inner\nouter again\n",
]
```

#### Test: multiline-interp.cdz

```cadenza
:"
    Hello ${name}
    Welcome to ${place}
"
```

**Expected AST:**

```
[
    [__interp__, "Hello ", name, "\nWelcome to ", place, "\n"],
]
```

#### Test: interp-escape.cdz

```cadenza
:"use \${literal} for literal"
```

**Expected AST:**

```
[
    [__interp__, "use ${literal} for literal"],
]
```

### Implementation Details

**Lexer changes:**

```rust
// In lexer, when we see `:` followed by `"`
fn lex_string(&mut self) {
    let is_interpolated = self.current == ':' && self.peek() == '"';
    if is_interpolated {
        self.emit(Kind::InterpolatedStringStart);
        self.advance(); // skip ':'
    } else {
        self.emit(Kind::StringStart);
    }
    self.advance(); // skip '"'

    // Check for multi-line (newline immediately after opening quote)
    let is_multiline = self.current == '\n';

    if is_multiline {
        self.lex_multiline_string(is_interpolated);
    } else {
        self.lex_inline_string(is_interpolated);
    }
}

fn lex_inline_string(&mut self, interpolated: bool) {
    loop {
        match self.current {
            '"' => {
                self.emit(Kind::StringEnd);
                break;
            }
            '$' if interpolated && self.peek() == '{' => {
                self.emit(Kind::InterpolationStart);
                self.advance(); self.advance();
                // Parser will handle expression parsing
                return;
            }
            '\\' => {
                // Handle escapes including \${
                self.lex_escape();
            }
            _ => {
                self.lex_string_content();
            }
        }
    }
}
```

**Parser changes:**

- When parsing `InterpolatedStringStart`, collect alternating string parts and expressions
- For multi-line strings, track indentation and strip common prefix
- Represent as `Apply(__interp__, [part1, expr1, part2, expr2, ...])`

### Open Questions

1. **Interpolation syntax**: Is `:"..."` the right prefix? Alternatives:

   - `$"..."` (C#-style)
   - `f"..."` (Python-style)
   - `\`...\`` (JavaScript template literals)

2. **Escape in interpolation**: Should `\${` escape the interpolation, or use `$${`?

3. **Expression restrictions**: Should any expression be allowed in `${}`, or only simple ones?

4. **Multi-line indentation**: Should indentation be stripped based on:

   - First content line's indentation
   - Minimum indentation of all lines
   - Closing quote's indentation

5. **Trailing newline**: Should the final newline before closing quote be included or stripped?

---

## Implementation Priority

Based on dependencies and common usage patterns, the suggested implementation order is:

1. **Complete Operator Support** - Add missing operators (bitwise, shifts, prefix unary, etc.)
2. **Error Recovery** - Critical for usability; reject invalid tokens, create Error nodes
3. **Quote/Unquote** - Decide on syntax first (blocking issue), then implement
4. **Closures and Functions** - Core language feature; requires design decision on fn vs closure syntax
5. **Partial Function Application** - `&foo arg` syntax with positional holes ($0, $1, etc.)
6. **String Interpolation** - `:"hello ${name}"` syntax with multi-line heredoc support
7. **Tuples** - Foundation for destructuring and multiple returns
8. **Array Literals** - Basic data structure
9. **Array Indexing** - Access array elements
10. **Record Field Access** - Dot notation is fundamental
11. **Record Creation** - Struct literals
12. **If/Else Expressions** - Control flow
13. **Loops** - while, for, loop with break/continue
14. **Match Expressions** - Pattern matching (most complex)

## Notes on AST Representation

Per the design document, all constructs use the `Apply` node type:

- `__quote__` - Quote operator receiver (syntax capture)
- `__unquote__` - Unquote operator receiver (syntax splice)
- `__list__` - Array literal receiver
- `__tuple__` - Tuple literal receiver
- `__record__` - Record literal receiver
- `__index__` - Array indexing receiver
- `__arm__` - Match arm receiver
- `__block__` - Indented block receiver
- `__partial__` - Partial function application (alternative to using `&` directly)
- `$0`, `$1`, `$2`, etc. - Positional placeholders for partial application holes
- `__interp__` - Interpolated string with alternating string parts and expressions

The parser tracks indentation through the `Whitespace` struct and uses the `Marker` trait to handle delimited expressions (parentheses, brackets, braces).

## Open Questions

1. **Quote/Unquote Syntax**: The single quote (`'`) for quoting may conflict with other uses. Need to decide between:

   - Backtick (`` ` ``) for quote, tilde (`~`) for unquote
   - Backtick for quote, dollar (`$`) for unquote
   - Some other combination

2. **Exponentiation Operator**: Cadenza uses `**` for exponentiation since `^` is reserved for bitwise XOR. This follows the convention of doubling the operator for "super" versions (like `||` for logical OR vs `|` for bitwise OR).

3. **Bitwise vs Logical Operators**: Should Cadenza follow the C/Rust convention of `|` for bitwise OR and `||` for logical OR, `&` for bitwise AND and `&&` for logical AND? This is consistent with most languages but could cause confusion.
4. **Empty Tuple vs Unit**: Should `()` be an empty tuple or a unit type? Most languages treat them the same.

5. **Record vs Struct**: Are records structural or nominal? This affects pattern matching.

6. **Match Exhaustiveness**: Should the parser validate exhaustive patterns or leave that to semantic analysis?

7. **Range Operator Precedence**: Rust has complex rules for range operators - ranges have very low precedence but cannot be used as operands without parentheses. For example, `1..2 + 3` in Rust is a syntax error. Options:
   - Follow Rust's approach (require parentheses in ambiguous contexts)
   - Give range operators low precedence so `1..2 + 3` parses as `1..(2 + 3)`
   - Give range operators high precedence so `1..2 + 3` parses as `(1..2) + 3`
