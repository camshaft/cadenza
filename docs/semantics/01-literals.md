# Literals

Literals are the simplest expressions - they evaluate to themselves.

## Integer Literals

Whole numbers without a decimal point or fraction.

### Syntax

```
[-] digit+
```

Underscores can be used for readability and are ignored: `1_000_000`

### Type

`Integer` - 128-bit signed integer. Sizes are further constrained based on subtyping.

### Test: Simple integer

**Input:**

```cadenza
42
```

**Output:**

```repl
42 : Integer
```

### Test: Zero

**Input:**

```cadenza
0
```

**Output:**

```repl
0 : Integer
```

### Test: Negative integer

**Input:**

```cadenza
-5
```

**Output:**

```repl
-5 : Integer
```

### Test: Integer with underscores

**Input:**

```cadenza
1_000_000
```

**Output:**

```repl
1_000_000 : Integer
```

**Notes:** Underscores are stripped during parsing for readability

### Test: ERROR - Integer overflow

**Input:**

```cadenza
999999999999999999999999999999999999999999
```

**Output:**

```
error: integer literal is too large
 --> test.cdz:1:1
  |
1 | 999999999999999999999999999999999999999999
  | ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ exceeds maximum 128-bit integer value
  |
  = note: maximum value is 170141183460469231731687303715884105727
  = help: consider using a BigInt type for arbitrarily large integers
```

**Notes:** Integer literals are limited to 128-bit signed integer range

---

## Float Literals

Numbers with a decimal point.

### Syntax

```
[-] digit+ '.' digit+ [e|E [+|-] digit+]
```

Scientific notation supported: `1.5e10`, `3.2e-5`

### Type

`Float` - 64-bit IEEE 754 floating point

### Test: Simple float

**Input:**

```cadenza
3.14159
```

**Output:**

```repl
3.14159 : Float
```

### Test: Float with leading zero

**Input:**

```cadenza
0.5
```

**Output:**

```repl
0.5 : Float
```

### Test: Integer-looking float

**Input:**

```cadenza
1.0
```

**Output:**

```repl
1.0 : Float
```

**Notes:** The decimal point makes it a float, even if the fractional part is zero

---

## Rational Literals

Rational numbers represent exact fractions. Unlike floats, they preserve precision without rounding errors. They're created from integer division or explicit fraction notation.

### Syntax

```
<integer> / <integer>        # Division creates rational
```

There's no direct literal syntax - rationals are created from operations or explicit construction.

### Semantics

When two integers are divided, the result is a rational number representing the exact fraction. Rationals are automatically simplified to lowest terms.

### Type

`Rational` - An exact fraction represented as numerator and denominator (both arbitrary precision integers).

### Test: Integer division creates rational

**Input:**

```cadenza
1 / 2
```

**Output:**

```repl
1/2 : Rational
```

**Notes:** Integer division produces a rational, not a truncated integer or float

### Test: Rational simplification

**Input:**

```cadenza
6 / 4
```

**Output:**

```repl
3/2 : Rational
```

**Notes:** Rationals are automatically simplified to lowest terms

### Test: Division by one

**Input:**

```cadenza
5 / 1
```

**Output:**

```repl
5 : Rational
```

**Notes:** When denominator is 1, simplifies to an integer

### Test: Whole number division

**Input:**

```cadenza
10 / 5
```

**Output:**

```repl
2 : Rational
```

**Notes:** When the division is exact, result is an integer

### Test: Rational arithmetic

**Input:**

```cadenza
1/2 + 1/3
```

**Output:**

```repl
5/6 : Rational
```

**Notes:** Rational addition finds common denominator: 3/6 + 2/6 = 5/6

### Test: Rational multiplication

**Input:**

```cadenza
2/3 * 3/4
```

**Output:**

```repl
1/2 : Rational
```

**Notes:** (2×3)/(3×4) = 6/12 = 1/2 (simplified)

### Test: Mixed arithmetic (rational + integer)

**Input:**

```cadenza
1/2 + 1
```

**Output:**

```repl
3/2 : Rational
```

**Notes:** Integers automatically promote to rationals in mixed operations

### Test: Mixed arithmetic (rational + float)

**Input:**

```cadenza
1/2 + 0.5
```

**Output:**

```repl
1.0 : Float
```

