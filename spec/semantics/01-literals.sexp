; Literals — each denotes a value directly, with a statically determined type
; (type-system.md) and a canonical value form (contracts/deterministic-value-form.md).
;
; Cases are s-expressions in the canonical homoiconic representation. A result is
; written as (: <value> <Type>). See README.md for the case vocabulary.
(case "a decimal integer literal" (input 42) (output (: 42 Int64)))

(case "an integer literal with digit separators" (input 1000000) (output (: 1000000 Int64)))

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
(case
  "a trailing digit separator is a malformed literal, not the digits with it dropped"
  (doc
    "`1_` has a digit separator with a digit on its left but none on its right — not BETWEEN two
           digits, so it is not a well-formed separator (contrast `1_000_000`, where every `_` sits
           between digits). A digit-led token is a number, and a number with a stray separator is
           malformed (CDZ0201), the same well-formedness class as an out-of-range literal below — never
           silently read as the value 1 with the `_` dropped. Pins that the digit-separator rule is
           between-digits in BOTH directions, so a reader cannot accept a trailing (or doubled) separator
           by stripping every `_`.")
  (input 1_)
  (error CDZ0201))

; The between-digits rule holds for a FLOAT literal too, not only an integer — a `_` must sit between two
; digits, so one adjacent to the decimal point (or trailing, or doubled) is malformed. `1._5` puts the
; separator between the `.` and `5` — the digit on its left is missing (a `.`, not a digit) — so it is not
; a well-formed separator, exactly as the integer `1_` is not; the compiler MUST reject it (CDZ0201) rather
; than silently drop the `_` and read `1.5`. Same for a trailing `1.5_`, a before-dot `1_.5`, a doubled
; `1.5__0`, and a stray `_` in the exponent (`1.5e_10`). A valid float separator sits between digits
; (`1_000.5`, `1.2_5`) and is accepted. A reader that strips every `_` from a float token regardless of
; position accepts these malformed forms — the between-digits rule must be applied to the float lexer as it
; is to the integer lexer (the integer case is pinned above; this pins the float sibling was not left out).
(case
  "a digit separator adjacent to a float's decimal point is a malformed literal"
  (doc
    "`1._5` places a digit separator between the decimal point and `5` — the `_` has no digit on its
           left (a `.`, not a digit), so it is not BETWEEN two digits and is malformed (CDZ0201), exactly as
           the integer `1_` above. The compiler MUST reject it rather than silently drop the `_` and read
           `1.5`. Pins that the between-digits separator rule is applied to FLOAT literals too, not only
           integers — a reader that strips every `_` from a float token regardless of position accepts this
           and other misplaced forms (`1.5_`, `1_.5`, `1.5__0`, `1.5e_10`). The valid float separator
           `1.2_5` (between digits) is accepted; only a misplaced one is rejected.")
  (input 1._5)
  (error CDZ0201))

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
(case
  "an underscore-prefixed token is an identifier, not an integer"
  (doc
    "`_1` begins with `_`, so it is an identifier — bindable like any name — not the integer
           1 with a stray leading separator. Bound to 99 and referenced, it yields 99. Contrast the
           digit-separator case above, where `_` sits BETWEEN digits; a leading `_` has no digit to
           its left, so it is not a separator and the token is a name. (Companion control below:
           `_x`, an unambiguous name, already works — `_1` must behave the same.)")
  (input (let ((_1 99)) _1))
  (output (: 99 Int64)))

(case
  "an underscore-letter identifier binds and resolves"
  (doc
    "The control for the case above: `_x` is unambiguously an identifier (no digits), binds to
           99, and resolves to 99. `_1` must classify the same way — as a name.")
  (input (let ((_x 99)) _x))
  (output (: 99 Int64)))

; The other end of the number/identifier boundary: an all-digit token that is numeric in shape but
; outside the Int64 range. The reader must not fall through to treating an unparseable all-digit
; token as an identifier — a digit-led token is a number, and a number out of range is a malformed
; literal (a type/well-formedness rejection), never a reference to a name. Misclassifying it as a
; name surfaces the misleading "unbound name" diagnostic (CDZ0101) for what is plainly a number.
(case
  "the maximum Int64 literal reads as an integer"
  (doc
    "9223372036854775807 is Int64.max — the largest value the checked Int64 default holds. It
           reads as an integer and is its own value. The companion below pins that one past this
           boundary is an out-of-range literal, not an identifier.")
  (input 9223372036854775807)
  (output (: 9223372036854775807 Int64)))

(case
  "the minimum Int64 literal reads as an integer"
  (doc
    "-9223372036854775808 is Int64.min — the smallest value the checked Int64 default holds.
           It reads as an integer and is its own value.")
  (input -9223372036854775808)
  (output (: -9223372036854775808 Int64)))

(case
  "an out-of-range integer literal is a malformed literal, not an unbound name"
  (doc
    "9223372036854775808 is Int64.max + 1: all digits, no letters, plainly a number and not a
           name. A digit-led all-digit token is a numeric literal; a numeric literal outside the
           Int64 range is malformed (a well-formedness/type rejection, CDZ0201), NOT a reference to
           a name. The reader must not fall back to Node::Name when the token fails to parse as an
           i64 — doing so surfaces the misleading `unbound name` diagnostic (CDZ0101) for a number.
           Same reader-boundary class as the `_1`-is-a-name cases above, at the high end.")
  (input 9223372036854775808)
  (error CDZ0201))

; The out-of-range case above is one shape of a digit-led token that fails to parse; the RADIX and
; FORMAT malformations are the others, and they classify the same way — a MALFORMED LITERAL (CDZ0201),
; never an `unbound name` (CDZ0101). A digit-led token is a number: an unsupported radix (`0o…` —
; only `0x`/`0b` are radices), an empty radix body (`0x`, `0b`), a bad radix digit (`0xGG`, `0b12`), or
; a decimal token trailing letters (`123abc`) all fail the numeric parse, and the reader classifies
; them as bare names — but the well-formedness call rejects a digit-led name as a malformed literal,
; not a reference to a name. These pin the radix/format siblings of the out-of-range case above.
(case
  "an octal-prefixed token is a malformed literal, not an unbound name"
  (doc
    "`0o17` is digit-led, so it is a numeric token — but octal is not a supported radix (only
           `0x`/`0b`), so it fails to parse and is a MALFORMED literal (CDZ0201), never `unbound name`
           (CDZ0101). A digit-led token is a number; the reader classifying it as a name does not make
           it an identifier. The reject message says `malformed numeric literal`, not `unbound name`.")
  (input 0o17)
  (error CDZ0201 (message "malformed numeric literal")))

; A LEXICAL well-formedness poison a bare leaf resolves to (a malformed numeric literal, an out-of-range
; float, a char naming a non-scalar) is a defect of the TOKEN, independent of reachability — like an unbound
; name. So `check` must surface it in EVERY body, including a PARAMETERIZED body or a NON-EXPORTED nullary
; def (neither reached by the standalone emit walk), not only a reached one — else a malformed literal there
; would pass `check` while `compile` rejects it on a reached body. And a reachable body reports the fault
; EXACTLY ONCE (the infer + emit copies dedup). (Migrated from rcdzc
; a_lexical_well_formedness_fault_surfaces_in_an_unreached_body.)
(case
  "a malformed literal in a parameterized (unreached) body still surfaces"
  (input (do (def (g (: n Int64)) (+ n 0o17)) (export g)))
  (error CDZ0201 (message "malformed numeric literal")))

(case
  "a non-scalar char in a parameterized (unreached) body still surfaces"
  (input (do (def (g (: n Int64)) (if (= n 0) #\u+D800 #\a)) (export g)))
  (error CDZ0002))

(case
  "a malformed literal in a non-exported nullary def body still surfaces"
  (input (do (def (f) 0o17) (def (main) 1) (export main)))
  (error CDZ0201 (message "malformed numeric literal")))

(case
  "a reachable malformed literal reports exactly once, not doubled"
  (input (do (def (main) 0o17) (export main)))
  (error CDZ0201 (message "malformed numeric literal") (count 1)))

(case
  "a bad binary digit is a malformed literal, not an unbound name"
  (doc
    "`0b12` is `0b`-prefixed but `2` is not a binary digit (only 0/1) — a malformed binary literal
           (CDZ0201), the binary sibling of the bad-hex `0xGG` case. A digit-led radix token with an
           out-of-alphabet digit is a malformed number, never an identifier.")
  (input 0b12)
  (error CDZ0201 (message "malformed numeric literal")))

(case
  "a radix literal with an empty body is a malformed literal, not an unbound name"
  (doc
    "`0x` has the hexadecimal prefix but no digits — a malformed radix literal (CDZ0201), not an
           `unbound name`. The prefix commits the token to being a number; with no body it is a
           well-formedness rejection.")
  (input 0x)
  (error CDZ0201))

(case
  "a radix literal with a bad digit is a malformed literal, not an unbound name"
  (doc
    "`0xGG` is `0x`-prefixed but `G` is not a hexadecimal digit — a malformed hex literal
           (CDZ0201). A digit-led radix token with an out-of-alphabet digit is a malformed number, not
           a name.")
  (input 0xGG)
  (error CDZ0201))

(case
  "a decimal token trailing letters is a malformed literal, not an unbound name"
  (doc
    "`123abc` is digit-led with trailing letters — an identifier may not start with a digit, so
           this is a malformed numeric token (CDZ0201), not an identifier `123abc`. Reporting it as
           `unbound name` (CDZ0101) is the misleading diagnostic this boundary forbids.")
  (input 123abc)
  (error CDZ0201))

; The `N` suffix means BigInt (a whole number), so it is malformed on a FLOAT-form literal — a common
; suffix slip. Rather than the bare "malformed numeric literal", the reject explains the cause (`N` = BigInt,
; spelled as a plain integer) and suggests the `R` Rational suffix, carried as a structural replace fix
; (`0.5N` → `0.5R`) so `cdz fix` applies it. Covers the decimal-fraction and exponent float forms. (Migrated
; from rcdzc a_malformed_digit_led_token_is_a_malformed_literal_not_an_unbound_name — the N-suffix-slip tail.)
(case
  "a float-form literal with an N (BigInt) suffix explains the slip and suggests the R suffix"
  (input 0.5N)
  (error CDZ0201 (message "the `N` suffix means BigInt") (fix (kind replace) (replacement "0.5R"))))

(case
  "an integer-valued float with an N suffix also gets the R-suffix fix"
  (input 2.0N)
  (error CDZ0201 (message "the `N` suffix means BigInt") (fix (kind replace) (replacement "2.0R"))))

(case
  "an exponent-form float with an N suffix gets the R-suffix fix"
  (input 1e3N)
  (error CDZ0201 (message "the `N` suffix means BigInt") (fix (kind replace) (replacement "1e3R"))))

(case
  "an N-suffixed token with a NON-numeric body gets the generic malformed message, not the N-suffix explanation"
  (input 12xN)
  (error CDZ0201 (message "malformed numeric literal") (not "the `N` suffix means BigInt")))

(case
  "an underscore-prefixed function parameter binds its argument"
  (doc
    "A parameter named `_1` is an identifier, so `(def (f _1) (+ _1 1))` binds the argument to
           `_1`; f(41) = 42. If the reader misclassified `_1` as the integer 1, the parameter list
           `(f _1)` would carry a number where a name is required and the def would be ill-formed —
           so this case pins the token as a name via its binding behavior.")
  (input (do (def (f _1) (+ _1 1)) (def (main) (f 41)) (export main)))
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
(case
  "a hexadecimal integer literal"
  (doc
    "`0xFF` denotes the Int64 255 — hex is another radix for the same integer value a decimal
           literal denotes, not a distinct type. Pins that the reader classifies a `0x`-prefixed
           digit-led token as a numeric literal, not as a name.")
  (input 0xff)
  (output (: 255 Int64)))

(case
  "a hexadecimal literal is case-insensitive"
  (doc
    "`0xab` and `0xAB` denote the one value 171: hex digits `a`–`f` are case-insensitive. Pins that
           digit case does not change a hex literal's value.")
  (input (= 0xab 0xab))
  (output (: true Bool)))

(case
  "a binary integer literal"
  (doc
    "`0b1010` denotes the Int64 10 — binary is another radix for the same integer value. Pins that
           the reader classifies a `0b`-prefixed token as a numeric literal.")
  (input 0b1010)
  (output (: 10 Int64)))

(case
  "a binary literal with digit separators"
  (doc
    "`0b1010_1010` denotes 170: the digit separator `_` composes with the binary radix under the
           same between-digits rule as decimal (`1_000_000`). Pins that separators group binary digits.")
  (input 0b10101010)
  (output (: 170 Int64)))

(case
  "a hexadecimal literal with digit separators"
  (doc
    "`0xFF_FF` denotes 65535: the separator groups hex digits, exactly as it groups decimal and
           binary digits. Pins separator composition with the hex radix.")
  (input 0xffff)
  (output (: 65535 Int64)))

(case
  "hexadecimal, binary, and decimal spellings of one value are equal"
  (doc
    "`0x2A`, `0b101010`, and `42` all denote the one Int64 42 — the radix is a spelling of the
           source, erased in the value. Pins that a value's identity is independent of the radix it was
           written in, so pattern matching and equality see one value across spellings.")
  (input (= 0x2a 0b101010))
  (output (: true Bool)))

(case
  "a negative hexadecimal literal negates its value"
  (doc
    "`-0xFF` denotes -255: a leading `-` is ordinary negation of the literal's face value, not a
           bit pattern. Pins that a radix literal is strict non-negative and the sign is applied on top,
           consistent with decimal `-255`.")
  (input -0xff)
  (output (: -255 Int64)))

(case
  "the maximum Int64 in hexadecimal reads as an integer"
  (doc
    "`0x7FFFFFFFFFFFFFFF` is Int64.max (9223372036854775807), the largest value the checked Int64
           default holds — it reads as that integer. The companion below pins that the next hex value up,
           which would set the sign bit, is out of range rather than a negative two's-complement pattern.")
  (input 0x7fffffffffffffff)
  (output (: 9223372036854775807 Int64)))

(case
  "a hexadecimal literal past Int64.max is a malformed literal, not a bit pattern"
  (doc
    "`0xFFFFFFFFFFFFFFFF` is 18446744073709551615 = Int64.max·2+1, outside the Int64 range. Under
           the strict-non-negative rule a radix literal denotes its face value, so this overflows and is
           a malformed literal (CDZ0201) — NOT -1 via a 64-bit two's-complement reinterpretation, and NOT
           an unbound name (the reader must classify a `0x`-prefixed token as numeric, so the honest
           diagnostic is out-of-range, not CDZ0101). Same reader-boundary class as the decimal
           out-of-range case above, at the radix boundary.")
  (input 0xffffffffffffffff)
  (error CDZ0201))

(case "a floating-point literal" (input 3.5) (output (: 3.5 Float64)))

(case
  "a scientific-notation float literal denotes the scaled value"
  (doc
    "`1.5e3` is the Float64 1500.0 — a fractional mantissa with a decimal exponent, the `<digits>.
           <digits>e<exp>` form (distinct from the integer-mantissa `1e19` cases below, which have no
           decimal point). Pinned by value equality against the plain decimal so it does not depend on the
           exact rendered form: the scientific literal and `1500.0` are the same Float64.")
  (input (= 1500.0 1500.0))
  (output (: true Bool)))

(case
  "the exponent marker is case-insensitive"
  (doc
    "`1.5E3` (uppercase `E`) reads as the same value as `1.5e3` (lowercase `e`) — the exponent marker
           is case-insensitive, exactly as a hexadecimal literal's digits are (`0xFF` = `0xff`). Pins that
           the lexer accepts both spellings of the exponent as one value, so a source using either case
           denotes the same float.")
  (input (= 1500.0 1500.0))
  (output (: true Bool)))

(case
  "a negative exponent scales the mantissa down"
  (doc
    "`2.5e-2` is 0.025 — a negative exponent divides by the power of ten (a distinct sign path in the
           exponent from the non-negative `1.5e3`). Pinned by equality against `0.025`. Pins that the `e-`
           form reads the fractional value, not a malformed token or a sign dropped.")
  (input (= 0.025 0.025))
  (output (: true Bool)))

(case
  "a large whole-valued float renders its full value, not an integer saturation"
  (doc
    "Witnesses contracts/deterministic-value-form.md #Numeric Values Serialize Deterministically
           (2nd/3rd sentences: floats equal under structural equality share a canonical form, and
           distinct floats have DISTINCT canonical forms). 1e19 is a whole-valued Float64 just beyond
           the Int64 range (2^63 ≈ 9.22e18). Its canonical form is its full decimal value
           `10000000000000000000.0`, NOT the Int64-saturated `9223372036854775807.0` a display that
           routes a whole float through an `as i64` cast produces — such a cast collapses EVERY float
           at or beyond 2^63 (1e19, 1e20, 1e100, 1.5e300 …) to one string, violating the
           distinct-canonical-form requirement. The underlying values are already distinct (their
           structural equality is false), so this pins that the SERIALIZED form is distinct too.")
  (input 10000000000000000000.0)
  (output (: 10000000000000000000.0 Float64)))

(case
  "distinct large floats are not equal"
  (doc
    "Companion witnessing contracts/deterministic-value-form.md: 1e19 and 1e20 are distinct
           Float64 values, so structural equality is false — the values are held to full precision,
           not clamped to a shared saturated representation. (This is the value-level counterpart of
           the canonical-form case above: distinct values, distinct serializations.)")
  (input (= 10000000000000000000.0 100000000000000000000.0))
  (output (: false Bool)))

(case
  "negative zero is distinct in the canonical value form"
  (doc "The canonical value form distinguishes -0.0 from 0.0; a canonical NaN is separate.")
  (input -0.0)
  (output (: -0.0 Float64)))

(case
  "the canonical NaN crosses the boundary as a not-a-number float"
  (doc
    "`Float64.nan` is the canonical not-a-number value (core-semantics.md #Floating-Point Equality
           Follows The Canonical Byte Form). Returned as the program result it crosses the component
           boundary as an IEEE f64 NaN — it is NOT saturated, mangled, or dropped — and renders as the
           canonical `nan` (the round-trippable value form the canonical binary-AST printer emits; seq-287
           routed render through it, retiring the old `NaN` spelling). Pins that a returned NaN survives the
           export marshalling as a genuine NaN value, the not-a-number companion of the -0.0/large-float cases.")
  (input (do (def (main) Float64.nan) (export main)))
  (call main)
  (output (: nan Float64)))

; A float literal whose magnitude exceeds the largest FINITE Float64 (~1.8e308) denotes no
; representable value: rounding it to the nearest binary64 gives an infinity, and the language provides
; no `inf` spelling, so the value would have no written form that reads back. numeric-model.md §"A
; Floating-Point Literal That Denotes No Representable Value Is Malformed" makes it a malformed literal
; (CDZ0201) at the reader boundary — the exact float analogue of the out-of-range INTEGER literal
; `9223372036854775808` above. This closes the prior spec gap where `1e400` silently produced `inf`.
(case
  "an out-of-range float literal is a malformed literal, not a non-finite value"
  (doc
    "1e400 is far past Float64.max (~1.8e308): rounding it to binary64 gives +infinity, which has
           no written form the reader accepts. A digit-led token with a `.`/exponent is a float literal;
           a float literal outside the finite range is malformed (CDZ0201), NOT silently saturated to a
           non-finite value nor a name. The float companion of the `9223372036854775808` out-of-range
           integer case — the reader classifies it as numeric and rejects the magnitude, rather than
           producing `inf` (which cannot be written back) or falling through to `unbound name`.")
  (input
    10000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000.0)
  (error CDZ0201))

(case "the boolean literals" (input true) (output (: true Bool)))

; The cross-language habit `True`/`False` reads as an unbound NAME — the lexer only classifies lowercase
; `true`/`false` as `Leaf::Bool`. The lowercase literal is edit-distance 1 (within the typo cutoff), so
; CDZ0101 carries a REPLACE fix to the lowercase literal (which re-lexes as the boolean). An all-caps
; `TRUE` is too far (distance 4) → no baseless suggestion. (migrated from rcdzc
; a_miscased_boolean_literal_suggests_the_lowercase_literal.)
(case
  "a miscased boolean True suggests the lowercase true literal with a replace fix"
  (input True)
  (error CDZ0101 (fix (kind replace) (replacement "true"))))

(case
  "a miscased boolean False suggests the lowercase false literal with a replace fix"
  (input (if False true false))
  (error CDZ0101 (fix (kind replace) (replacement "false"))))

(case
  "the unit value"
  (doc
    "Witnesses core-semantics.md #An Effect-Only Expression Yields The Unit Value: `unit` denotes
           the unit value, the normal-termination value of a program that produces nothing else
           (\"A program that terminates normally without producing a value other than through its
           emitted events MUST produce the unit value as its normal-termination value\"). It is a
           first-class result that must cross the run boundary — the effect-only programs in
           03-equality/04-capabilities all terminate in it. This bare-`unit` case pins that a program
           whose result IS unit runs and yields unit, independent of any capability.")
  (input unit)
  (output (: unit Unit)))

(case
  "unit and the empty tuple are the same value"
  (doc
    "Witnesses core-semantics.md #The Empty Tuple Is The Unit Value (\"unit and () are the same
           value\"): the empty tuple `()` denotes exactly the unit value, so `(= unit ())` is true and
           each yields the unit value as a program result.")
  (input (= unit ()))
  (output (: true Bool)))

(case
  "the unit value observes its total order — equal to itself, never less"
  (doc
    "Unit is a total-order type with exactly one value, so all four ordering operators observe the
           trivial order: `(< unit ())` is FALSE (Equal, not Less), while `(<= unit ())` and `(>= () unit)`
           are TRUE (reflexive). Folded at compile time — unit carries no data and has no machine slot.
           Weighted so one result pins all three: (if (< unit ()) 100 0) + (if (<= unit ()) 1 0) +
           (if (>= () unit) 10 0) = 0 + 1 + 10 = 11. Relocated from rcdzc two_unit_values_compare_equal
           (its (= unit ()) equality is the case above; its (= unit 5) cross-type reject is the case below).")
  (input
    (do
      (def (main) (+ (if (< unit ()) 100 0) (+ (if (<= unit ()) 1 0) (if (>= () unit) 10 0))))
      (export main)))
  (call main)
  (output (: 11 Int64)))

(case
  "comparing the unit value against a non-unit is a type error"
  (doc
    "The comparison operator is `∀a. a -> a -> Bool`, so both operands must be the SAME type. `(= unit
           5)` cannot unify Unit with Int64, so it rejects CDZ0203 — a unit compared against a non-unit is a
           type error, not a runtime `false`. (migrated from rcdzc comparing_unit_against_a_non_unit; the
           unit-vs-unit equality and total order are the two cases above.)")
  (input (do (def (main) (= unit 5)) (export main)))
  (error CDZ0203))

(case "a string literal" (input "hello") (output (: "hello" String)))

(case
  "a string literal is normalized to the canonical text form"
  (doc
    "Stored in the pinned text normalization form (options/hashing-and-encoding/),
           so two literals differing only in normalization are one value.")
  (input "café")
  (output (: "café" String)))

; --- A string literal's escape sequences are a closed set -----------------------------------
; collections-and-text.md #A String Literal's Escapes Are A Closed Set: within a string literal a
; backslash introduces an escape sequence, and a conforming reader recognizes exactly `\n` (U+000A),
; `\t` (U+0009), `\r` (U+000D), `\\` (U+005C), and `\"` (U+0022). A backslash before any other
; character is a compile-time error, so an unrecognized escape is a rejected program rather than a
; silently-dropped backslash or an implementation-defined character. These pin the recognized set (the
; escape denotes the one scalar value it names, so `(= "\t" <a literal tab>)` is true) and the
; rejection of an unrecognized escape.
(case
  "a recognized string escape denotes its one scalar value"
  (doc
    "`\"\\t\"` is the one-scalar string containing a tab (U+0009): the reader expands the escape,
           so it equals a literal that contains an actual tab character (collections-and-text.md #A
           String Literal's Escapes Are A Closed Set). Witnesses that `\\t` is recognized and denotes
           exactly U+0009, not the two characters backslash-t.")
  (input (= "\t" "\t"))
  (output (: true Bool)))

(case
  "an unrecognized string escape is rejected"
  (doc
    "`\"\\q\"` uses a backslash before `q`, which begins none of the recognized escape sequences,
           so the reader MUST reject it at compile time (collections-and-text.md #A String Literal's
           Escapes Are A Closed Set) rather than drop the backslash and read `q` (length 1) or emit an
           implementation-defined character. The seed's reader today
           accepts an unknown escape as the bare character, so it DECLINES this case until a generation
           enforces the closed set; a later generation rejects the program.")
  (input "\q")
  (error CDZ0001))

; --- Boundary values crossing the PARAMETER boundary --------------------------------------------------
; The literal cases above read boundary values as CONSTANTS (Int64.min/max, UInt64.max, -0.0, 1e19) — each
; folds at compile time. A value that arrives as a runtime PARAMETER instead crosses the component
; boundary and is decoded from its boundary representation (contracts/component-abi.md; deterministic-value-
; form.md). The extreme values are where a naive ABI slips: `Int64.min` has only the sign bit set,
; `UInt64.max` fills all 64 bits (a signed-i64 read would see -1), `-0.0` differs from `0.0` only in the
; sign bit, and a large whole float exceeds the Int64 range. These pin that an identity export returns each
; boundary value UNCHANGED across the boundary — the runtime-parameter companion of the constant cases.
(case
  "Int64.min crosses the parameter boundary and back unchanged"
  (doc
    "`(def (main (: x Int64)) x)` called with Int64.min returns it unchanged: -9223372036854775808 —
           the only-the-sign-bit value — decodes from and re-encodes to its boundary form exactly. The
           runtime companion of the constant `the minimum Int64 literal` case; a sign-mishandling boundary
           decode would corrupt it.")
  (input (do (def (main (: x Int64)) x) (export main)))
  (call main (: -9223372036854775808 Int64))
  (output (: -9223372036854775808 Int64)))

(case
  "Int64.min crossing the boundary is still negative"
  (doc
    "The value-check companion: `(< x 0)` with `x` = Int64.min arriving as a parameter is true — the
           boundary decode preserves its negativity, not an unsigned reinterpretation. Pins that the sign of
           the extreme minimum survives the crossing.")
  (input (do (def (main (: x Int64)) (< x 0)) (export main)))
  (call main (: -9223372036854775808 Int64))
  (output (: true Bool)))

(case
  "UInt64.max crosses the parameter boundary and back unchanged"
  (doc
    "`(def (main (: x UInt64)) x)` called with UInt64.max returns 18446744073709551615 = 2^64-1 —
           all 64 bits set — unchanged. A boundary that read the cell as a SIGNED i64 would see -1 and
           render it wrong; this pins the full unsigned width round-trips. The runtime companion of the
           constant `UInt64.max` case.")
  (input (do (def (main (: x UInt64)) x) (export main)))
  (call main (: 18446744073709551615 UInt64))
  (output (: 18446744073709551615 UInt64)))

(case
  "UInt64.max crossing the boundary compares as a large unsigned, not -1"
  (doc
    "The value-check companion: `(> x 0)` with `x` = UInt64.max arriving as a parameter is true —
           the all-bits-set value is a large POSITIVE unsigned, not the -1 a signed read would make it (for
           which `> 0` is false). Pins that the boundary decode keeps UInt64 unsigned.")
  (input (do (def (main (: x UInt64)) (> x 0)) (export main)))
  (call main (: 18446744073709551615 UInt64))
  (output (: true Bool)))

(case
  "negative zero crosses the parameter boundary preserving its sign"
  (doc
    "`(def (main (: x Float64)) x)` called with -0.0 returns -0.0, NOT 0.0 — the canonical value form
           distinguishes them by the sign bit (deterministic-value-form.md; the constant `negative zero is
           distinct` case). A boundary decode that normalized -0.0 to 0.0 would render `0.0` here. Pins that
           negative zero survives the parameter crossing as a distinct value.")
  (input (do (def (main (: x Float64)) x) (export main)))
  (call main (: -0.0 Float64))
  (output (: -0.0 Float64)))

(case
  "a large whole float crosses the boundary preserving its full value"
  (doc
    "`(def (main (: x Float64)) x)` called with 1e19 — a whole-valued Float64 just past the Int64
           range (2^63 ≈ 9.22e18) — returns its full value 10000000000000000000.0, not an Int64-saturated
           approximation. The runtime companion of the constant `a large whole-valued float renders its full
           value` case: the boundary carries the exact binary64, not a value routed through an integer cast.")
  (input (do (def (main (: x Float64)) x) (export main)))
  (call main (: 10000000000000000000.0 Float64))
  (output (: 10000000000000000000.0 Float64)))

(case
  "a large-magnitude float whose shortest form differs from its exact value renders the full decimal expansion"
  (doc
    "`(def (main (: x Float64)) x)` called with 3.4028235e38 — a large-magnitude Float64 near the
           f32 max, well inside binary64's ~1.8e308 finite ceiling, chosen because its shortest
           round-tripping decimal (`3402823500...0.0`) DIFFERS from its exact binary64 value — returns
           340282349999999991754788743781432688640.0, the FULL decimal expansion of the exact binary64
           the source `3.4028235e38` denotes. This is the exact value form, NOT the shortest re-reading
           decimal: both re-parse to the same double, and the value renderer emits the exact expansion
           (the large-magnitude companion of the `1e19` full-value case above), whereas the
           shortest-round-tripping rule governs the `Ast.Float` print/read metaprogramming path. Both
           backends agree — pins the exact-expansion value render against a renderer regression.")
  (input (do (def (main (: x Float64)) x) (export main)))
  (call main (: 340282350000000000000000000000000000000.0 Float64))
  (output (: 340282349999999991754788743781432688640.0 Float64)))

(case
  "a NEGATIVE large-magnitude float renders its full expansion with its sign"
  (doc
    "The sign companion of the large-magnitude render pin: -3.4028235e38 renders
           -340282349999999991754788743781432688640.0 — the full decimal expansion carrying its leading minus, NOT the shortest
           re-reading form. Pins that the exact-expansion value render preserves the sign on a negative
           top-of-magnitude Float64 (the sign is orthogonal to the compound-vs-scalar axis pinned by the
           tuple/list/Option cases below). Both backends agree.")
  (input (do (def (main (: x Float64)) x) (export main)))
  (call main (: -340282350000000000000000000000000000000.0 Float64))
  (output (: -340282349999999991754788743781432688640.0 Float64)))

(case
  "a large-magnitude float renders its full expansion as a TUPLE element (compound matches scalar)"
  (doc
    "Compound-element companion of the scalar large-magnitude pin: a Float64 whose shortest form
           differs from its exact value, as a tuple element, renders the FULL decimal expansion — the same
           form the scalar path and rust emit — guarding the wasm KIND_FLOAT (float_leaf) renderer against
           diverging to the shortest form. Converged by v-runtime (all three KIND_FLOAT encode paths emit the
           full expansion for whole floats).")
  (input (do (def (main) #tuple(340282350000000000000000000000000000000.0 1.0)) (export main)))
  (output (: #tuple(340282349999999991754788743781432688640.0 1.0) (Tuple Float64 Float64))))

(case
  "a large-magnitude float renders its full expansion as a LIST element"
  (doc
    "The list-element face of the compound float render — same KIND_FLOAT heap path as the tuple case;
           a large-magnitude Float64 in a list literal renders its full exact expansion, matching rust.")
  (input (do (def (main) #list(340282350000000000000000000000000000000.0)) (export main)))
  (output (: #list(340282349999999991754788743781432688640.0) (List Float64))))

(case
  "a large-magnitude float renders its full expansion as an OPTION (sum) payload"
  (doc
    "The sum-payload face of the compound float render — a boxed KIND_FLOAT inside Option.Some renders
           its full exact expansion, matching scalar + rust.")
  (input (do (def (main) (Some 340282350000000000000000000000000000000.0)) (export main)))
  (output (: (Some 340282349999999991754788743781432688640.0) (Option Float64))))

(case
  "a float at f64::MAX renders its full 309-digit expansion (large-significand codec)"
  (doc
    "Guards the large-significand KIND_FLOAT doc-codec path (a 309-digit / 128-byte significand) that
           the shortest form never exercised (shortest sig <= 17 digits) — the latent large-significand
           readback the KIND_FLOAT codec hardened (LEB significand length + arbitrary-length limb magnitude)
           alongside the full-expansion convergence. f64::MAX = 1.7976931348623157e308.")
  (input
    (do
      (def
        (main)
        179769313486231570000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000.0)
      (export main)))
  (output
    (:
      179769313486231570814527423731704356798070567525844996598917476803157260780028538760589558632766878171540458953514382464234321326889464182768467546703537516986049910576551282076245490090389328944075868508455133942304583236903222948165808559332123348274797826204144723168738177180919299881250404026184124858368.0
      Float64)))

(case
  "a compound-element float renders the same full expansion as the scalar path"
  (doc
    "The COMPOUND-element companion of the full-expansion case above: the same 3.4028235e38
           returned as a TUPLE element must render the identical exact expansion — the compound
           value-encode walk and the scalar path share one float renderer. Before the KIND_FLOAT
           convergence the compound paths emitted the SHORTEST round-tripping form instead, so a
           Float64 rendered DIFFERENTLY as a bare scalar vs a tuple/list element (the float_leaf
           divergence); this pins the converged behavior — one canonical expansion regardless of the
           position the float crosses the boundary in. Both backends.")
  (input (do (def (main (: x Float64)) #tuple(x 1)) (export main)))
  (call main (: 340282350000000000000000000000000000000.0 Float64))
  (output (: #tuple(340282349999999991754788743781432688640.0 1) (Tuple Float64 Int64)))
  (live-objects known-leak))

; The hex sibling of the bare-overflow malformed literal: `0xFFFFFFFFFFFFFFFF` is a VALID hex radix (unlike
; the bad-radix cases) but its value (2^64-1) overflows the signed Int64 a bare literal defaults to, so it is
; a malformed literal (CDZ0201) — a bare literal is signed even when the bits would fit unsigned-64. And the
; bare-overflow check must NOT misfire on an EXPLICIT UInt64 annotation: `(: 18446744073709551615 (UInt 64))`
; = UInt64.max is in range and runs. (Migrated from rcdzc a_bare_literal_past_int64_is_malformed_not_out_of_range;
; the decimal bare-overflow / Int64.max-fits faces are the existing 01 cases above, the explicit-width CDZ0302
; control is 06-numeric-model's over-width cases.)
(case
  "a hex bare literal that overflows signed Int64 is a malformed literal"
  (input 0xffffffffffffffff)
  (error CDZ0201))

(case
  "an explicit UInt64-max annotation is in range and runs (the bare-overflow check does not misfire)"
  (input (: 18446744073709551615 (UInt 64)))
  (output (: 18446744073709551615 UInt64)))

; A BARE RATIONAL LITERAL `n/d` compiles + escapes — the literal twin of `(Rational.of n d)`. The reader emits
; both `3/2` and the `#rational(3 2)` alias to the SAME `(RationalTag n d)` node; before this, that node had no
; compiler fold (resolve fell to its `Leaf::Rational` head → the bare-head CDZ0201), so a bare rational literal
; was UNCOMPILABLE on every surface incl. the binary-AST path — never caught because every corpus rational used
; `(Rational.of n d)` in source and bare `n/d` only ever appeared in OUTPUT value position (v-ast-compound report).
(case
  "a bare rational literal compiles and escapes as its normalized value"
  (input 3/2)
  (output (: 3/2 Rational)))

(case
  "a bare rational literal normalizes to lowest terms (gcd-reduce), like Rational.of"
  (input 6/4)
  (output (: 3/2 Rational)))

(case
  "the #rational(n d) alias reads to the same rational literal and compiles"
  (input 6/4)
  (output (: 3/2 Rational)))

(case
  "a bare rational literal and Rational.of build one identical normalized value"
  (input (= 6/4 (Rational.of 6 4)))
  (output (: true Bool)))
