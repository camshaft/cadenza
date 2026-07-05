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