**Notes:** Operations with floats produce floats (precision lost)

### Test: Rational to float conversion

**Input:**

```cadenza
Float.from_rational 1/3
```

**Output:**

```repl
0.333333... : Float
```

**Notes:** Explicit conversion to float when approximation is acceptable

### Test: Comparing rationals

**Input:**

```cadenza
1/2 > 1/3
```

**Output:**

```repl
true : Bool
```

**Notes:** Rational comparison is exact (no floating point issues)

### Test: ERROR - Division by zero

**Input:**

```cadenza
1 / 0
```

**Output:**

```
error: division by zero
 --> test.cdz:1:1
  |
1 | 1 / 0
  | ^^^^^ cannot divide by zero
  |
  = note: division by zero is undefined
```

### Numeric Tower

Cadenza uses a numeric tower where types automatically promote:

```
Integer < Rational < Float
```

Operations return the most precise type that can represent the result:

- Integer ops → Integer (if exact)
- Integer division → Rational (preserves exactness)
- Rational ops → Rational (if exact)
- Any op with Float → Float (precision lost)

### Test: Numeric tower promotion

**Input:**

```cadenza
let a = 1          # Integer
let b = 1 / 2      # Rational
let c = 1.0        # Float
typeof (a + b)
typeof (b + c)
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
Rational : Type
Float : Type
```

**Notes:** Integer promotes to Rational, Rational promotes to Float

---

## Character Literals

A single Unicode character enclosed in single quotes.

Character literals represent individual Unicode code points. They are distinct from single-character strings - a character is a primitive scalar value while a string is a sequence.

### Syntax

```
' character '
```

Escape sequences are supported: `'\n'` (newline), `'\t'` (tab), `'\''` (single quote), `'\\'` (backslash), `'\u{XXXX}'` (Unicode code point).

### Type

`Char` - A single Unicode scalar value (U+0000 to U+D7FF and U+E000 to U+10FFFF)

### Test: Simple character

**Input:**

```cadenza
'a'
```

**Output:**

```repl
'a' : Char
```

### Test: Digit character

**Input:**

```cadenza
'7'
```

**Output:**

```repl
'7' : Char
```

### Test: Unicode character

**Input:**

```cadenza
'λ'
```

**Output:**

```repl
'λ' : Char
```

### Test: Escaped newline

**Input:**

```cadenza
'\n'
```

**Output:**

```repl
'\n' : Char
```

### Test: Unicode escape sequence

**Input:**

```cadenza
'\u{03BB}'
```

**Output:**

```repl
'λ' : Char
```

**Notes:** Unicode escape sequences allow specifying characters by their code point in hexadecimal

### Test: Unicode escape with emoji

**Input:**

```cadenza
'\u{1F30D}'
```

**Output:**

```repl
'🌍' : Char
```

### Test: ERROR - Unclosed character literal

**Input:**

```cadenza
'a
```

**Output:**

```
error: unterminated character literal
 --> test.cdz:1:1
  |
1 | 'a
  | ^^ missing closing single quote
  |
  = note: character literals must be closed on the same line
```

**Notes:** Character literals must be closed before the end of the line

### Test: ERROR - Empty character literal

**Input:**

```cadenza
''
```

**Output:**

```
error: empty character literal
 --> test.cdz:1:1
  |
1 | ''
  | ^^ this character literal is empty
  |
  = note: character literals must contain exactly one character
  = help: try using \"\" for an empty string instead
```

**Notes:** A character literal must contain exactly one character

### Test: ERROR - Multiple characters

**Input:**

```cadenza
'ab'
```

**Output:**

```
error: character literal contains multiple characters
 --> test.cdz:1:1
  |
1 | 'ab'
  | ^^^^ contains 2 characters
  |
  = note: character literals can only contain a single Unicode scalar value
  = help: use a string literal "ab" for multiple characters
```

**Notes:** Character literals are for single characters only; use strings for sequences

### Test: ERROR - Invalid escape sequence

**Input:**

```cadenza
'\x'
```

**Output:**

```
error: unknown escape sequence
 --> test.cdz:1:2
  |
1 | '\x'
  |   ^^ unknown escape: \x
  |
  = note: valid escape sequences are: \n \t \r \\ \' \u{...}
  = help: use \\ to include a literal backslash
```

