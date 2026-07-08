; Literals — each denotes a value directly, with a statically determined type
; (type-system.md) and a canonical value form (contracts/deterministic-value-form.md).
;
; Cases are s-expressions in the canonical homoiconic representation. A result is
; written as (: <value> <Type>). See README.md for the case vocabulary.

(case "a decimal integer literal"
  (input  42)
  (output (: 42 Int64)))

(case "an integer literal with digit separators"
  (input  1_000_000)
  (output (: 1000000 Int64)))

; A digit separator `_` is meaningful ONLY between two digits (the between-digits rule the sections
; below pin for the leading-underscore boundary and that the radix cases restate). A `_` that is NOT
; between two digits — a TRAILING separator `1_` (a digit to its left, none to its right), a doubled
; `1__0` (the second `_` sits between a `_` and a digit, not two digits), or a trailing group `1_000_` —
; is therefore malformed, not a well-formed literal with the stray `_` silently dropped. A digit-led
; token is a number (the number/identifier boundary below), and a malformed number is a well-formedness
; rejection (CDZ0201), the same class as an out-of-range literal — never silently normalized to the
; digits with the separator removed. A reader that strips every `_` regardless of position reads `1_`
; as the value 1, silently accepting a malformed literal; the between-digits rule requires a separator
; to have a digit on BOTH sides.

(case "a trailing digit separator is a malformed literal, not the digits with it dropped"
  (doc    "`1_` has a digit separator with a digit on its left but none on its right — not BETWEEN two
           digits, so it is not a well-formed separator (contrast `1_000_000`, where every `_` sits
           between digits). A digit-led token is a number, and a number with a stray separator is
           malformed (CDZ0201), the same well-formedness class as an out-of-range literal below — never
           silently read as the value 1 with the `_` dropped. Pins that the digit-separator rule is
           between-digits in BOTH directions, so a reader cannot accept a trailing (or doubled) separator
           by stripping every `_`.")
  (input  1_)
  (error  CDZ0201))

; The between-digits rule holds for a FLOAT literal too, not only an integer — a `_` must sit between two
; digits, so one adjacent to the decimal point (or trailing, or doubled) is malformed. `1._5` puts the
; separator between the `.` and `5` — the digit on its left is missing (a `.`, not a digit) — so it is not
; a well-formed separator, exactly as the integer `1_` is not; the compiler MUST reject it (CDZ0201) rather
; than silently drop the `_` and read `1.5`. Same for a trailing `1.5_`, a before-dot `1_.5`, a doubled
; `1.5__0`, and a stray `_` in the exponent (`1.5e_10`). A valid float separator sits between digits
; (`1_000.5`, `1.2_5`) and is accepted. A reader that strips every `_` from a float token regardless of
; position accepts these malformed forms — the between-digits rule must be applied to the float lexer as it
; is to the integer lexer (the integer case is pinned above; this pins the float sibling was not left out).

(case "a digit separator adjacent to a float's decimal point is a malformed literal"
  (doc    "`1._5` places a digit separator between the decimal point and `5` — the `_` has no digit on its
           left (a `.`, not a digit), so it is not BETWEEN two digits and is malformed (CDZ0201), exactly as
           the integer `1_` above. The compiler MUST reject it rather than silently drop the `_` and read
           `1.5`. Pins that the between-digits separator rule is applied to FLOAT literals too, not only
           integers — a reader that strips every `_` from a float token regardless of position accepts this
           and other misplaced forms (`1.5_`, `1_.5`, `1.5__0`, `1.5e_10`). The valid float separator
           `1.2_5` (between digits) is accepted; only a misplaced one is rejected.")
  (input  1._5)
  (error  CDZ0201))

; --- The number / identifier boundary ---------------------------------------------------
; A token's classification as a numeric literal vs. an identifier is a lexical rule, and the
; digit-separator rule (above) must not swallow identifiers that merely contain the separator
; character. An identifier may begin with `_` (the wildcard `_` and names like `_x` are
; identifiers); a digit separator `_` is only meaningful BETWEEN digits. So a token beginning
; with `_` is an identifier, never a number — its leading `_` is not a separator with a digit
; on each side. These cases pin that boundary so the reader cannot misclassify a `_`-prefixed
; name as an integer (which would make such a name unbindable). The reader is not in the
; compiler's trusted path (contracts/ast-encoding.md), but it is the seed toolchain's front
; door and must classify tokens correctly for every generation that reads source text.

(case "an underscore-prefixed token is an identifier, not an integer"
  (doc    "`_1` begins with `_`, so it is an identifier — bindable like any name — not the integer
           1 with a stray leading separator. Bound to 99 and referenced, it yields 99. Contrast the
           digit-separator case above, where `_` sits BETWEEN digits; a leading `_` has no digit to
           its left, so it is not a separator and the token is a name. (Companion control below:
           `_x`, an unambiguous name, already works — `_1` must behave the same.)")
  (input  (let ((_1 99)) _1))
  (output (: 99 Int64)))

(case "an underscore-letter identifier binds and resolves"
  (doc    "The control for the case above: `_x` is unambiguously an identifier (no digits), binds to
           99, and resolves to 99. `_1` must classify the same way — as a name.")
  (input  (let ((_x 99)) _x))
  (output (: 99 Int64)))

; The other end of the number/identifier boundary: an all-digit token that is numeric in shape but
; outside the Int64 range. The reader must not fall through to treating an unparseable all-digit
; token as an identifier — a digit-led token is a number, and a number out of range is a malformed
; literal (a type/well-formedness rejection), never a reference to a name. Misclassifying it as a
; name surfaces the misleading "unbound name" diagnostic (CDZ0101) for what is plainly a number.

(case "the maximum Int64 literal reads as an integer"
  (doc    "9223372036854775807 is Int64.max — the largest value the checked Int64 default holds. It
           reads as an integer and is its own value. The companion below pins that one past this
           boundary is an out-of-range literal, not an identifier.")
  (input  9223372036854775807)
  (output (: 9223372036854775807 Int64)))

(case "the minimum Int64 literal reads as an integer"
  (doc    "-9223372036854775808 is Int64.min — the smallest value the checked Int64 default holds.
           It reads as an integer and is its own value.")
  (input  -9223372036854775808)
  (output (: -9223372036854775808 Int64)))

(case "an out-of-range integer literal is a malformed literal, not an unbound name"
  (doc    "9223372036854775808 is Int64.max + 1: all digits, no letters, plainly a number and not a
           name. A digit-led all-digit token is a numeric literal; a numeric literal outside the
           Int64 range is malformed (a well-formedness/type rejection, CDZ0201), NOT a reference to
           a name. The reader must not fall back to Node::Name when the token fails to parse as an
           i64 — doing so surfaces the misleading `unbound name` diagnostic (CDZ0101) for a number.
           Same reader-boundary class as the `_1`-is-a-name cases above, at the high end.")
  (input    9223372036854775808)
  (error    CDZ0201))

(case "an underscore-prefixed function parameter binds its argument"
  (doc    "A parameter named `_1` is an identifier, so `(def (f _1) (+ _1 1))` binds the argument to
           `_1`; f(41) = 42. If the reader misclassified `_1` as the integer 1, the parameter list
           `(f _1)` would carry a number where a name is required and the def would be ill-formed —
           so this case pins the token as a name via its binding behavior.")
  (input  (module m
            (def (f _1) (+ _1 1))
            (def (main) (f 41))))
  (output (: 42 Int64)))

; --- Radix-prefixed integer literals: hexadecimal (0x) and binary (0b) --------------------
; A hexadecimal literal `0x…` and a binary literal `0b…` are alternate RADICES for the same Int64
; value a decimal literal denotes — the radix is a lexical convenience, not a distinct type or value:
; `0xFF`, `0b1111_1111`, and `255` all denote the one Int64 255, whose canonical value form serializes
; the integer, not its source spelling (contracts/deterministic-value-form.md). A hex literal's digits
; are case-insensitive (`0xAB` = `0xab`), and the pinned digit-separator `_` composes with both radices
; (`0b1010_1010`, `0xFF_FF`) under the same between-digits rule as decimal (the reader treats `_` as
; meaningful only between digits). A radix literal is STRICT NON-NEGATIVE: it denotes its face value in
; `0..=Int64.max`, exactly as decimal does — a literal always means its value, so a leading `-` is
; ordinary negation (`-0xFF` = -255) and a radix literal that overflows Int64 is a malformed literal
; (CDZ0201), never a two's-complement bit pattern that turns a positive-looking literal negative. (The
; all-ones mask idiom belongs to the wrapping/unsigned numeric layer, a deferred numeric-model
; capability, not to a literal's meaning.) These are CORE cases — every generation, including the seed,
; reads them — the same reader-boundary class as the decimal out-of-range and `_`-prefixed cases above.

(case "a hexadecimal integer literal"
  (doc    "`0xFF` denotes the Int64 255 — hex is another radix for the same integer value a decimal
           literal denotes, not a distinct type. Pins that the reader classifies a `0x`-prefixed
           digit-led token as a numeric literal, not as a name.")
  (input  0xFF)
  (output (: 255 Int64)))

(case "a hexadecimal literal is case-insensitive"
  (doc    "`0xab` and `0xAB` denote the one value 171: hex digits `a`–`f` are case-insensitive. Pins that
           digit case does not change a hex literal's value.")
  (input  (= 0xab 0xAB))
  (output (: true Bool)))

(case "a binary integer literal"
  (doc    "`0b1010` denotes the Int64 10 — binary is another radix for the same integer value. Pins that
           the reader classifies a `0b`-prefixed token as a numeric literal.")
  (input  0b1010)
  (output (: 10 Int64)))

(case "a binary literal with digit separators"
  (doc    "`0b1010_1010` denotes 170: the digit separator `_` composes with the binary radix under the
           same between-digits rule as decimal (`1_000_000`). Pins that separators group binary digits.")
  (input  0b1010_1010)
  (output (: 170 Int64)))

(case "a hexadecimal literal with digit separators"
  (doc    "`0xFF_FF` denotes 65535: the separator groups hex digits, exactly as it groups decimal and
           binary digits. Pins separator composition with the hex radix.")
  (input  0xFF_FF)
  (output (: 65535 Int64)))

(case "hexadecimal, binary, and decimal spellings of one value are equal"
  (doc    "`0x2A`, `0b101010`, and `42` all denote the one Int64 42 — the radix is a spelling of the
           source, erased in the value. Pins that a value's identity is independent of the radix it was
           written in, so pattern matching and equality see one value across spellings.")
  (input  (= 0x2A 0b101010))
  (output (: true Bool)))

(case "a negative hexadecimal literal negates its value"
  (doc    "`-0xFF` denotes -255: a leading `-` is ordinary negation of the literal's face value, not a
           bit pattern. Pins that a radix literal is strict non-negative and the sign is applied on top,
           consistent with decimal `-255`.")
  (input  -0xFF)
  (output (: -255 Int64)))

(case "the maximum Int64 in hexadecimal reads as an integer"
  (doc    "`0x7FFFFFFFFFFFFFFF` is Int64.max (9223372036854775807), the largest value the checked Int64
           default holds — it reads as that integer. The companion below pins that the next hex value up,
           which would set the sign bit, is out of range rather than a negative two's-complement pattern.")
  (input  0x7FFFFFFFFFFFFFFF)
  (output (: 9223372036854775807 Int64)))

(case "a hexadecimal literal past Int64.max is a malformed literal, not a bit pattern"
  (doc    "`0xFFFFFFFFFFFFFFFF` is 18446744073709551615 = Int64.max·2+1, outside the Int64 range. Under
           the strict-non-negative rule a radix literal denotes its face value, so this overflows and is
           a malformed literal (CDZ0201) — NOT -1 via a 64-bit two's-complement reinterpretation, and NOT
           an unbound name (the reader must classify a `0x`-prefixed token as numeric, so the honest
           diagnostic is out-of-range, not CDZ0101). Same reader-boundary class as the decimal
           out-of-range case above, at the radix boundary.")
  (input  0xFFFFFFFFFFFFFFFF)
  (error  CDZ0201))

(case "a floating-point literal"
  (input  3.5)
  (output (: 3.5 Float64)))

(case "a large whole-valued float renders its full value, not an integer saturation"
  (doc    "Witnesses contracts/deterministic-value-form.md #Numeric Values Serialize Deterministically
           (2nd/3rd sentences: floats equal under structural equality share a canonical form, and
           distinct floats have DISTINCT canonical forms). 1e19 is a whole-valued Float64 just beyond
           the Int64 range (2^63 ≈ 9.22e18). Its canonical form is its full decimal value
           `10000000000000000000.0`, NOT the Int64-saturated `9223372036854775807.0` a display that
           routes a whole float through an `as i64` cast produces — such a cast collapses EVERY float
           at or beyond 2^63 (1e19, 1e20, 1e100, 1.5e300 …) to one string, violating the
           distinct-canonical-form requirement. The underlying values are already distinct (their
           structural equality is false), so this pins that the SERIALIZED form is distinct too.")
  (input  1e19)
  (output (: 10000000000000000000.0 Float64)))

(case "distinct large floats are not equal"
  (doc    "Companion witnessing contracts/deterministic-value-form.md: 1e19 and 1e20 are distinct
           Float64 values, so structural equality is false — the values are held to full precision,
           not clamped to a shared saturated representation. (This is the value-level counterpart of
           the canonical-form case above: distinct values, distinct serializations.)")
  (input  (= 1e19 1e20))
  (output (: false Bool)))

(case "negative zero is distinct in the canonical value form"
  (doc    "The canonical value form distinguishes -0.0 from 0.0; a canonical NaN is separate.")
  (input  -0.0)
  (output (: -0.0 Float64)))

(case "the boolean literals"
  (input  true)
  (output (: true Bool)))

(case "the unit value"
  (doc    "Witnesses core-semantics.md #An Effect-Only Expression Yields The Unit Value: `unit` denotes
           the unit value, the normal-termination value of a program that produces nothing else
           (\"A program that terminates normally without producing a value other than through its
           emitted events MUST produce the unit value as its normal-termination value\"). It is a
           first-class result that must cross the run boundary — the effect-only programs in
           03-equality/04-capabilities all terminate in it. This bare-`unit` case pins that a program
           whose result IS unit runs and yields unit, independent of any capability.")
  (input  unit)
  (output (: unit Unit)))

(case "unit and the empty tuple are the same value"
  (doc    "Witnesses core-semantics.md #The Empty Tuple Is The Unit Value (\"unit and () are the same
           value\"): the empty tuple `()` denotes exactly the unit value, so `(= unit ())` is true and
           each yields the unit value as a program result.")
  (input  (= unit ()))
  (output (: true Bool)))

(case "a string literal"
  (input  "hello")
  (output (: "hello" String)))

(case "a string literal is normalized to the canonical text form"
  (doc    "Stored in the pinned text normalization form (options/hashing-and-encoding/),
           so two literals differing only in normalization are one value.")
  (input  "café")
  (output (: "café" String)))

; --- A string literal's escape sequences are a closed set -----------------------------------
; collections-and-text.md #A String Literal's Escapes Are A Closed Set: within a string literal a
; backslash introduces an escape sequence, and a conforming reader recognizes exactly `\n` (U+000A),
; `\t` (U+0009), `\r` (U+000D), `\\` (U+005C), and `\"` (U+0022). A backslash before any other
; character is a compile-time error, so an unrecognized escape is a rejected program rather than a
; silently-dropped backslash or an implementation-defined character. These pin the recognized set (the
; escape denotes the one scalar value it names, so `(= "\t" <a literal tab>)` is true) and the
; rejection of an unrecognized escape.

(case "a recognized string escape denotes its one scalar value"
  (doc    "`\"\\t\"` is the one-scalar string containing a tab (U+0009): the reader expands the escape,
           so it equals a literal that contains an actual tab character (collections-and-text.md #A
           String Literal's Escapes Are A Closed Set). Witnesses that `\\t` is recognized and denotes
           exactly U+0009, not the two characters backslash-t.")
  (input  (= "\t" "	"))
  (output (: true Bool)))

(case "an unrecognized string escape is rejected"
  (doc    "`\"\\q\"` uses a backslash before `q`, which begins none of the recognized escape sequences,
           so the reader MUST reject it at compile time (collections-and-text.md #A String Literal's
           Escapes Are A Closed Set) rather than drop the backslash and read `q` (length 1) or emit an
           implementation-defined character. Carries `(needs strict-escapes)`: the seed's reader today
           accepts an unknown escape as the bare character, so it SKIPS this case until a generation
           enforces the closed set; a later generation rejects the program.")
  (needs  strict-escapes)
  (input  "\q")
  (error  CDZ0001))