**Notes:** Only defined escape sequences are recognized

---

## String Literals

Strings are sequences of characters enclosed in double quotes. They can span multiple lines and contain any Unicode text.

### Syntax

Single-line strings use double quotes:

```
" character* "
```

Multi-line strings also use double quotes and preserve all whitespace including newlines. The string continues until the closing quote is found:

```
" character*
  character*
  character* "
```

Escape sequences: `\n` (newline), `\t` (tab), `\"` (quote), `\\` (backslash), `\u{XXXX}` (Unicode code point).

### Type

`String` - UTF-8 encoded text, dynamically sized

### Test: Simple string

**Input:**

```cadenza
"hello"
```

**Output:**

```repl
"hello" : String
```

### Test: Empty string

**Input:**

```cadenza
""
```

**Output:**

```repl
"" : String
```

### Test: String with spaces

**Input:**

```cadenza
"hello world"
```

**Output:**

```repl
"hello world" : String
```

### Test: Multi-line string

**Input:**

```cadenza
"line one
line two
line three"
```

**Output:**

```repl
"line one
line two
line three" : String
```

**Notes:** Multi-line strings preserve all whitespace including newlines exactly as written

### Test: Multi-line string with indentation

**Input:**

```cadenza
"  indented line 1
  indented line 2
    more indented"
```

**Output:**

```repl
"  indented line 1
  indented line 2
    more indented" : String
```

**Notes:** All indentation and spacing is preserved in the string literal

### Test: String with Unicode

**Input:**

```cadenza
"Hello, 世界! 🌍"
```

**Output:**

```repl
"Hello, 世界! 🌍" : String
```

**Notes:** Strings support full Unicode including emoji and non-Latin scripts

### Test: String with mathematical symbols

**Input:**

```cadenza
"∀x ∈ ℝ: x² ≥ 0"
```

**Output:**

```repl
"∀x ∈ ℝ: x² ≥ 0" : String
```

### Test: String with escape sequences

**Input:**

```cadenza
"Line 1\nLine 2\tTabbed"
```

**Output:**

```repl
"Line 1
Line 2	Tabbed" : String
```

**Notes:** Escape sequences like `\n` and `\t` are processed into their actual characters

### Test: String with quote escape

**Input:**

```cadenza
"She said \"hello\""
```

**Output:**

```repl
"She said \"hello\"" : String
```

### Test: Empty multi-line string

**Input:**

```cadenza
"

"
```

**Output:**

```repl
"

" : String
```

**Notes:** Even strings that appear empty may contain whitespace characters like newlines

### Test: Unicode escape in string

**Input:**

```cadenza
"Greek letter: \u{03BB}"
```

**Output:**

```repl
"Greek letter: λ" : String
```

**Notes:** Unicode escapes work in strings just like in character literals

### Test: ERROR - Unterminated string

**Input:**

```cadenza
"hello world
```

**Output:**

```
error: unterminated string literal
 --> test.cdz:1:1
  |
1 | "hello world
  |             ^ missing closing double quote
  |
  = note: string literals must have a closing quote
  = help: try adding a closing "
```

**Notes:** Even multi-line strings need a closing quote

### Test: ERROR - Invalid escape in string

**Input:**

```cadenza
"hello\xworld"
```

**Output:**

```
error: unknown escape sequence
 --> test.cdz:1:7
  |
1 | "hello\xworld"
  |       ^^ unknown escape: \x
  |
  = note: valid escape sequences are: \n \t \r \\ \" \u{...}
  = help: use \\ to include a literal backslash
```

**Notes:** The same escape sequences work in both characters and strings

---

## Boolean Literals

Truth values: `true` and `false`

### Syntax

```
true | false
```

### Type

`Bool`

### Test: True literal

**Input:**

```cadenza
true
```

**Output:**

```repl
true : Bool
```

### Test: False literal

**Input:**

```cadenza
false
```

**Output:**

```repl
false : Bool
```

---

## Compiler Queries

Literals require:

- `eval` - Direct evaluation rule: literal always evaluates to itself
- `typeof` - Type is determined by literal form
- `lifetime` - Literals are static values

## Implementation Notes

- Literals are the simplest query rules - they just return their value
- No environment lookup needed
- No recursion needed
