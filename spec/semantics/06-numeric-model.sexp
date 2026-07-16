; Numeric model — witnesses numeric-model.md. The primary clause is the recorded oracle: a well-typed
; program's terminal value or trap, or — for an ill-typed program — its (error <CODE>) rejection, because
; an ill-typed program has no run and therefore no terminal value. For a type rule a generation does not
; yet cover it DECLINES rather than running the program (reject-don't-miscompile); the gate scores a
; decline as todo, not disagreement. A generation that has not yet realized the extended numerics
; declines those cases (the seed realizes only the checked Int64 core and Float64 literals/equality —
; options/realized-capability-set/). Diagnostic codes are from options/diagnostics-schema/.

(case "arithmetic within one integer type"
  (input  (+ 2 3))
  (output (: 5 Int64)))

(case "mixing two numeric types without an explicit conversion does not silently promote"
  (doc    "Witnesses numeric-model.md #Numeric Types Do Not Silently Promote: `(+ 2 2.0)` adds an Int64
           and a Float64, which do not silently unify, so the compiler rejects it (CDZ0301) rather than
           coercing either way — or declines if it does not yet cover the no-promotion rule
           (reject-don't-miscompile). The rejection is the program's outcome; there is no value.")
  (input  (+ 2 2.0))
  (error  CDZ0301))

; --- No silent promotion holds for EVERY Int64-operand operator, not only `+` --------------
; numeric-model.md #Numeric Types Do Not Silently Promote applies to any operation on two numeric
; values of different types — "The type of an arithmetic result MUST be determined by the operand
; types and the operation, not by an implicit widening the author did not write." The `+` case above
; is the only witness; but `-` `*` `/` `%` `&` `|` `<<` `>>` all take Int64 operands and each is a
; natural place a reimplementation could coerce a Float64 operand to f64 (or truncate it to i64) and
; proceed. Each MUST reject an Int64/Float64 mix (CDZ0301) exactly as `+` does — the bitwise and shift
; operators especially, whose operands are bit patterns with no float meaning at all. A generation
; that does not yet cover the rule for a given operator declines rather than coercing
; (reject-don't-miscompile); the gate scores a decline as todo, not disagreement.

(case "subtraction of an integer and a float does not silently promote"
  (doc    "`(- 5 2.0)` mixes Int64 and Float64, rejected (CDZ0301) exactly as `(+ 2 2.0)` is. Pins the
           no-promotion rule for `-`.")
  (input  (- 5 2.0))
  (error  CDZ0301))

(case "multiplication of an integer and a float does not silently promote"
  (doc    "`(* 5 2.0)` mixes two numeric types, rejected (CDZ0301). Pins no-promotion for `*`.")
  (input  (* 5 2.0))
  (error  CDZ0301))

(case "division of an integer and a float does not silently promote"
  (doc    "`(/ 5 2.0)` mixes Int64 and Float64, rejected (CDZ0301) rather than performing a float
           division of a promoted 5.0 — the author did not write the conversion. Pins no-promotion for
           `/`.")
  (input  (/ 5 2.0))
  (error  CDZ0301))

(case "modulo of an integer and a float does not silently promote"
  (doc    "`(% 5 2.0)` mixes two numeric types, rejected (CDZ0301). Pins no-promotion for `%`, which
           has no defined meaning across a mixed Int64/Float64 pair.")
  (input  (% 5 2.0))
  (error  CDZ0301))

(case "bitwise AND of an integer and a float does not silently promote"
  (doc    "`(& 1 2.0)` applies a bitwise operator to an Int64 and a Float64 — a Float64 is not a bit
           pattern to mask, so the mix is rejected (CDZ0301), not coerced. Pins no-promotion for `&`,
           where a silent conversion is especially wrong (a float has no meaningful low bits to AND).")
  (input  (& 1 2.0))
  (error  CDZ0301))

(case "bitwise OR of an integer and a float does not silently promote"
  (doc    "`(| 1 2.0)` mixes Int64 and Float64 in a bitwise OR, rejected (CDZ0301). Pins no-promotion
           for `|`, the companion of the bitwise AND case.")
  (input  (| 1 2.0))
  (error  CDZ0301))

(case "bitwise XOR of an integer and a float does not silently promote"
  (doc    "`(^ 1 2.0)` applies bitwise XOR to an Int64 and a Float64, rejected (CDZ0301). Pins
           no-promotion for `^`, the third bitwise operator alongside `&` and `|`.")
  (input  (^ 1 2.0))
  (error  CDZ0301))

(case "a left shift by a floating-point count does not silently promote"
  (doc    "`(<< 1 2.0)` supplies a Float64 shift count where a shift count is an Int64 bit position —
           a numeric-type mismatch rejected (CDZ0301), not a coerced `<< 2`. Pins no-promotion for the
           shift count of `<<`.")
  (input  (<< 1 2.0))
  (error  CDZ0301))

(case "a right shift by a floating-point count does not silently promote"
  (doc    "`(>> 1 2.0)` mixes an Int64 value with a Float64 shift count, rejected (CDZ0301). Pins
           no-promotion for `>>`, completing the eight Int64-operand operators against the rule the
           `(+ 2 2.0)` case introduces.")
  (input  (>> 1 2.0))
  (error  CDZ0301))

(case "overflow of the default integer traps deterministically"
  (doc    "Witnesses numeric-model.md #Overflow Is Defined: the compiler REJECTS operations
           it can PROVE will overflow (via constant folding or β-reduction), failing the build
           with CDZ0304 rather than deferring to a runtime trap. This is the static safety
           guarantee — catch errors as early as possible.")
  (input  (+ Int64.max 1))
  (error  CDZ0304))

(case "an explicit conversion makes the operation well-typed"
  (doc    "Witnesses numeric-model.md #Exact Arithmetic Is Exact (2nd sentence): the conversion
           Float64.of-int is written explicitly, then the two Float64 operands are added with the ONE
           arithmetic operator `+` (numeric-model.md #An Arithmetic Operator Requires Both Operands To Be
           One Numeric Type) — `+` over two Float64 operands is float addition, dispatched on the operand
           type. The unconverted `(+ 1 2.0)` would reject (an Int64/Float64 mix), which is why the `1` is
           converted first.")
  (input  (+ (Float64.of-int 1) 2.0))
  (output (: 3.0 Float64)))

(case "wrapping arithmetic uses the named wrapping form of the operator"
  (doc    "Witnesses numeric-model.md #A Wrapping Operation Has A Defined Modular Outcome: the wrapping
           overflow behavior is a NAMED FORM of the operator on the integer type, opted into at the call
           (`Int64.wrapping-add`), NOT a distinct wrapping type. `(Int64.wrapping-add Int64.max 1)` wraps
           to Int64.min in two's complement rather than trapping — the defined modular outcome the model
           admits alongside the trapping default `+`. (See the dedicated checked/wrapping section below
           for the full add/mul coverage.)")
  (input  (Int64.wrapping-add Int64.max 1))
  (output (: -9223372036854775808 Int64)))

; --- Exact rationals: a normalized pair of big-integers, opted into explicitly ------------
; The exact rational type `Rational` (options/numeric-model/) is a numerator/denominator pair kept in
; CANONICAL NORMALIZED FORM — lowest terms, sign on the numerator, denominator strictly positive — so
; two rationals denoting the same number are one value with one canonical byte form (numeric-model.md
; #An Exact Rational Has A Canonical Normalized Form; deterministic-value-form.md #A Value Has One
; Canonical Byte Form). Arithmetic is exact and closed: `+ - * /` on two Rationals yield a normalized
; Rational with no rounding, and rational `/` by a nonzero rational is total and exact — unlike integer
; `/` (truncates) and float `/` (rounds). A zero denominator denotes no number, so it traps. `Rational`
; is a DISTINCT numeric type opted into explicitly; no operation silently produces or consumes one (the
; old-Cadenza behavior where integer `/` yielded a rational is exactly the silent promotion this model
; rejects). The written value form is `n/d` in lowest terms. A generation realizing only the checked
; Int64 core and Float64 literals/equality declines this until the extended numerics land.

(case "exact rational arithmetic is exact and normalized"
  (doc    "Witnesses numeric-model.md #Exact Arithmetic Is Exact; reduced to lowest terms per
           options/numeric-model/. `(+ (Rational.of 1 3) (Rational.of 1 6))` = 1/3 + 1/6 = 1/2 exactly,
           the canonical rational value form (no float rounding — a Float64 sum would not be exact).")
  (input  (+ (Rational.of 1 3) (Rational.of 1 6)))
  (output (: 1/2 Rational)))

(case "a rational is normalized to lowest terms on construction"
  (doc    "`(Rational.of 2 4)` reduces to 1/2 — a Rational is kept in lowest terms (numerator and
           denominator share no common factor), so 2/4 and 1/2 are ONE value with one canonical byte
           form (numeric-model.md #An Exact Rational Has A Canonical Normalized Form). Normalization is a
           function of the number, not of how it was written.")
  (input  (Rational.of 2 4))
  (output (: 1/2 Rational)))

(case "an integer literal annotated Rational grounds to that integer over one"
  (doc    "`(: 5 Rational)` grounds the bare integer literal 5 to the exact rational 5/1 — the same
           "Annotations Constrain" rule that fixes `(: 200 UInt8)`, extended to Rational with no range
           check (an exact rational holds any integer). The annotation is what SELECTS Rational; the
           literal keeps its exact value. Being explicit (an annotation), not an implicit widening.")
  (input  (: 5 Rational))
  (output (: 5/1 Rational)))

(case "a decimal literal annotated Rational grounds to its exact fraction"
  (doc    "`(: 0.5 Rational)` grounds the decimal literal to the EXACT rational 1/2 — a decimal literal is
           captured exactly as significand*10^exp (0.5 = 5*10^-1), so it converts to 5/10 = 1/2 with NO
           float rounding. This is why the annotation, not `/`, is the rational spelling: `(/ 1 2)` is
           integer division (= 0), while `(: 0.5 Rational)` is the exact one-half.")
  (input  (: 0.5 Rational))
  (output (: 1/2 Rational)))

(case "a decimal literal annotated Rational is exact for a value no float rounds cleanly"
  (doc    "`(: 0.1 Rational)` is EXACTLY 1/10 — the classic value a binary float cannot represent. The
           exact-Decimal capture (1*10^-1) makes the rational grounding lossless where a Float64 would
           round, witnessing that the annotation grounds from the exact literal, not an f64.")
  (input  (: 0.1 Rational))
  (output (: 1/10 Rational)))

(case "a negative decimal literal annotated Rational normalizes its sign onto the numerator"
  (doc    "`(: -0.75 Rational)` grounds to -3/4: 75*10^-2 = 75/100, reduced to 3/4 with the sign on the
           numerator (the canonical normalized form — the denominator is strictly positive).")
  (input  (: -0.75 Rational))
  (output (: -3/4 Rational)))

(case "a scientific-notation literal annotated Rational scales the numerator"
  (doc    "`(: 12e2 Rational)` grounds to 1200/1: a non-negative decimal exponent multiplies the
           significand (12*10^2 = 1200), the whole being an exact integer-valued rational.")
  (input  (: 12e2 Rational))
  (output (: 1200/1 Rational)))

(case "a Rational-annotated literal composes with exact rational arithmetic"
  (doc    "The grounding is a real Rational value, so it flows into the exact `+ - * /` arithmetic: `(+ (:
           0.5 Rational) (Rational.of 1 3))` = 1/2 + 1/3 = 5/6 exactly. The annotated literal and the
           explicit constructor produce the same kind of value; math "just works" over both.")
  (input  (+ (: 0.5 Rational) (Rational.of 1 3)))
  (output (: 5/6 Rational)))

(case "an N-suffixed integer literal is a BigInt"
  (doc    "The `N` type suffix (`100N`) selects `BigInt` per-literal — the Rust-style opt-in that reads
           as a terse `(: 100 BigInt)` annotation. Being EXPLICIT: a bare `100` stays a fixed-width
           default; you write `N` to ask for the unbounded integer. `100N` is exactly 100 as a BigInt.")
  (input  100N)
  (output (: 100 BigInt)))

(case "an N suffix lets a huge literal need no annotation"
  (doc    "`100000000000000000000N` — a value no fixed width holds — is a BigInt via the suffix alone,
           no `(: … BigInt)` wrapper. The suffix is the concise spelling of the annotation grounding.")
  (input  100000000000000000000N)
  (output (: 100000000000000000000 BigInt)))

(case "N-suffixed literals compose under BigInt arithmetic"
  (doc    "`(+ 100N 1N)` = 101 as a BigInt: each suffixed literal is a real BigInt value, so the exact
           `+` runs over them — the math 'just works' over the suffixed spelling exactly as over
           `(BigInt.of …)`.")
  (input  (+ 100N 1N))
  (output (: 101 BigInt)))

(case "a RADIX literal carries the N type suffix"
  (doc    "The `N` (BigInt) / `R` (Rational) type suffix applies to a RADIX body (`0x…`/`0b…`) exactly as
           to a decimal one — `0xFFN` is the BigInt 255, `0b1010N` the BigInt 10 — matching the documented
           `f91a9001` example (`100N`, `0xFFN`, `1_000N`). The suffix peel reads the radix body then the
           glued suffix letter, so a hex/binary literal opts into BigInt with the same terse spelling as a
           decimal one. (This was an ML-surface lexer gap: `0xFFN` there mis-lexed as a quantity `(Qty.of
           0xFF (Unit.of \"N\"))` → CDZ0201 'unknown unit N', while the s-expr reader — and now the ML
           lexer too — reads it as the suffixed BigInt. These s-expr cases guard the VALUE on both surfaces.)")
  (input  0xFFN)
  (output (: 255 BigInt)))

(case "a binary-radix literal carries the N suffix and an underscore group"
  (doc    "`0b1010N` is the BigInt 10; a hex literal with an underscore group + suffix `0xFF_FFN` is the
           BigInt 65535 — the radix suffix peel composes with `_` digit separators, as the decimal path
           does. Pins that the whole `<radix-body-with-underscores><suffix>` is one suffixed literal.")
  (input  (+ 0b1010N 0xFF_FFN))
  (output (: 65545 BigInt)))

(case "a radix literal carries the R (Rational) suffix"
  (doc    "The `R` suffix over a radix body: `0xFFR` is the Rational 255/1 (an integer body grounds to
           `n/1`), the radix twin of `5R`. Confirms both suffix kinds reach the radix path.")
  (input  0xFFR)
  (output (: 255/1 Rational)))

(case "an R-suffixed integer literal is that integer over one"
  (doc    "The `R` type suffix (`5R`) selects `Rational`: an integer body grounds to `5/1`, the terse
           form of `(: 5 Rational)`.")
  (input  5R)
  (output (: 5/1 Rational)))

(case "an R-suffixed decimal literal is its exact fraction"
  (doc    "`0.5R` is EXACTLY 1/2 — the `R` suffix over a decimal body grounds to the exact rational
           (the decimal is captured exactly, so no float rounding). `0.5R` reads as `(: 0.5 Rational)`.")
  (input  0.5R)
  (output (: 1/2 Rational)))

; --- A suffixed literal carries its OWN type: annotating it narrower is a MISMATCH, not an overflow ---
; A suffix gives a literal a concrete type (`N`→BigInt, `R`→Rational) — so annotating `100N` as `Int64`
; is a genuine type MISMATCH (BigInt ≠ Int64), a single CDZ0203, NOT a bare literal being grounded that
; overflows a width. This distinguishes two paths that share a surface: a SUFFIXED literal already has a
; type (mismatch → CDZ0203), whereas a BARE literal types as the `Int64` default and, if it exceeds the
; width, overflows its grounding (→ CDZ0302 "does not fit"). The suffixed case reports ONE fault (the
; mismatch), not the mismatch PLUS a redundant "does not fit Int64" (the width check must not see through
; the suffix's desugar to range-check the inner integer — a BigInt is the wrong TYPE, not an out-of-range
; Int64). The bare-overflow case below reports the OTHER, single fault.

(case "a suffixed BigInt literal annotated as Int64 is a type mismatch"
  (doc    "`(: 100N Int64)` annotates a BigInt literal (`100N` carries type BigInt via the suffix) as
           `Int64` — a genuine type mismatch (BigInt ≠ Int64), reported as a single CDZ0203. NOT a bare
           `100` grounding that overflows Int64 (it fits), and NOT double-reported with a redundant CDZ0302
           'does not fit Int64': a suffixed literal has its own type, so the annotation is a mismatch, not a
           width overflow. Pins the single-fault mismatch for the BigInt suffix.")
  (input  (: 100N Int64))
  (error  CDZ0203))

(case "a suffixed Rational literal annotated as Int64 is a type mismatch"
  (doc    "`(: 1R Int64)` annotates a Rational literal (`1R` = 1/1, type Rational) as `Int64` — Rational ≠
           Int64, a single CDZ0203. The Rational twin of the BigInt suffix case: an explicitly-suffixed
           literal's type does not silently coerce to the annotation's narrower type.")
  (input  (: 1R Int64))
  (error  CDZ0203))

(case "a bare literal that overflows the annotated width does not fit it"
  (doc    "The contrasting BARE case: `(: 100000000000000000000 Int64)` — an UNSUFFIXED literal (10^20)
           annotated `Int64`. A bare literal types as the Int64 default and is grounded by the annotation,
           so a value past the width is an OVERFLOW of that grounding — CDZ0302 'does not fit the annotated
           type Int64', with the BigInt-widen fix. This is the DIFFERENT path from the suffixed cases above
           (which are type mismatches, CDZ0203): a bare literal has no type of its own to clash, only a
           range to exceed. Pins that the width fit-check still fires for a bare literal.")
  (input  (: 100000000000000000000 Int64))
  (error  CDZ0302))

; The width fit-check must ALSO reach a literal NESTED in a COMPOUND payload. `(: (Some 999) (Option Int8))`
; propagates the annotation's `Int8` into the `Some` payload, so `999` is grounded at `Int8` — and 999
; OVERFLOWS Int8 (max 127), a CDZ0302 exactly as the bare `(: 999 Int8)` is. The literal's OWN type stays a
; deferred `Int64` (only the enclosing value's type carries the `Int8`), so the top-level fit-check does not
; see it; the check descends the annotation's expected type against the value's payload to range-check the
; nested literal. Pins that the range check is not fooled by a compound wrapper — the same "check descends
; into the compound payload" the type-agreement check already does, now for the width fit-check (it closes a
; check-vs-emit gap: the emit path already rejected this, `cdz check` used to accept it).
(case "a literal nested in a compound payload that overflows the annotated width is rejected"
  (doc    "`(: (Some 999) (Option Int8))` — the annotation's `Int8` grounds the `Some` payload literal
           `999`, which overflows Int8 (valid range -128..=127) → CDZ0302, exactly as the bare `(: 999
           Int8)`. The nested literal's own type is a deferred Int64, so the width fit-check must DESCEND the
           annotation into the payload to catch it (the range-check analogue of the annotation-descends-into-
           compound-payload type check). Pins that a compound wrapper does not hide an out-of-range payload
           literal from the width check.")
  (input  (: (Some 999) (Option Int8)))
  (error  CDZ0302))

(case "an R-suffixed literal composes with exact rational arithmetic"
  (doc    "`(+ 0.5R (Rational.of 1 3))` = 1/2 + 1/3 = 5/6 exactly — the suffixed literal flows into the
           exact `+` just like the explicit constructor, so both spellings denote one kind of value.")
  (input  (+ 0.5R (Rational.of 1 3)))
  (output (: 5/6 Rational)))

(case "a decimal literal annotated Rational equals its explicit constructor form"
  (doc    "`(= (: 1.25 Rational) (Rational.of 5 4))` is true — the decimal grounding 1.25 -> 5/4 is the
           SAME normalized value the constructor builds, so the two spellings denote one rational.")
  (input  (= (: 1.25 Rational) (Rational.of 5 4)))
  (output (: true Bool)))

(case "two rationals denoting the same number are equal regardless of how they were written"
  (doc    "`(= (Rational.of 2 4) (Rational.of 1 2))` is true: because both normalize to 1/2, rational
           equality is structural over the normalized pair (deterministic-value-form.md #A Value Has One
           Canonical Byte Form). Pins that equality compares canonical forms, not the raw numerator/
           denominator a program supplied.")
  (input  (= (Rational.of 2 4) (Rational.of 1 2)))
  (output (: true Bool)))

(case "a rational's sign is normalized onto the numerator"
  (doc    "`(Rational.of 1 -2)` normalizes to -1/2 — the fixed sign convention puts the sign on the
           numerator and keeps the denominator strictly positive (numeric-model.md #An Exact Rational
           Has A Canonical Normalized Form). So `(Rational.of 1 -2)`, `(Rational.of -1 2)`, and
           `(Rational.of -1 -2)`'s companions all resolve to one signed canonical form; here the result
           is negative.")
  (input  (Rational.of 1 -2))
  (output (: -1/2 Rational)))

(case "a rational with numerator and denominator both negative normalizes to positive"
  (doc    "`(Rational.of -1 -2)` = 1/2: the two negatives cancel under the sign convention (denominator
           forced strictly positive), so a both-negative pair is a positive rational. Companion of the
           sign-on-numerator case, pinning that sign normalization is by the number's sign, not by which
           component carried the minus.")
  (input  (Rational.of -1 -2))
  (output (: 1/2 Rational)))

(case "exact rational division is total and exact for a nonzero divisor"
  (doc    "`(/ (Rational.of 1 2) (Rational.of 3 4))` = (1/2)/(3/4) = 4/6 = 2/3 exactly. Rational `/` by
           a NONZERO rational is total and exact — it neither truncates (as integer `/` does) nor rounds
           (as float `/` does) — and the result is normalized to lowest terms. This is the exactness the
           type is opted into for.")
  (input  (/ (Rational.of 1 2) (Rational.of 3 4)))
  (output (: 2/3 Rational)))

(case "a whole rational carries a denominator of one"
  (doc    "`(Rational.of-int 5)` is the whole rational 5/1 : Rational — a DISTINCT type from `5 : Int64`.
           Crossing between the integer and the rational is explicit (`Rational.of-int` in), never an
           implicit promotion, the same no-promotion discipline the integer widths obey. Its canonical
           written form is 5/1.")
  (input  (Rational.of-int 5))
  (output (: 5/1 Rational)))

(case "constructing a rational with a zero denominator traps"
  (doc    "`(Rational.of 1 0)` denotes no number — a zero denominator has no rational value
           (numeric-model.md #A Rational With A Zero Denominator Is Not A Value), the rational analogue of
           integer division by zero. The denominator here is the CONSTANT `0`, so — exactly as `(/ 5 0)`
           is rejected CDZ0304 rather than emitting a runtime trap — the compiler PROVES the zero
           denominator via constant folding and rejects at compile time (CDZ0304). Static safety: catch a
           provable error early. (A runtime-computed zero denominator is the defined runtime trap, a later
           increment when a rational is constructed from runtime operands.)")
  (input  (Rational.of 1 0))
  (error  CDZ0304))

(case "a rational operation does not silently promote an integer operand"
  (doc    "`(+ (Rational.of 1 2) 1)` mixes a Rational and an Int64 — two distinct numeric types — so it
           is rejected (CDZ0301) rather than promoting the 1 to 1/1, exactly as an Int64/Float64 mix is
           (numeric-model.md #Numeric Types Do Not Silently Promote). To add the integer, a program
           writes the conversion explicitly: `(+ (Rational.of 1 2) (Rational.of-int 1))`.")
  (input  (+ (Rational.of 1 2) 1))
  (error  CDZ0301))

; --- Arbitrary-precision integers: BigInt, unbounded range, opted into explicitly ---------
; `BigInt` (options/numeric-model/) represents every integer with NO bound — the signed, unbounded
; companion of the fixed-width family. Its arithmetic NEVER traps for magnitude and never wraps: the
; representation grows as the result requires (numeric-model.md #An Arbitrary-Precision Integer Has
; Unbounded Range). This is the type a domain that must not think about overflow reaches for — a
; factorial, a cryptographic modulus, an exact accumulator. `(BigInt.of x)` converts a fixed-width
; integer up; `Int64.of`/`(UInt N).of` convert back CHECKED (trap out of range). It is a DISTINCT type
; opted into explicitly: no operation silently produces or consumes one (a BigInt/Int64 mix is CDZ0301).
; A generation realizing only the checked Int64 core declines this until the extended numerics land.

(case "an arbitrary-precision integer multiplication does not overflow"
  (doc    "`(* (BigInt.of 9223372036854775807) (BigInt.of 9223372036854775807))` multiplies two values
           each equal to Int64.max — a product far beyond 64 bits — and yields the exact BigInt
           85070591730234615847396907784232501249, NOT a trap and NOT a wrap (numeric-model.md #An
           Arbitrary-Precision Integer Has Unbounded Range). The same product over Int64 traps
           (`integer overflow`); BigInt's representation grows instead. THE reason the type exists.
           Computed on the RUNTIME limb library (the compiler does NOT fold BigInt arithmetic — a
           repeated-squaring chain would blow up at compile time), then rendered at the host boundary via
           the `value-encode` walker (`Shape::BigInt`, a variable-length KIND_INT leaf).")
  (input  (* (BigInt.of 9223372036854775807) (BigInt.of 9223372036854775807)))
  (output (: 85070591730234615847396907784232501249 BigInt)))

(case "a beyond-i64 constant BigInt is an operand of a runtime bigint op"
  (doc    "`(: 100000000000000000000 BigInt)` is a constant BigInt = 1e20, BEYOND i64::MAX (~9.2e18). As an
           OPERAND of a runtime bigint op it must materialize a heap leaf: an i64-fitting constant widens via
           `bigint-of-i64`, but a beyond-i64 one has no i64 to feed it — so the compiler bakes its canonical
           sign-magnitude bytes as a Bytes leaf and re-tags them via `bigint-of-bytes`. Compared against
           `(BigInt.of 1e10)²` = 1e20 (computed on the runtime limb library), the `=` is true. Before the
           `bigint-of-bytes` op the beyond-i64 constant operand declined 'not yet materialized as a heap leaf
           (B4)', while an in-i64 constant operand + a beyond-i64 constant as the sole export body both
           worked — the arbitrary-magnitude constant-operand leaf was the remaining B4 gap. THE defining
           BigInt use case: exact arithmetic/comparison against a literal larger than i64 holds.")
  (input  (do
            (def (main)
              (if (= (: 100000000000000000000 BigInt) (* (BigInt.of 10000000000) (BigInt.of 10000000000)))
                  1 0))
            (export main)))
  (output (: 1 Int64)))

(case "a runtime-computed BigInt result crosses the host boundary"
  (doc    "`(+ (BigInt.of 40) (BigInt.of 2))` is a RUNTIME BigInt add (the compiler does not fold BigInt
           arithmetic — it emits `bigint-add`), whose result crosses to the host as its value form `42`
           via the `value-encode` walker (a nullary export returning a BigInt, the runtime-computed
           analogue of the constant-BigInt escape). Pins the small in-range case of the same escape path
           the overflow product exercises — a BigInt result is a variable-length leaf, so it routes
           through the looping `value-encode` (like a runtime collection), not the fixed hole-template.")
  (input  (+ (BigInt.of 40) (BigInt.of 2)))
  (output (: 42 BigInt)))

(case "a negative runtime BigInt result crosses the host boundary with its sign"
  (doc    "`(- (BigInt.of 42) (BigInt.of 100))` = -58 : BigInt — a runtime BigInt SUBTRACT whose result is
           NEGATIVE crosses to the host via the `value-encode` walker with its sign intact (the walker's
           `bigint_leaf` emits the sign-magnitude `KIND_INT` leaf, neg kind for a non-zero negative). Pins
           the sign path of the runtime-BigInt escape, distinct from the positive `(+ 40 2)` companion.")
  (input  (- (BigInt.of 42) (BigInt.of 100)))
  (output (: -58 BigInt)))

(case "a runtime BigInt divide crosses the host boundary as its truncated quotient"
  (doc    "`(/ (BigInt.of 100) (BigInt.of 7))` = 14 : BigInt — a runtime BigInt DIVIDE (truncating toward
           zero, the same semantics as fixed-width `/`) computes on the runtime limb library and crosses
           as its exact quotient. Pins that `/` — not just `+`/`-`/`*` — routes through the runtime op and
           the escape walker (100/7 = 14 remainder 2, truncated to 14).")
  (input  (/ (BigInt.of 100) (BigInt.of 7)))
  (output (: 14 BigInt)))

(case "a runtime BigInt in an Option payload crosses the host boundary"
  (doc    "`(Some (* (BigInt.of 1000000) (BigInt.of 1000000)))` — a runtime BigInt (the 10^12 product does
           not fold) wrapped in an `Option` crosses to the host as `(Some 1000000000000) : (Option
           BigInt)`. Pins that a runtime BigInt rides the value-encode walker THROUGH a sum payload (the
           `Shape::BigInt` leaf nested under the Option's variant), the sum-payload companion of the bare
           runtime-BigInt escape — a BigInt in a sum crosses with its exact value, not dropped or folded.")
  (input  (Some (* (BigInt.of 1000000) (BigInt.of 1000000))))
  (output (: (Some 1000000000000) (Option BigInt))))

(case "a parameterized export drives an Option-of-runtime-BigInt on both arms"
  (doc    "The parameterized companion: `main(v)` returns `None` for v=0 else `(Some (v * 10^6))`, a
           runtime BigInt payload computed from the argument. v=5 → `(Some 5000000)`, v=0 → `None`. Pins
           that both the Some (runtime BigInt payload crossing) and None arms of an `(Option BigInt)`
           result render correctly from a boundary parameter.")
  (input  (do (def (main (: v Int64))
                (if (= v 0) (None) (Some (* (BigInt.of v) (BigInt.of 1000000))))) (export main)))
  (call   main (: 5 Int64)) (output (: (Some 5000000) (Option BigInt)))
  (call   main (: 0 Int64)) (output (: (None unit) (Option BigInt))))

(case "an arbitrary-precision literal beyond 64 bits is an exact BigInt"
  (doc    "`(: 100000000000000000000 BigInt)` annotates a literal larger than Int64.max as a BigInt — an
           exact value with no width worry. Pins that BigInt carries a magnitude the fixed-width family
           cannot (this literal would not fit any `(Int N)`/`(UInt N)` with N ≤ 64) and that its
           canonical written form is the ordinary decimal.")
  (input  (: 100000000000000000000 BigInt))
  (output (: 100000000000000000000 BigInt)))

(case "converting a fixed-width integer to BigInt is explicit"
  (doc    "`(BigInt.of 42)` converts the Int64 42 up to the BigInt 42 — the explicit widening into the
           unbounded type. A BigInt and the Int64 42 are DISTINCT types with distinct canonical forms;
           the conversion is always written, never implicit.")
  (input  (BigInt.of 42))
  (output (: 42 BigInt)))

(case "converting a BigInt back to a fixed width is checked and a compile-provable out-of-range narrowing is rejected"
  (doc    "`(UInt8.of (BigInt.of 300))` converts a BigInt down to `UInt8`, whose range is 0..=255, so 300
           does not fit. The conversion is CHECKED (numeric-model.md #A Conversion Between Integer Types Is
           Explicit — the range-checked form, never a silent truncation): the narrowing that overflows is
           a fault, not an accepted value. Here BOTH operands are constants, so the overflow is provable at
           compile time and the compiler REJECTS it (CDZ0302 — the checked conversion cannot fit the
           target), exactly as the constant-fold overflow of `+`/`*` is rejected rather than run to a trap
           (the runtime companion — a checked narrowing of a RUNTIME BigInt that overflows — traps at run
           time; a constant is caught earlier). Pins that narrowing OUT of BigInt is checked, and that a
           compile-provable out-of-range narrowing fails the build.")
  (input  (UInt8.of (BigInt.of 300)))
  (error  CDZ0302))

(case "a BigInt operation does not silently promote a fixed-width operand"
  (doc    "`(+ (BigInt.of 1) 1)` mixes a BigInt and an Int64 — two distinct numeric types — rejected
           (CDZ0301) rather than absorbing the Int64 1 into BigInt (numeric-model.md #Numeric Types Do
           Not Silently Promote). The unbounded type does not swallow a fixed-width operand; to add, a
           program writes `(+ (BigInt.of 1) (BigInt.of 1))`.")
  (input  (+ (BigInt.of 1) 1))
  (error  CDZ0301))

; --- BigInt constant folding: the widening + checked narrowing over COMPILE-TIME constants (B1) -----
; The seed realizes BigInt's CONSTRUCTION + CHECKED CONVERSION over compile-time constants first: a
; constant `(BigInt.of x)` widens (exact, never traps — every fixed-width value fits the unbounded type),
; and `(Int64.of b)` / `((UInt N).of b)` narrow it back CHECKED (range-checked at compile time on the
; constant). Because `IntValue` is already arbitrary-precision, the widening carries any magnitude and the
; narrowing's range check is exact. (A bare CONSTANT BigInt now crosses the host boundary as `(: N BigInt)`
; via the value-form escape — `Shape::BigInt` reuses the arbitrary-width KIND_INT leaf, no two's-complement
; needed; the runtime-valued escape awaits the general runtime-heap-return path, like a computed list/map.)

(case "a constant integer widens to BigInt and narrows back through Int64 unchanged"
  (doc    "`(Int64.of (BigInt.of 42))` widens the Int64 42 to a BigInt then narrows it back — the exact
           round-trip, yielding 42 : Int64. Pins that `BigInt.of` is the exact widening (no loss) and
           `Int64.of` its checked inverse; a value in range converts back unchanged, exactly as
           fixed-width `Int64.of` does.")
  (input  (Int64.of (BigInt.of 42)))
  (output (: 42 Int64)))

(case "the widening carries a full-width magnitude through BigInt"
  (doc    "`(Int64.of (BigInt.of 9223372036854775807))` round-trips Int64.max through BigInt — the
           BigInt carries the full 64-bit magnitude and narrows back in range. Pins that the widening
           does not truncate at any fixed width on the way up (the value is a `num-bigint` magnitude,
           unbounded), so a value up to the target's range survives the round-trip.")
  (input  (Int64.of (BigInt.of 9223372036854775807)))
  (output (: 9223372036854775807 Int64)))

(case "a constant BigInt crosses the host boundary as its value form"
  (doc    "A `main` returning a constant `BigInt` escapes to the host as the value form `(: N BigInt)` —
           the value-encode walker renders it via `Shape::BigInt` (descriptor tag 17), which reuses the
           codec's arbitrary-width `KIND_INT` leaf (sign + big-endian magnitude), so NO new wire kind and
           NO two's-complement form is needed. `(BigInt.of 42)` folds to a constant BigInt and crosses as
           `(: 42 BigInt)`. This is the bare-BigInt boundary the earlier note said 'awaits' — it is now
           the same value-form escape a Bytes/String/collection result uses.")
  (input  (do (def (main) ((. BigInt of) 42)) (export main)))
  (output (: 42 BigInt)))

(case "a negative constant BigInt crosses the boundary with its sign"
  (doc    "`(BigInt.of -7)` crosses as `(: -7 BigInt)` — the KIND_INT leaf carries the sign (the canonical
           form has no negative zero), so a negative BigInt renders correctly at the host boundary.")
  (input  (do (def (main) ((. BigInt of) -7)) (export main)))
  (output (: -7 BigInt)))

(case "narrowing a constant BigInt out of the target range is rejected at compile time"
  (doc    "`((UInt 8).of (BigInt.of 300))` narrows a BigInt to `(UInt 8)` (range 0..=255) where 300 does
           not fit — a statically-known out-of-range conversion the compiler rejects up front (CDZ0302,
           `integer does not fit the target`), exactly as `((UInt 8).of 300)` on a constant Int64 does.
           The checked narrowing OUT of BigInt is not a silent truncation. (A RUNTIME-valued BigInt
           out-of-range narrowing traps at run time — the contract case above; this pins the constant
           case the compiler decides statically.)")
  (input  (do (def (main) ((. UInt8 of) (BigInt.of 300))) (export main)))
  (error  CDZ0302))

; --- RUNTIME BigInt arithmetic — the unbounded ops run on the runtime limb library (B3b) ----------
; When an operand is not a compile-time constant (a BigInt PARAMETER, a derived value), the arithmetic
; runs at RUN TIME on the runtime's arbitrary-precision limb library: `BigInt.of` widens a runtime int
; into a BigInt, `+`/`-`/`*`/`/` never overflow (the representation grows — the whole point of the type),
; and `Int64.of` narrows back CHECKED. A value that overflows a fixed width mid-computation is still exact
; as a BigInt; only the final narrowing back to Int64 is range-checked.

(case "runtime BigInt addition runs on the arbitrary-precision runtime"
  (doc    "`(Int64.of (+ (BigInt.of a) (BigInt.of b)))` with runtime a,b widens each to a BigInt, adds on
           the runtime limb library, and narrows the in-range sum back: a=40,b=2 → 42. Pins the runtime
           path (a constant pair would fold; these are parameters, so the runtime `bigint-of-i64`/
           `bigint-add`/`bigint-to-i64-checked` ops run).")
  (input  (do
            (def (main (: a Int64) (: b Int64))
              (Int64.of (+ (BigInt.of a) (BigInt.of b))))
            (export main)))
  (call   main (: 40 Int64) (: 2 Int64))
  (output (: 42 Int64)))

(case "runtime BigInt remainder runs on the arbitrary-precision runtime"
  (doc    "`(Int64.of (% (BigInt.of a) (BigInt.of b)))` with runtime a,b takes the remainder on the runtime
           limb library (the `bigint-rem` op, backed by the same `divmod` as `bigint-div`): a=17,b=5 → 2.
           Pins that `%` — not just `/` — routes to the runtime BigInt path.")
  (input  (do
            (def (main (: a Int64) (: b Int64))
              (Int64.of (% (BigInt.of a) (BigInt.of b))))
            (export main)))
  (call   main (: 17 Int64) (: 5 Int64))
  (output (: 2 Int64)))

(case "a runtime BigInt remainder takes the dividend's sign"
  (doc    "`(% (BigInt.of a) (BigInt.of b))` is the remainder of TRUNCATING division, so it takes the
           DIVIDEND's sign (the companion of `bigint-div`'s truncate-toward-zero): a=-17,b=5 → -2 (not the
           floored +3). Matches fixed-width `%` semantics.")
  (input  (do
            (def (main (: a Int64) (: b Int64))
              (Int64.of (% (BigInt.of a) (BigInt.of b))))
            (export main)))
  (call   main (: -17 Int64) (: 5 Int64))
  (output (: -2 Int64)))

(case "a runtime BigInt intermediate that overflows Int64 does not trap"
  (doc    "`(Int64.of (/ (* big big) big))` with `big = BigInt.of 5000000000`: the product `big*big` =
           2.5e19 OVERFLOWS Int64 (max ~9.2e18), but BigInt's representation grows rather than trapping —
           the exact intermediate is carried, then `/big` brings it back to 5000000000, in range for the
           final narrowing. THE reason BigInt exists (numeric-model.md §An Arbitrary-Precision Integer Has
           Unbounded Range): the same expression over Int64 would trap at the multiply. Pins that the
           unbounded intermediate never overflows at run time.")
  (input  (do
            (def (main (: a Int64))
              (let ((big (BigInt.of a)))
                (Int64.of (/ (* big big) big))))
            (export main)))
  (call   main (: 5000000000 Int64))
  (output (: 5000000000 Int64)))

(case "a runtime BigInt operation does not silently promote a fixed-width operand"
  (doc    "`(+ (BigInt.of a) b)` with `a`,`b` runtime Int64 mixes a BigInt and an Int64 — rejected
           (CDZ0301) exactly as the constant mix is, at run-time-typed operands too. The no-promotion rule
           holds regardless of whether the operands are constant; to add, both must be BigInt.")
  (input  (do
            (def (main (: a Int64) (: b Int64)) (Int64.of (+ (BigInt.of a) b)))
            (export main)))
  (error  CDZ0301))

(case "a runtime BigInt ordering compares by the arbitrary-precision value"
  (doc    "`(< (BigInt.of a) (BigInt.of b))` with runtime a,b widens each to a BigInt and orders by the
           true value via the runtime three-way `bigint-cmp` + a signed compare-with-zero (B3c): a=2,b=5 →
           `<` true → 1. A BigInt has no fixed machine slot, so the comparison cannot ride the scalar path;
           it routes through the runtime primitive. Pins the runtime comparison wiring.")
  (input  (do
            (def (main (: a Int64) (: b Int64))
              (if (< (BigInt.of a) (BigInt.of b)) 1 0))
            (export main)))
  (call   main (: 2 Int64) (: 5 Int64))
  (output (: 1 Int64)))

(case "a runtime BigInt comparison sees an intermediate that overflows Int64"
  (doc    "`(> (* big big) big)` with `big = BigInt.of 5000000000`: the product 2.5e19 OVERFLOWS Int64 but
           is a valid BigInt, and `bigint-cmp` orders it against `big` by the TRUE unbounded value → the
           product is far larger → `>` true → 1. Pins that the comparison, like the arithmetic, sees the
           full arbitrary-precision magnitude rather than a wrapped/trapped machine value.")
  (input  (do
            (def (main (: a Int64))
              (let ((big (BigInt.of a)))
                (if (> (* big big) big) 1 0)))
            (export main)))
  (call   main (: 5000000000 Int64))
  (output (: 1 Int64)))

(case "a constant BigInt operand of a runtime BigInt op materializes as a heap value"
  (doc    "`(* big (BigInt.of 2))` with `big = BigInt.of a` runtime: the constant `(BigInt.of 2)` operand
           has no heap leaf of its own (it folded to a constant), so it MATERIALIZES inline as a BigInt
           leaf via `bigint-of-i64` at the use site (it fits i64) and the runtime `bigint-mul` runs: a=21
           → 42. Pins that a literal BigInt beside a runtime BigInt operand is accepted, not declined
           (the compiler boxes the constant rather than requiring both operands to be runtime).")
  (input  (do
            (def (main (: a Int64))
              (let ((big (BigInt.of a)))
                (Int64.of (* big (BigInt.of 2)))))
            (export main)))
  (call   main (: 21 Int64))
  (output (: 42 Int64)))

(case "runtime BigInt equality holds for values reached by different arithmetic"
  (doc    "`(= (+ big big) (* big (BigInt.of 2)))` with `big = BigInt.of a`: `big+big` and `big*2` are the
           SAME value reached two ways; `bigint-cmp` returns 0 → `=` (`i64.eqz`) true → 1. Confirms BigInt
           `=` compares by value (each op returns a normalized leaf, so equal values are byte-identical)
           rather than by handle identity — and that a constant `(BigInt.of 2)` operand materializes.")
  (input  (do
            (def (main (: a Int64))
              (let ((big (BigInt.of a)))
                (if (= (+ big big) (* big (BigInt.of 2))) 1 0)))
            (export main)))
  (call   main (: 123456789 Int64))
  (output (: 1 Int64)))

; --- Module pragma `default-integer`: fixes a literal's TYPE, never a conversion ----------
; A module MAY declare `(pragma default-integer <T>)` so a bare integer literal with no other constraint
; takes `<T>` instead of Int64 within that module (numeric-model.md #A Module May Declare Its Default
; Integer Literal Type). `pragma` is the module's compiler-directive channel (options/module-pragmas/):
; its key comes from a PINNED registry, and an UNRECOGNIZED key is REJECTED, not ignored (CDZ0601) — a
; directive that changes meaning must never be silently dropped (modules-and-namespaces.md #An
; Unrecognized Module Directive Is Rejected). THE LOAD-BEARING PROPERTY of `default-integer`: it fixes
; what type a literal STARTS as, and NOTHING else — every no-silent-promotion rule applies unchanged (#A
; Declared Default Fixes A Type, Not A Conversion). DEFINITION-SITE scoped (the module the literal is
; WRITTEN in, never a module that imports it — #A Declared Default Applies At The Definition Site), so
; importing a module never changes the type of its literals. An explicit annotation wins over the
; default. Resolved at compile time, then types erase — zero ABI impact. A generation without the
; extended numerics declines it.

(case "a default-integer pragma makes a bare literal take the declared integer type"
  (doc    "The `crypto` module declares `(pragma default-integer BigInt)`, so the bare literal 2 in
           `double`'s body is a BigInt, x is a BigInt, and `(double (BigInt.of 21))` = 42 : BigInt — the
           ergonomic escape hatch: a bignum-heavy module writes bare literals without `(BigInt.of …)`
           around each. Pins that the declared default is the type an unconstrained literal takes.")
  (input  (do
            (module crypto
              (pragma default-integer BigInt)
              (def (double x) (* x 2)))
            ((. crypto double) (BigInt.of 21))))
  (output (: 42 BigInt)))

(case "a default-integer pragma fixes a type but adds no conversion — no-promotion still holds"
  (doc    "In a `(pragma default-integer BigInt)` module, `(mix)` writes `(+ 2 (Int64.of 1))`: the bare 2
           is a BigInt (the module default), `(Int64.of 1)` is an Int64, so the mix is rejected (CDZ0301)
           exactly as any BigInt/Int64 mix is (numeric-model.md #A Declared Default Fixes A Type, Not A
           Conversion). THE load-bearing case: the default changed what type the literal STARTS as, it
           did NOT introduce a coercion — the feature buys ergonomics without weakening no-promotion.")
  (input  (do
            (module m
              (pragma default-integer BigInt)
              (def (mix) (+ 2 (Int64.of 1))))
            ((. m mix) unit)))
  (error  CDZ0301))

(case "an explicit annotation overrides the default-integer pragma"
  (doc    "In a `(pragma default-integer BigInt)` module, `(: 5 Int64)` is still Int64 — an explicit
           annotation takes precedence over the module default (numeric-model.md #A Declared Default
           Fixes A Type…, last sentence). Pins that the default only decides the OTHERWISE-UNCONSTRAINED
           case; a constrained literal keeps its constrained type.")
  (input  (do
            (module m
              (pragma default-integer BigInt)
              (def (pinned) (: 5 Int64)))
            ((. m pinned) unit)))
  (output (: 5 Int64)))

(case "the default-integer pragma is definition-site scoped, not use-site"
  (doc    "`lib` declares no pragma (its literals are Int64); `app` declares `(pragma default-integer
           BigInt)`. `app` calls `(. lib answer)`, whose body's literal 42 was WRITTEN in `lib` — so it
           stays Int64 regardless of `app`'s default (numeric-model.md #A Declared Default Applies At The
           Definition Site). Importing a module never changes the type of code inside it; the result is
           Int64, not BigInt. THE case that keeps the feature compatible with separate compilation.")
  (input  (do
            (module lib
              (def (answer) 42))
            (module app
              (pragma default-integer BigInt)
              (def (go) ((. lib answer) unit)))
            ((. app go) unit)))
  (output (: 42 Int64)))

(case "a default-integer pragma naming a non-integer type is rejected"
  (doc    "`(pragma default-integer Float64)` names a type that is not an integer the numeric model
           admits, so the module is rejected (CDZ0303, numeric-model.md #A Module May Declare Its Default
           Integer Literal Type). The pragma KEY is recognized and its argument is a valid type — it just
           fails the integer-domain predicate — so this is the numeric CDZ0303, distinct from the
           structural CDZ0602 (malformed args) and CDZ0601 (unknown key).")
  (input  (do
            (module m
              (pragma default-integer Float64)
              (def (x) 5))
            ((. m x) unit)))
  (error  CDZ0303))

(case "a bare literal exceeding the default-integer pragma's narrow type is rejected"
  (doc    "`(pragma default-integer Int8)` makes bare literals Int8. A bare `300` is out of Int8's range
           (-128..=127), so it must be REJECTED with the SAME literal-fit check an explicit `(: 300 Int8)`
           runs (CDZ0302) — not silently accepted at its full value. Without this, the pragma applied the
           type TAG (`x : Int8`, it feeds an Int8 param) but SKIPPED the fit-check, so `(Int64.of x)` read
           back 300: an Int8 holding an out-of-range value, a soundness hole the explicit annotation
           correctly rejects (numeric-model.md #A Bare Integer Literal Is Grounded By Its Annotation,
           Subject To A Range Check — the pragma default is a grounding, so the same range check applies).
           A WIDENING default (BigInt/Int64) never faults (every literal fits); only a narrowing one
           (Int8/UInt8/…) catches an out-of-range literal.")
  (input  (do
            (module m
              (pragma default-integer Int8)
              (def (x) 300))
            (Int64.of ((. m x) unit))))
  (error  CDZ0302))

(case "a default-integer pragma naming an unbound type is rejected as unbound, like an annotation"
  (doc    "`(pragma default-integer Nope)` names a type `Nope` that does not exist — no prelude type, no
           declared type. The SAME name in a type-annotation position (`(: x Nope)`) is CDZ0101 'unbound
           name', and a meaning-changing directive naming a nonexistent type must not be silently accepted
           (a dropped directive makes one source mean two things across toolchains,
           modules-and-namespaces.md #An Unrecognized Module Directive Is Rejected). Resolution — not the
           `typeval_of` reduction — is what tells an UNBOUND name (a `Poison` CDZ0101) apart from a BOUND
           type this compiler does not yet model as a `Ty`: the numeric-domain predicate conservatively
           accepts an argument that does not reduce to a concrete `Ty` (so a legitimate unmodeled integer
           default is not falsely rejected), but an unbound name is distinguishable, so it is the SAME
           CDZ0101 the annotation gives, not a silent accept.")
  (input  (do
            (module m
              (pragma default-integer Nope)
              (def (x) 5))
            ((. m x) unit)))
  (error  CDZ0101))
; (The general pragma mechanism — an unrecognized key rejected CDZ0601, malformed args CDZ0602 — is
;  witnessed by the module-pragma cases in 11-modules.sexp; here we pin only the numeric-domain
;  behavior of the `default-integer` key.)

; --- Module pragma `default-fraction`: exact-by-default ------------------------------------
; The exactness sibling of `default-integer`. A module MAY declare `(pragma default-fraction Rational)`
; so a bare NUMERIC literal (integer OR decimal) with no other constraint grounds to the exact rational
; it denotes within that module (numeric-model.md #A Module May Declare Its Default Fraction Literal
; Type) — making ordinary arithmetic exact by default: `(/ 1 3)` is 1/3, not integer-truncated 0. Same
; discipline as default-integer: definition-site scoped, fixes a TYPE not a conversion (no-promotion
; holds), an explicit annotation wins, and the directive MUST name an exact rational type (a non-rational
; is the numeric-domain CDZ0303).

(case "a default-fraction pragma makes a bare literal exact — 1/3 not 0"
  (doc    "`m` declares `(pragma default-fraction Rational)`, so the bare literals 1 and 3 in `third`'s
           body are Rationals — `(/ 1 3)` is EXACT rational division = 1/3, not the integer-truncated 0 a
           default-Int64 module gives. THE load-bearing effect: exact-by-default, the calculator's reason
           for the directive (numeric-model.md #A Module May Declare Its Default Fraction Literal Type).")
  (input  (do
            (module m
              (pragma default-fraction Rational)
              (def (third) (/ 1 3)))
            ((. m third) unit)))
  (output (: 1/3 Rational)))

(case "a default-fraction pragma grounds a bare DECIMAL literal to its exact fraction"
  (doc    "The default applies to a decimal-written literal too: bare `0.5` in a `(pragma default-fraction
           Rational)` module is the EXACT fraction its digits denote, 1/2 — no float rounding
           (numeric-model.md #A declared default fraction literal type MUST apply to both an integer- and a
           decimal-written literal). So `0.5` is 1/2 : Rational, not 0.5 : Float64.")
  (input  (do
            (module m
              (pragma default-fraction Rational)
              (def (half) 0.5))
            ((. m half) unit)))
  (output (: 1/2 Rational)))

(case "a default-fraction pragma fixes a type but adds no conversion — no-promotion still holds"
  (doc    "In a `(pragma default-fraction Rational)` module, `(mix)` writes `(+ 1 (Int64.of 1))`: the bare
           1 is a Rational (the module default), `(Int64.of 1)` is an Int64, so the mix is rejected
           (CDZ0301) exactly as any Rational/Int64 mix is (numeric-model.md #A declared default fraction …
           introduces no implicit conversion). The default changed what type the literal STARTS as, it did
           NOT add a coercion — exactness ergonomics without weakening no-promotion.")
  (input  (do
            (module m
              (pragma default-fraction Rational)
              (def (mix) (+ 1 (Int64.of 1))))
            ((. m mix) unit)))
  (error  CDZ0301))

(case "an explicit annotation overrides the default-fraction pragma"
  (doc    "In a `(pragma default-fraction Rational)` module, `(: 5 Int64)` is still Int64 — an explicit
           annotation takes precedence over the module default (numeric-model.md, the fraction default
           inherits the integer default's override rule). Pins that the default only decides the
           OTHERWISE-UNCONSTRAINED case; a constrained literal keeps its constrained type.")
  (input  (do
            (module m
              (pragma default-fraction Rational)
              (def (pinned) (: 5 Int64)))
            ((. m pinned) unit)))
  (output (: 5 Int64)))

(case "a default-fraction pragma naming a non-rational type is rejected"
  (doc    "`(pragma default-fraction Int64)` names a type that is not an exact rational, so the module is
           rejected (CDZ0303, numeric-model.md #A Module May Declare Its Default Fraction Literal Type).
           The KEY is recognized and the argument is a valid type — it just fails the rational-domain
           predicate — so this is the numeric CDZ0303, the fraction twin of the default-integer domain
           reject, distinct from the structural CDZ0602/CDZ0601.")
  (input  (do
            (module m
              (pragma default-fraction Int64)
              (def (x) 5))
            ((. m x) unit)))
  (error  CDZ0303))

(case "floating-point uses the fixed rounding mode"
  (doc    "The round-to-nearest-even sum under the pinned deterministic float mode
           (contracts/determinism-and-fuel.md); byte-identical on every conforming runtime. Written with
           the ONE arithmetic operator `+` — 0.1 and 0.2 are Float64, so `+` is float addition, dispatched
           on the operand type (numeric-model.md #An Arithmetic Operator Requires Both Operands To Be One
           Numeric Type). The famous non-exact sum: 0.1 + 0.2 rounds to 0.30000000000000004, not 0.3.")
  (input  (+ 0.1 0.2))
  (output (: 0.30000000000000004 Float64)))

; --- Float arithmetic uses the ONE arithmetic operator `+` `-` `*` `/` -------------------------------
; numeric-model.md #An Arithmetic Operator Requires Both Operands To Be One Numeric Type: there is a
; SINGLE arithmetic-operator spelling; the operand type selects the operation. `+`/`-`/`*`/`/` over two
; Float64 operands is float arithmetic; over two Int64 operands it is integer arithmetic. Both operands
; must be one numeric type — a mix (a float and an integer) → CDZ0301 (the mixing cases above), in EITHER
; operand order. These pin `+`/`-`/`*`/`/` over Float64 operands and the mixed-operand rejection.

(case "float multiplication uses the arithmetic operator"
  (doc    "`(* 6.0 7.0)` = 42.0 : Float64 — float multiplication under `*`, dispatched on the Float64
           operands. Result is a Float64 (42.0), not the Int64 42 the integer `(* 6 7)` gives — the same
           operator, the operand type deciding.")
  (input  (* 6.0 7.0))
  (output (: 42.0 Float64)))

(case "float subtraction uses the arithmetic operator"
  (doc    "`(- 5.5 2.0)` = 3.5 : Float64 — float subtraction under `-`, dispatched on the Float64 operands.")
  (input  (- 5.5 2.0))
  (output (: 3.5 Float64)))

(case "float division rounds under the fixed mode"
  (doc    "`(/ 1.0 4.0)` = 0.25 : Float64 — float division under `/` over Float64 operands, which ROUNDS
           (unlike integer `/` over Int64 which truncates and rational `/` which is exact — the same
           operator, the operand type deciding). 1/4 is exactly representable, so 0.25.")
  (input  (/ 1.0 4.0))
  (output (: 0.25 Float64)))

(case "float division that does not divide evenly rounds to nearest"
  (doc    "`(/ 1.0 3.0)` = 0.3333333333333333 : Float64 — 1/3 is not exactly representable in binary64,
           so the quotient rounds to the nearest representable value under the fixed round-to-nearest-even
           mode. Pins that float `/` (Float64 operands) rounds deterministically.")
  (input  (/ 1.0 3.0))
  (output (: 0.3333333333333333 Float64)))

(case "the arithmetic operator rejects a float-first mixed operand pair"
  (doc    "`(+ 2.0 2)` supplies a Float64 `2.0` and an Int64 `2` — two distinct numeric types under the ONE
           arithmetic operator — so it is rejected (CDZ0301), NOT promoted (numeric-model.md #An Arithmetic
           Operator Requires Both Operands To Be One Numeric Type; #Numeric Types Do Not Silently Promote).
           The operand-order dual of `(+ 2 2.0)` above (integer-first): the rejection follows from the
           operands disagreeing, regardless of which operand comes first — neither operand's type wins.")
  (input  (+ 2.0 2))
  (error  CDZ0301))

; --- Runtime float operands: the EMITTED machine op, not the constant fold -----------------------
; The float-arithmetic cases above use CONSTANT operands, so the compiler folds them at build time. A
; value that arrives at RUN TIME (an argument to the exported entry) cannot be folded, so the arithmetic
; operator is emitted as a real machine instruction (`f64.add`/…) — dispatched to the float op by the
; Float64 operand type. These `(call <export> <arg>…)` cases run each arithmetic operator over runtime
; Float64 operands and pin that the emitted path AGREES with the folded constant cases. Unlike the integer
; arithmetic these emit NO overflow guard — a float op never traps (IEEE overflow → inf). CORE cases (the
; seed realizes runtime Float64 operators).

(case "a runtime float addition emits the machine add"
  (doc    "`(def (main (: a Float64) (: b Float64)) (+ a b))` called with (0.1, 0.2). The addition cannot
           fold (both operands are runtime), so it is emitted as `f64.add` — the non-exact IEEE sum
           0.30000000000000004, matching the folded `(+ 0.1 0.2)` case. Pins the emitted float-add path.")
  (input  (do (def (main (: a Float64) (: b Float64)) (+ a b)) (export main)))
  (call   main (: 0.1 Float64) (: 0.2 Float64))
  (output (: 0.30000000000000004 Float64)))

(case "a runtime float multiplication emits the machine mul"
  (doc    "`(* a b)` over runtime Float64 operands emits `f64.mul`; `(6.0, 7.0)` = 42.0, matching the
           folded `(* 6.0 7.0)`. Pins the emitted float-multiply path.")
  (input  (do (def (main (: a Float64) (: b Float64)) (* a b)) (export main)))
  (call   main (: 6.0 Float64) (: 7.0 Float64))
  (output (: 42.0 Float64)))

(case "a runtime float division rounds under the fixed mode"
  (doc    "`(/ a b)` over runtime Float64 operands emits `f64.div`, which rounds under the fixed round-to-
           nearest-even mode; `(1.0, 3.0)` = 0.3333333333333333, matching the folded `(/ 1.0 3.0)`. Pins
           the emitted float-divide path and that it rounds deterministically (not a trap on inexactness).")
  (input  (do (def (main (: a Float64) (: b Float64)) (/ a b)) (export main)))
  (call   main (: 1.0 Float64) (: 3.0 Float64))
  (output (: 0.3333333333333333 Float64)))

(case "a runtime integer converts to a float with the machine convert"
  (doc    "`(Float64.of-int n)` over a runtime Int64 `n` emits `f64.convert_i64_s`; `(of-int 42)` = 42.0.
           The explicit int→float conversion (numeric-model.md #A Conversion Involving A Floating-Point
           Type Is Explicit) is TOTAL — an integer always has a float image (a large magnitude rounds to
           the nearest representable float, it does not trap). Pins the emitted int→float convert path,
           the runtime dual of the folded `(Float64.of-int 1)` conversion.")
  (input  (do (def (main (: n Int64)) (Float64.of-int n)) (export main)))
  (call   main (: 42 Int64))
  (output (: 42.0 Float64)))

(case "a runtime float promotes to a wider width with the machine promote"
  (doc    "`(Float64.of x)` over a runtime Float32 `x` emits `f64.promote_f32` — exact widening; `1.5`
           promotes to 1.5 : Float64. Pins the emitted promote path, the runtime dual of the folded
           promote case above.")
  (input  (do (def (main (: x Float32)) (Float64.of x)) (export main)))
  (call   main (: 1.5 Float32))
  (output (: 1.5 Float64)))

; --- Float is WIDTH-INDEXED: (Float N) over N in {32, 64}, with Float32/Float64 aliases -------------
; numeric-model.md #A Floating-Point Type Is Indexed By A Compile-Time Width: a float type is the
; width-indexed constructor `Float` applied to a compile-time width, and Float32/Float64 alias
; (Float 32)/(Float 64) — the exact machinery Int/UInt use (options/numeric-model/). The admitted set is
; {32, 64} (the widths the backend provides as f32/f64); a width outside it is CDZ0302, exactly as an
; out-of-range integer width is. These pin the parametric surface, the alias equivalence, and the
; width constraint.

(case "a float annotation reaches the 32-bit width"
  (doc    "`(: 3.5 Float32)` is the binary32 value 3.5 — an annotation reaches a float width other than
           the default Float64 (numeric-model.md #A Floating-Point Type Is Indexed By A Compile-Time
           Width). 3.5 is exactly representable in binary32, so its value is 3.5 at type Float32; the
           boundary maps Float32 to the component model's f32.")
  (input  (: 3.5 Float32))
  (output (: 3.5 Float32)))

(case "a named float width alias and its width-indexed expansion name the same type"
  (doc    "`(: 3.5 (Float 32))` is the same binary32 value 3.5 as `(: 3.5 Float32)` — `Float32` is the
           alias `(Float 32)`, exactly as `UInt8` aliases `(UInt 8)`. The width-indexed constructor
           applied to 32 is the same type the alias names, not a distinct one.")
  (input  (: 3.5 (Float 32)))
  (output (: 3.5 Float32)))

(case "the width-indexed and aliased float annotations of one value do not conflict"
  (doc    "`(: (: 1.5 (Float 64)) Float64)` annotates a value first as `(Float 64)` then as `Float64` —
           the same type under two names, so the annotations agree (NOT a CDZ0203 conflict). The float
           analogue of the integer alias-equivalence-through-the-annotation-checker case.")
  (input  (: (: 1.5 (Float 64)) Float64))
  (output (: 1.5 Float64)))

(case "mixing two float widths without a conversion does not silently promote"
  (doc    "`(+ (: 1.5 Float32) (: 2.0 Float64))` mixes a Float32 and a Float64 — two distinct float
           types — so it is rejected (CDZ0301) rather than silently widening the Float32 to Float64
           (numeric-model.md #Numeric Types Do Not Silently Promote; #A Conversion Involving A
           Floating-Point Type Is Explicit). The float-width analogue of the integer-width no-promotion
           case; to add them a program converts one side (`(Float64.of …)`).")
  (input  (+ (: 1.5 Float32) (: 2.0 Float64)))
  (error  CDZ0301))

; --- Explicit conversion between float widths: Float64.of promotes, Float32.of demotes ------------
; numeric-model.md #A Conversion Involving A Floating-Point Type Is Explicit — "between two floating-
; point types of different width" — is written `Float N.of`, the float-width analogue of the integer
; `T.of`/`Float N.of-int`. `Float64.of` from a Float32 PROMOTES (widening, exact — every binary32 is a
; binary64); `Float32.of` from a Float64 DEMOTES (narrowing, rounds to the nearest binary32 under the
; fixed mode). This is what resolves the no-promotion rejection above: to add a Float32 and a Float64 a
; program converts one side explicitly.

(case "promoting a Float32 to Float64 is exact"
  (doc    "`(Float64.of (: 1.5 Float32))` widens the binary32 1.5 to Float64 — exact (every binary32
           value is representable in binary64), so the result is 1.5 : Float64. The explicit widening
           the no-promotion rule requires; `Float64.of` from a narrower float is the promote.")
  (input  (Float64.of (: 1.5 Float32)))
  (output (: 1.5 Float64)))

(case "demoting a Float64 to Float32 rounds to the nearest binary32"
  (doc    "`(Float32.of 0.1)` narrows the binary64 0.1 to Float32, which rounds to the nearest
           representable binary32 — 0.10000000149011612 when read back as the canonical value form (the
           binary32 nearest to 0.1 is not 0.1). Pins that `Float32.of` DEMOTES with rounding under the
           fixed mode (numeric-model.md #A Conversion Involving A Floating-Point Type Is Explicit), the
           narrowing companion of the exact promote.")
  (input  (Float32.of 0.1))
  (output (: 0.10000000149011612 Float32)))

(case "an explicit float-width conversion makes a mixed-width operation well-typed"
  (doc    "The no-promotion rejection `(+ (: 1.5 Float32) (: 2.0 Float64))` is resolved by converting
           the Float32 up explicitly: `(+ (Float64.of (: 1.5 Float32)) 2.0)` = 3.5 : Float64 — both
           operands are now Float64, so `+` type-checks and adds. Pins that the explicit conversion is
           the sanctioned way to combine two float widths.")
  (input  (+ (Float64.of (: 1.5 Float32)) 2.0))
  (output (: 3.5 Float64)))

(case "converting a float to its own width is the identity"
  (doc    "`(Float64.of 2.5)` where the operand is already a Float64 is 2.5 : Float64 — a same-width
           conversion is the identity (no rounding). Pins that `Float N.of` accepts a Float of the same
           width, not only a different one.")
  (input  (Float64.of 2.5))
  (output (: 2.5 Float64)))

(case "a float width outside the admitted set is rejected"
  (doc    "`(: 1.5 (Float 16))` names a 16-bit float, which is not in the admitted set {32, 64} — the
           widths the backend provides — so the type is rejected at compile time (CDZ0302), the float
           analogue of `(UInt 65)` / `(UInt 0)` failing the integer width constraint. Not a runtime
           trap: the type itself is ill-formed. `(Float 16)` (binary16) is reserved to a later increment,
           exactly as `(UInt 128)` is reserved above the 64-bit ceiling.")
  (input  (: 1.5 (Float 16)))
  (error  CDZ0302))

(case "a non-power-of-two float width is rejected"
  (doc    "`(: 1.5 (Float 48))` names a 48-bit float — not an IEEE binary format the model admits — so it
           is rejected (CDZ0302). Unlike integers (where every width in 1..=64 is a first-class type),
           float widths are drawn from the fixed IEEE set {32, 64}; an arbitrary width is not a float
           type. Pins that the float width constraint is set-membership, not a range.")
  (input  (: 1.5 (Float 48)))
  (error  CDZ0302))

(case "a non-admitted float width NESTED in a compound annotation is rejected"
  (doc    "`(: (list 1.0) (List (Float 8)))` carries the non-admitted float width `8` one level down, in a
           `List` element type. `(Float 8)` reduces to a well-formed container of the sentinel float type,
           so the top-level annotation LOOKS valid and the ill-formed width slipped past `cdz check` (which
           exited 0) while the value compiled with a default width — the float twin of the nested-INTEGER
           -width gap, and one nesting level deeper than the bare `(Float 16)` reject above. The front-end
           descends the compound annotation and rejects CDZ0302 at the nested float width, exactly as if it
           were written bare. Set-membership {32,64} is checked wherever a float width appears.")
  (input  (: (list 1.0) (List (Float 8))))
  (error  CDZ0302))

(case "a non-admitted float width in a parameter annotation is rejected"
  (doc    "`(Float 8)` as the type of a parameter of a private, never-called def — `(def (f (: x (Float
           8))) x)` with `f` unused — is rejected CDZ0302, exactly as the value annotation `(: 1.5 (Float
           8))` is. Before, the parameter path had NO float-width check at all, so a bad float width in a
           parameter type slipped past `cdz check` entirely; well-formedness is TOTAL (a width is ill-formed
           wherever the annotation appears, reachable or not — an unbound name in the same unused def is
           CDZ0101), so the float admitted-set constraint is checked at the annotation itself, the float
           companion of the integer parameter-width case.")
  (input  (do
            (def (f (: x (Float 8))) x)
            (def (main) 0)
            (export main)))
  (error  CDZ0302))

(case "a non-admitted float width in a type-declaration payload is rejected"
  (doc    "`(type T (Mk (Float 8)))` puts the non-admitted float width `8` in a variant payload field of a
           type declaration — a type-expression position the shared front-end validates, not a value
           annotation. A float width outside {32,64} is rejected CDZ0302 at the declaration, before any
           value of `T` is constructed, exactly as the same width in a value or parameter annotation is (and
           as an ill-formed INTEGER width in a payload field is). Pins that the float admitted-set
           constraint is TOTAL over every type-expression position, declaration payloads included.")
  (input  (do
            (type T (Mk (Float 8)))
            (def (main) 0)
            (export main)))
  (error  CDZ0302))

(case "subtraction"
  (input  (- 10 3))
  (output (: 7 Int64)))

; --- Subtraction and multiplication overflow trap, exactly as addition does --------------
; #Overflow Is Defined is stated for "an integer operation that overflows its type", not only
; for `+`. Addition overflow (`(+ Int64.max 1)`) and division overflow (`(/ Int64.min -1)`)
; are already witnessed; subtraction and multiplication are the remaining checked operations
; whose overflow must reach the same defined outcome (a trap under the checked-Int64 default).
; The seed routes `-`/`*` through the same overflow-checked helpers as `+`, so these must trap
; rather than wrap to a wrong value — a silent two's-complement wrap is the classic C-style
; undefined behavior the numeric model forbids ("The compiler MUST NOT emit an integer operation
; whose overflow behavior is undefined").

(case "subtraction below the minimum integer overflows and traps"
  (doc    "`(- Int64.min 1)` = -2^63 - 1, one below the checked Int64 range, so it overflows. The
           compiler can PROVE this overflow via constant folding, so it rejects at compile time
           (CDZ0304) rather than emitting a runtime trap. Static safety: catch provable errors early.")
  (input  (- Int64.min 1))
  (error  CDZ0304))

(case "multiplication past the maximum integer overflows and traps"
  (doc    "`(* Int64.max 2)` is ~2^64, well outside the checked Int64 range, so it overflows. The
           compiler can prove this via constant folding and rejects at compile time (CDZ0304) rather
           than emitting a runtime trap. Static safety: provable overflows are build errors.")
  (input  (* Int64.max 2))
  (error  CDZ0304))

(case "multiplication of the minimum integer by -1 overflows and traps"
  (doc    "`(* Int64.min -1)` = +2^63, one past Int64.max — the compiler can prove this overflow via
           constant folding and rejects at compile time (CDZ0304) rather than emitting a runtime trap.")
  (input  (* -9223372036854775808 -1))
  (error  CDZ0304))

; A RUNTIME subtraction/multiplication (parameter operands) must trap identically to the constant
; fold above — the overflow check belongs on the emitted operation, not only in the constant folder.
; These runtime companions pin that the checked helper is emitted for `-`/`*` on runtime operands,
; so the const and runtime paths agree (the same const-vs-runtime discipline the shift cases pin).

(case "a runtime multiplication that overflows traps"
  (doc    "The 'runtime' companion of `(* Int64.max 2)`: with the operands supplied as parameters,
           the compiler β-reduces by substituting the constant arguments, turning the body into
           `(* 9223372036854775807 2)` — now a provable constant overflow. The compiler rejects at
           compile time (CDZ0304) via β-reduction, exactly as direct constant expressions do.")
  (input  (do
            (def (mul a b) (* a b))
            (def (main) (mul 9223372036854775807 2)) (export main)))
  (error  CDZ0304))

(case "a runtime subtraction that overflows traps"
  (doc    "The 'runtime' companion of `(- Int64.min 1)`: with constant arguments supplied as parameters,
           the compiler β-reduces by substituting them, turning the body into `(- -9223372036854775808 1)`
           — now a provable constant overflow. The compiler rejects at compile time (CDZ0304).")
  (input  (do
            (def (sub a b) (- a b))
            (def (main) (sub -9223372036854775808 1)) (export main)))
  (error  CDZ0304))

(case "multiplication"
  (input  (* 6 7))
  (output (: 42 Int64)))

(case "integer division truncates toward zero"
  (input  (/ 7 2))
  (output (: 3 Int64)))

; --- Division and remainder for negative operands: truncate toward zero, remainder follows -
; --- the dividend's sign ----------------------------------------------------------------
; Integer division "truncates toward zero" (witnessed above for `(/ 7 2)` = 3). Truncation toward
; zero is a definite rule that differs from flooring (toward -∞) precisely on negative operands:
; `(/ -7 2)` truncates to -3 (floor would give -4). The remainder is then fixed by the identity
; `a = (/ a b)*b + (% a b)`, so `(% a b)` takes the sign of the DIVIDEND `a` — `(% -7 2)` = -1,
; `(% 7 -2)` = 1. These pin the sign conventions the LEB128/section-encoding math depends on; a
; lowering that floored (or took the divisor's sign for the remainder) would encode wrong bytes.
; The ASCII `(/ 7 2)` case above cannot witness this — both operands are non-negative there.

(case "integer division of a negative dividend truncates toward zero, not toward negative infinity"
  (doc    "`(/ -7 2)` truncates toward zero to -3, NOT floors to -4. Truncation toward zero is the
           pinned rule (the `(/ 7 2)` = 3 case states it); it diverges from flooring only for negative
           operands, so this is the case that actually distinguishes the two. wasm's i64.div_s
           truncates toward zero, matching this.")
  (input  (/ -7 2))
  (output (: -3 Int64)))

(case "a runtime negative dividend divided by a constant power of two truncates toward zero"
  (doc    "Division by a constant power of two may be strength-reduced to a shift, but a signed division
           truncates toward ZERO while an arithmetic right shift floors toward −∞ — so the strength
           reduction must add a bias before shifting, or a negative dividend gives the wrong quotient.
           `(/ n 4)` with the RUNTIME parameter `n` = −7 must be −1 (−7 truncated toward zero), NOT −2 (what
           `-7 >> 2` yields). Pins that the constant-power-of-two `/` strength reduction reproduces
           truncation-toward-zero for a negative runtime dividend — the runtime-emit companion of the
           constant `(/ -7 2)` = −3 case above (which folds; this exercises the bias+shift the emit uses).
           The remainder companion `(% n 4)` = −3 (sign of the dividend) is pinned below.")
  (input  (do
            (def (main (: n Int64)) (/ n 4))
            (export main)))
  (call   main (: -7 Int64))
  (output (: -1 Int64)))

(case "a runtime negative dividend mod a constant power of two takes the dividend's sign"
  (doc    "The remainder companion: `(% n 4)` with `n` = −7 must be −3 (the remainder takes the sign of the
           DIVIDEND), NOT 1 (what the bitmask `n & 3` yields). A strength reduction of signed `%` by a
           power of two to a bitmask is only valid for a non-negative dividend; a negative one needs the
           sign-correcting form. Pins that `%` by a constant power of two reproduces the dividend-signed
           remainder for a negative runtime value.")
  (input  (do
            (def (main (: n Int64)) (% n 4))
            (export main)))
  (call   main (: -7 Int64))
  (output (: -3 Int64)))

(case "integer division by a negative divisor truncates toward zero"
  (doc    "`(/ 7 -2)` = -3: the quotient's magnitude is 3 (truncated, not 4) and its sign is negative.
           Pins that truncation toward zero holds when the DIVISOR is the negative operand too.")
  (input  (/ 7 -2))
  (output (: -3 Int64)))

(case "the remainder takes the sign of the dividend for a negative dividend"
  (doc    "`(% -7 2)` = -1: from the identity a = (a/b)*b + (a%b) with (/ -7 2) = -3, the remainder is
           -7 - (-3*2) = -1, taking the DIVIDEND's sign. (A flooring modulo would give +1.) Pins the
           remainder-sign convention wasm's i64.rem_s follows.")
  (input  (% -7 2))
  (output (: -1 Int64)))

(case "the remainder takes the sign of the dividend for a negative divisor"
  (doc    "`(% 7 -2)` = 1: with (/ 7 -2) = -3, the remainder is 7 - (-3*-2) = 1 — positive, the sign of
           the dividend 7, not the divisor. Pins that the remainder sign follows the dividend
           regardless of the divisor's sign.")
  (input  (% 7 -2))
  (output (: 1 Int64)))

; --- Division and modulo by zero have no result: they trap -------------------------------
; core-semantics.md #Partial Operations Have A Defined Outcome: an operation with no result for some
; inputs MUST raise a trap of a defined kind rather than produce an unspecified value. Division and
; modulo by zero have no result, so they MUST trap — the seed realizes this runtime trap (a trap
; that survives type-checking, README.md §"Which cases a generation runs": "division by zero"). This
; is distinct from the OVERFLOW trap `(/ Int64.min -1)`: divide-by-zero has no quotient at all, at
; any dividend. A lowering that emitted a value (0, or the dividend) would be an unspecified value the
; contract forbids; wasm's i64.div_s / i64.rem_s trap on a zero divisor, matching this.

(case "division by zero traps"
  (doc    "`(/ 5 0)` has no quotient — division by zero has no result. The compiler can prove the
           divisor is zero via constant folding, so it rejects at compile time (CDZ0304) rather than
           emitting a runtime trap. Static safety: catch provable errors early.")
  (input  (/ 5 0))
  (error  CDZ0304))

(case "modulo by zero traps"
  (doc    "`(% 5 0)` likewise has no remainder when the divisor is zero. The compiler can prove the
           divisor is zero via constant folding, so it rejects at compile time (CDZ0304) rather than
           emitting a runtime trap. The modulo companion of division by zero.")
  (input  (% 5 0))
  (error  CDZ0304))

(case "a runtime division by zero traps"
  (doc    "The 'runtime' companion: with a zero divisor supplied as a parameter, the compiler β-reduces
           by substituting the constant arguments, turning the body into `(/ 5 0)` — now a provable
           division by zero. The compiler rejects at compile time (CDZ0304) via β-reduction.")
  (input  (do
            (def (div a b) (/ a b))
            (def (main) (div 5 0)) (export main)))
  (error  CDZ0304))

(case "a division whose divisor folds to zero still traps"
  (doc    "`(/ 10 (- 3 3))`: the divisor is not the literal 0 but a constant expression that reduces to
           0. Constant folding reduces `(- 3 3)` to 0, making the division provably division-by-zero.
           The compiler rejects at compile time (CDZ0304) because it can prove the trap via bottom-up
           constant folding, exactly as `(/ 10 0)` is rejected.")
  (input  (/ 10 (- 3 3)))
  (error  CDZ0304))

; A COMPARISON that FOLDS to a constant (a tautology `>= min` / unsatisfiable `< min`, against the
; operand's type bound OR a derived masked/shifted range) must NOT discard the operand's evaluation —
; a trapping operand still traps (core-semantics.md #Partial Operations Have A Defined Outcome). The
; fold is an optimization on the comparison RESULT; it must preserve the operand's effect. A
; provably-total operand may be dropped, but a possibly-trapping one keeps the runtime comparison (which
; evaluates it). These use a RUNTIME divisor `z` so the div-by-zero fires at run time, under a
; comparison the compiler folds to a constant.

(case "a tautology comparison against the type minimum still traps on a div-by-zero operand"
  (doc    "`(>= (/ 10 z) Int64.min)` is a tautology — every Int64 is >= Int64.min — so the compiler folds
           it to true. But the operand `(/ 10 z)` with z = 0 divides by zero and MUST trap: the fold to a
           constant must not drop the operand's evaluation. A genuine comparison `(< (/ 10 z) 5)` traps on
           z = 0, and the self-comparison fold preserves the operand's trap — the type-bound fold must too.")
  (input  (do (def (main (: z Int64)) (if (>= (/ 10 z) -9223372036854775808) 1 0)) (export main)))
  (call   main (: 0 Int64))
  (trap   "divide by zero"))

(case "a tautology comparison against a masked value's derived range still traps on the operand"
  (doc    "`(< (& (/ 10 z) 15) 16)` — the masked value `(& _ 15)` lives in [0,15], so `< 16` is a
           tautology the derived-range fold collapses to true. But the operand `(/ 10 z)` with z = 0
           divides by zero and MUST trap. The mask op itself preserves the trap (`(& (/ 10 z) 0)` traps);
           the comparison-to-constant fold must not drop it either — the derived-range companion of the
           type-bound case.")
  (input  (do (def (main (: z Int64)) (if (< (& (/ 10 z) 15) 16) 1 0)) (export main)))
  (call   main (: 0 Int64))
  (trap   "divide by zero"))

(case "a tautology comparison still folds when its operand cannot trap"
  (doc    "The optimization is preserved for a TRAP-FREE operand: `(< (& z 15) 16)` masks the runtime
           parameter `z` to [0,15], so `< 16` is always true and the comparison folds to a constant (no
           runtime compare) — the mask of a bare parameter cannot trap. With z = 5 the `if` yields 1.
           Pins that trap-preservation does not over-conservatively suppress the fold on a total operand.")
  (input  (do (def (main (: z Int64)) (if (< (& z 15) 16) 1 0)) (export main)))
  (call   main (: 5 Int64))
  (output (: 1 Int64)))

; The SELF-COMPARISON fold (`x < x` → false, `x <= x`/`x >= x`/`x = x` → true — the ordering is fixed
; when both operands are the SAME value) is the sibling the type-bound cases above reference. It DISCARDS
; the operand, so — exactly like the tautology fold — it is sound to fold the RESULT to a constant only
; when the operand cannot trap; a possibly-trapping operand must still be evaluated. These pin that
; directly: a self-comparison of `(/ 10 z)` still div-by-zero traps at z = 0 (the fold must not drop the
; operand), while the `<=` form yields its constant `true` for a nonzero divisor (the fold is not
; over-suppressed). A RUNTIME divisor forces the trap to run time (a constant `(/ 10 0)` is a compile-time
; CDZ0304); the fold is a Core rewrite, so both backends preserve the trap identically.

(case "a self-comparison of a trapping operand still traps (less-than)"
  (doc    "`(< (/ 10 z) (/ 10 z))` with z = 0: `x < x` is always false, but the operand `(/ 10 z)`
           divides by zero and MUST trap — the self-comparison fold to `false` must not drop the
           operand's evaluation. The direct self-comparison companion of the tautology-comparison
           trap-preservation cases above (which only referenced this fold).")
  (input  (do (def (main (: z Int64)) (if (< (/ 10 z) (/ 10 z)) 1 0)) (export main)))
  (call   main (: 0 Int64))
  (trap   "divide by zero"))

(case "a self-comparison less-equal still traps on a zero divisor but folds true for a nonzero one"
  (doc    "`(<= (/ 10 z) (/ 10 z))` — `x <= x` is always true, yet at z = 0 the operand MUST still trap
           (the fold to `true` keeps the trapping operand's evaluation), and at z = 2 it yields the
           constant true → the `if` gives 1 (the fold is not over-suppressed on a nonzero divisor). Pins
           both faces of the self-comparison fold's trap-preservation for the `<=` operator.")
  (input  (do (def (main (: z Int64)) (if (<= (/ 10 z) (/ 10 z)) 1 0)) (export main)))
  (call   main (: 0 Int64))
  (trap   "divide by zero")
  (call   main (: 2 Int64))
  (output (: 1 Int64)))

; ── SELF-OPERAND arithmetic identities on a RUNTIME value: which fold, and which MUST NOT ────────────
; The two-of-a-kind operand cases `x ⊕ x` — where BOTH operands are the same runtime binding — have fixed
; results the compiler may fold: `x - x = 0` (always, even at Int64.min — subtraction of equals never
; overflows), `x ^ x = 0` (every bit cancels), `x & x = x` and `x | x = x` (idempotent). Each is a
; backend-independent value the emit computes correctly on both backends. But `x / x` is the CRITICAL
; NON-identity: it is NOT 1 in general, because at `x = 0` it is `0 / 0`, a DIVIDE-BY-ZERO TRAP — folding
; `x / x → 1` would ELIDE that defined trap (a miscompile). So the compiler must keep the division; these
; pin that `x - x`/`x ^ x`/`x & x`/`x | x` compute their identity value AND that `x / x` still traps at 0.

(case "self-operand subtraction and xor are zero, and-or are the operand, on a runtime value"
  (doc    "The self-operand identities that hold unconditionally over a runtime `x`: `(- x x)` = 0 (even at
           Int64.min — equals never overflow), `(^ x x)` = 0 (bits cancel), `(& x x)` = `(| x x)` = x
           (idempotent). Packed into `(tuple (- x x) (^ x x) (& x x) (| x x))`: x = 7 → (0, 0, 7, 7),
           x = Int64.min → (0, 0, min, min) (the boundary survives the keeping identities). Pins the
           value of each self-operand fold on both backends.")
  (input  (do (def (main (: x Int64)) (tuple (- x x) (^ x x) (& x x) (| x x))) (export main)))
  (call   main (: 7 Int64))
  (output (: (tuple 0 0 7 7) (Tuple Int64 Int64 Int64 Int64)))
  (call   main (: -9223372036854775808 Int64))
  (output (: (tuple 0 0 -9223372036854775808 -9223372036854775808) (Tuple Int64 Int64 Int64 Int64))))

(case "self-operand division is NOT folded to one — it still traps at zero"
  (doc    "The critical NON-identity: `(/ x x)` is NOT `1` in general, because at x = 0 it is `0 / 0`, a
           divide-by-zero trap. A compiler that folded `x / x → 1` would ELIDE that defined trap — a
           miscompile. So the division is kept: x = 7 → 1 (7/7), but x = 0 → TRAPS. Pins that the
           self-operand fold family stops at division, preserving the trap on both backends (a runtime x
           keeps the divisor out of the constant fold).")
  (input  (do (def (main (: x Int64)) (/ x x)) (export main)))
  (call   main (: 7 Int64))
  (output (: 1 Int64))
  (call   main (: 0 Int64))
  (trap   "divide by zero"))

; ── An ANNIHILATOR algebraic identity (x*0, x&0 → 0) must not DISCARD a trapping runtime operand ──────
; These are the annihilator companions of the tautology-comparison cases above, aimed at the algebraic
; SIMPLIFICATION the compiler applies at the Core (backend-independent) tier: `x * 0` and `x & 0` fold to
; the constant 0. Unlike a KEEPING identity (`x + 0 = x`, which returns the operand so its traps still
; fire), an annihilator DISCARDS the other operand — so it may be applied ONLY when that operand cannot
; trap. `(* (/ 10 z) 0)` and `(& (/ 10 z) 0)` with z = 0 each carry a div-by-zero in the discarded
; operand: the fold to 0 MUST NOT drop it — the trap is a defined outcome (core-semantics.md §Partial
; Operations Have A Defined Outcome), observed because the operand is EVALUATED before being annihilated.
; A runtime divisor `z` forces the trap to run time (a constant `(/ 10 0)` folds to a compile-time
; CDZ0304); the fold is a Core rewrite in `lower`, so BOTH backends must preserve the trap identically.
; The trap-FREE companion pins that a total operand IS annihilated to 0 — the optimization is not
; over-suppressed. (An operand the compiler PROVES traps and would-discard earns the non-error CDZ0305,
; but here the trap is a RUNTIME value the compiler cannot prove, so the operand is kept and evaluated.)

(case "the multiply-by-zero annihilator does not discard a trapping runtime operand"
  (doc    "`(* (/ 10 z) 0)` with z = 0: `x * 0` folds to the constant 0, but the discarded operand
           `(/ 10 z)` divides by zero and MUST trap — the annihilator may drop the operand's VALUE, not
           its evaluation. A runtime divisor keeps the div out of the constant fold, so the emitted code
           must evaluate `(/ 10 z)` (and trap) before annihilating. The `x * 0` companion of the
           tautology-comparison trap-preservation cases above; the trap-free case below pins the fold
           still fires when the operand is total.")
  (input  (do (def (main (: z Int64)) (* (/ 10 z) 0)) (export main)))
  (call   main (: 0 Int64))
  (trap   "divide by zero"))

(case "the bitwise-AND-with-zero annihilator does not discard a trapping runtime operand"
  (doc    "The bitwise companion: `(& (/ 10 z) 0)` with z = 0 folds to 0 (AND with all-zero bits), but the
           discarded operand `(/ 10 z)` divides by zero and MUST trap. Pins that the `& 0` annihilator,
           like `* 0`, preserves the discarded operand's evaluation — the mask op preserving the trap that
           the tautology case above only mentions in passing, pinned directly here.")
  (input  (do (def (main (: z Int64)) (& (/ 10 z) 0)) (export main)))
  (call   main (: 0 Int64))
  (trap   "divide by zero"))

(case "an annihilator identity folds to zero when its operand is trap-free"
  (doc    "The control: `(* z 0)` over a bare runtime parameter `z` folds to the constant 0 — the operand
           is a plain parameter that cannot trap, so the annihilator applies and no runtime multiply is
           emitted. With z = 7 the result is 0 (not 7). Pins that trap-preservation does not
           over-conservatively suppress the annihilator on a total operand — the dual of the trapping
           cases above.")
  (input  (do (def (main (: z Int64)) (* z 0)) (export main)))
  (call   main (: 7 Int64))
  (output (: 0 Int64)))

(case "modulo gives the remainder"
  (doc    "The compiler needs modulo for LEB128 encoding: extract 7-bit groups from an integer.")
  (input  (% 130 128))
  (output (: 2 Int64)))

; --- Modulo by -1 is always zero, even at Int64.min ----------------------------------------
; `x % -1` is 0 for every x: the remainder of dividing by ±1 is always 0. This holds at Int64.min
; too — `Int64.min % -1` = 0 — even though `Int64.min / -1` OVERFLOWS (the quotient +2^63 is out of
; range and traps, numeric-model.md #Overflow Is Defined). Modulo does not compute the quotient's
; value, so it has no overflow: the overflow check that (correctly) makes `/` trap must NOT be applied
; to `%`. The seed's RUNTIME path is correct (`i64.rem_s` yields 0), but its CONST-FOLD path over-
; eagerly reuses the division-overflow check and wrongly TRAPS on the constant `(% Int64.min -1)`.

(case "modulo by -1 is zero even at the minimum integer"
  (doc    "`(% -9223372036854775808 -1)` (Int64.min % -1) is 0 — every x % -1 is 0, and modulo does not
           overflow because it never forms the out-of-range quotient. It MUST yield 0, not trap. The
           seed's runtime path is correct (the companion below), but the compile-time constant fold
           wrongly applies the division-overflow check and traps. Contrast `(/ -9223372036854775808 -1)`,
           which genuinely overflows (quotient +2^63) and correctly traps (the other companion).")
  (input  (% -9223372036854775808 -1))
  (output (: 0 Int64)))

(case "a runtime modulo of the minimum integer by -1 is zero on every backend"
  (doc    "The RUNTIME companion of the constant case above — and the one that catches a backend that only
           the runtime path reaches. `(% a b)` with a = Int64.min, b = -1 must be 0: `x % -1` is 0 for
           every x, and modulo forms no quotient so it never overflows (numeric-model §Modulo by -1 is
           always zero). The constant `(% MIN -1)` folds to 0 at compile time on every backend, so it does
           NOT exercise a backend's runtime remainder emit; only RUNTIME operands reaching that emit do.
           The wasm backend's `i64.rem_s` yields 0; the Rust backend must too — a `%` guards ONLY the zero
           divisor (a wrapping remainder gives the correct 0 at MIN%-1), NOT the MIN/-1 overflow that only
           `/` has. Contrast the runtime `(/ a b)` at (MIN,-1) below, which genuinely overflows and traps.")
  (input  (do (def (main (: a Int64) (: b Int64)) (% a b)) (export main)))
  (call   main (: -9223372036854775808 Int64) (: -1 Int64))
  (output (: 0 Int64)))

(case "a runtime division of the minimum integer by -1 overflows and traps"
  (doc    "The RUNTIME companion pinning that `/` (unlike `%`) DOES trap at (MIN,-1): `(/ a b)` with
           a = Int64.min, b = -1 forms the out-of-range quotient +2^63 and MUST trap, on both backends.
           Together with the modulo case above this pins the `/`-vs-`%` split at the shared MIN/-1 input —
           the divergence a backend that treats them identically (e.g. one `checked_rem` for both) gets
           wrong: `%` must yield 0, `/` must trap.")
  (input  (do (def (main (: a Int64) (: b Int64)) (/ a b)) (export main)))
  (call   main (: -9223372036854775808 Int64) (: -1 Int64))
  (trap   "overflow"))

(case "division of the minimum integer by -1 overflows and traps"
  (doc    "`(/ -9223372036854775808 -1)` = +2^63, which is out of the Int64 range. The compiler can
           prove this overflow via constant folding and rejects at compile time (CDZ0304) rather than
           emitting a runtime trap. Contrast with modulo above, which does not overflow.")
  (input  (/ -9223372036854775808 -1))
  (error  CDZ0304))

(case "bitwise AND masks bits"
  (doc    "The compiler needs bitwise AND to extract low bits for LEB128 encoding: (& n 127)
           extracts the low 7 bits of n.")
  (input  (& 255 127))
  (output (: 127 Int64)))

(case "bitwise OR combines bits"
  (doc    "The compiler needs bitwise OR to set the continuation bit in LEB128: (| byte 128)
           sets bit 7.")
  (input  (| 42 128))
  (output (: 170 Int64)))

(case "bitwise XOR toggles bits"
  (doc    "`(^ 12 10)` = 6: XOR sets each result bit where exactly one operand's bit is set (1100 ^ 1010
           = 0110). The third bitwise operator alongside `&`/`|`, over Int64. XOR is its own inverse —
           `(^ (^ a k) k)` = a — which a compiler uses for cheap toggling / masking.")
  (input  (^ 12 10))
  (output (: 6 Int64)))

(case "bitwise XOR is its own inverse"
  (doc    "`(^ (^ 42 255) 255)` = 42: XOR-ing twice by the same key returns the original value. Pins the
           involution the single `(^ 12 10)` case cannot — a wrong opcode (say AND) would not round-trip.")
  (input  (^ (^ 42 255) 255))
  (output (: 42 Int64)))

; ── XOR-cancellation `(^ (^ v w) w) → v` on a RUNTIME operand — a backend-independent Core fold ───────
; The involution case above is a CONSTANT fold (both operands known). The compiler ALSO recognizes the
; cancellation on a RUNTIME value in `arith_identity` (lower.rs): `(^ (^ v w) w)` simplifies to `v`
; because `w ^ w == 0` and `v ^ 0 == v`, for a constant OR a runtime key `w`. That fold is a Core rewrite
; above the backend split, so it must yield `v` identically on BOTH backends — and a constant-scrutinee
; test never exercises it (a constant `v` folds through the const path instead). The runtime-key case also
; pins that the fold does NOT require `w` to be known: the two XORs by the SAME occurrence cancel whatever
; its value. (The discarded `w` is trap-free here — a bare parameter — so eliding the redundant pair is
; sound; a trapping `w` would be kept, the same is_trap_free discipline the annihilator cases above pin.)

(case "XOR cancellation returns the original runtime value (constant key)"
  (doc    "`(^ (^ v 255) 255)` over a runtime parameter `v` simplifies to `v` — the two XORs by the same
           constant key cancel. A runtime operand keeps this out of the constant fold, so it exercises the
           emitted code: v = 42 → 42, and a NEGATIVE v = -7 → -7 (the top bits toggle and toggle back, so
           the sign survives). The runtime companion of the constant involution case above; pins the fold
           is value-exact at run time on both backends, not just for a constant `v`.")
  (input  (do (def (main (: v Int64)) (^ (^ v 255) 255)) (export main)))
  (call   main (: 42 Int64))
  (output (: 42 Int64))
  (call   main (: -7 Int64))
  (output (: -7 Int64)))

(case "XOR cancellation returns the original value even when the key is a runtime value"
  (doc    "`(^ (^ v k) k)` over TWO runtime parameters simplifies to `v` — the cancellation does not need
           `k` known, only that both XORs use the SAME `k` (`k ^ k == 0`). v = 42, k = 99 → 42: the key is
           never constant-folded, so this pins that the Core fold recognizes the shared runtime occurrence
           and both backends drop the redundant XOR pair, recovering `v` exactly.")
  (input  (do (def (main (: v Int64) (: k Int64)) (^ (^ v k) k)) (export main)))
  (call   main (: 42 Int64) (: 99 Int64))
  (output (: 42 Int64)))

; ── The KEEPING identities (`x + 0`, `x << 0`, `x >> 0`) preserve a runtime operand exactly ──────────
; The additive/shift-by-zero identities RETURN the surviving operand (unlike the annihilators, which
; discard it), so a boundary-crossing runtime `x` must come back unchanged — including the extreme
; Int64.min, whose bit pattern a fold that re-derived the boundary would corrupt. `x + 0`, `x << 0`,
; `x >> 0` are all no-ops that keep `x` and its own traps; these pin them on an operand that CROSSES the
; boundary at run time (a constant `x` folds before reaching the identity), on both backends.

(case "the additive-zero identity preserves a boundary-crossing runtime operand"
  (doc    "`(+ x 0)` over a runtime parameter is the identity — adding zero never overflows and returns
           `x`. Called with Int64.min it comes back Int64.min exactly (a fold that dropped the sign or
           re-derived the boundary would corrupt it). Pins `x + 0 = x` on a genuinely runtime operand.")
  (input  (do (def (main (: x Int64)) (+ x 0)) (export main)))
  (call   main (: -9223372036854775808 Int64))
  (output (: -9223372036854775808 Int64)))

(case "shift-by-zero is a no-op on a runtime operand for both directions"
  (doc    "`(<< x 0)` and `(>> x 0)` are no-ops — a zero shift count keeps `x` (the count is the RIGHT
           operand; `arith_identity` elides the shift). `(f x)` = `(tuple (<< x 0) (>> x 0))` returns `x`
           twice: with x = -7 both are -7 (a right shift by 0 does not touch the sign bit either). Pins
           the shift-count-zero identities preserve a runtime operand, including a negative one, on both
           backends.")
  (input  (do (def (main (: x Int64)) (tuple (<< x 0) (>> x 0))) (export main)))
  (call   main (: -7 Int64))
  (output (: (tuple -7 -7) (Tuple Int64 Int64))))

(case "arithmetic right shift"
  (doc    "The compiler needs right shift for LEB128 encoding: (>> n 7) shifts n right by 7 bits,
           extracting the next group. Arithmetic shift preserves sign for signed LEB128.")
  (input  (>> 256 7))
  (output (: 2 Int64)))

; The right shift is ARITHMETIC (sign-preserving), which the doc above asserts but the `(>> 256 7)`
; case — a non-negative operand — cannot witness: a logical shift would give the same 2. The
; distinguishing case is a NEGATIVE operand: an arithmetic `(>> -256 7)` fills with the sign bit and
; yields -2 (floor division by 2^7), where a LOGICAL shift (wasm's i64.shr_u) would fill with zeros
; and yield a large positive number. The seed emits i64.shr_s (arithmetic); this pins that it must not
; be a logical shift. Signed LEB128 decoding depends on the sign-extending behavior.

(case "arithmetic right shift preserves the sign bit for a negative operand"
  (doc    "`(>> -256 7)` = -2: an arithmetic (sign-preserving) right shift fills the vacated high bits
           with the sign bit, so shifting the negative -256 right by 7 yields -2, not a large positive
           value. This is the case the non-negative `(>> 256 7)` cannot distinguish — a logical shift
           (i64.shr_u) would answer 144115188075855870. Pins that `>>` is the arithmetic shift the
           `arithmetic right shift` doc promises, which signed LEB128 relies on.")
  (input  (>> -256 7))
  (output (: -2 Int64)))

(case "arithmetic right shift of negative one is negative one"
  (doc    "`(>> -1 1)` = -1: -1 is all-ones in two's complement, and an arithmetic right shift
           sign-extends, so every shift of -1 stays -1. A logical shift would give a large positive
           value (2^63 - 1). The degenerate witness that `>>` sign-extends.")
  (input  (>> -1 1))
  (output (: -1 Int64)))

(case "left shift"
  (input  (<< 1 7))
  (output (: 128 Int64)))

; --- Shift bounds and overflow ----------------------------------------------------------
; A shift is not exempt from #Overflow Is Defined. A left shift is exact multiplication by a
; power of two, so an overflowing left shift MUST behave like an overflowing * (trap, per the
; checked-Int64 default), and a shift count outside the type's bit width has no defined value.
; wasm's i64.shl / i64.shr_s mask the shift count mod 64 and never trap, so a naive lowering
; leaks C-style undefined-shift behavior (shift-by-64 == shift-by-0, negative counts masked
; into 0..63, silent wrap on overflow); these cases pin that the seed must not.

(case "a left shift that overflows Int64 traps like multiplication"
  (doc    "Witnesses numeric-model.md #Overflow Is Defined for shifts: a left shift is exact
           multiplication by a power of two, so `(<< 4611686018427387904 1)` = 2^63, which overflows
           the checked Int64 range. The compiler can prove this overflow via constant folding and
           rejects at compile time (CDZ0304) rather than emitting a runtime trap.")
  (input  (<< 4611686018427387904 1))
  (error  CDZ0304))

(case "a left shift by the bit width or more traps rather than wrapping"
  (doc    "`1 << 64` is 2^64, which overflows Int64. A shift count equal to or beyond the type's
           bit width is out of range. The compiler can prove this via constant folding and rejects
           at compile time (CDZ0304) rather than emitting a runtime trap.")
  (input  (<< 1 64))
  (error  CDZ0304))

(case "a negative shift count traps rather than masking"
  (doc    "A negative shift count has no defined value. The compiler can prove the count is negative
           via constant folding and rejects at compile time (CDZ0304) rather than emitting a runtime
           trap or masking into a valid range.")
  (input  (<< 1 -1))
  (error  CDZ0304))

; The count = 63 boundary — the largest in-range shift count — is where a left shift's exact-2^count
; multiplication meets Int64's edge, and where a folder that builds the `2^count` factor with a signed
; `1 << 63` (= i64::MIN, a NEGATIVE 2^63) miscomputes in BOTH directions: `(<< 1 63)` folds to a
; wrong VALUE instead of overflowing, and `(<< -1 63)` overflows the checked multiply instead of
; yielding the representable Int64.min. The fold must compute `x * 2^count` at a width where 2^63 is
; exactly +2^63 (the seed uses i128) and then check the product fits Int64 — the same fit-check `*`
; makes — so the const path agrees with the runtime `<< 63` companions below.

(case "a constant left shift of 1 by 63 overflows and is rejected"
  (doc    "1 * 2^63 = +9223372036854775808, one past Int64.max — a provable overflow, rejected
           CDZ0304 exactly as `(<< 2 62)` (same value) is. The fold computes the 2^63 factor at a
           width where it is +2^63, not the signed `1 << 63` = i64::MIN that would fold this to a
           spurious -9223372036854775808 and disagree with the runtime path (which traps).")
  (input  (do (def (main) (<< 1 63)) (export main)))
  (error  CDZ0304))

(case "a constant left shift of -1 by 63 is exactly the minimum integer"
  (doc    "-1 * 2^63 = -9223372036854775808 = Int64.min, which FITS — the one in-range shl-by-63,
           over a negative operand. The fold must accept it (its exact product is representable), not
           reject it as it would if the 2^63 factor were the signed i64::MIN and `-1 * factor`
           overflowed the checker. The runtime companion below produces the same Int64.min.")
  (input  (do (def (main) (<< -1 63)) (export main)))
  (output (: -9223372036854775808 Int64)))

; The three shift cases above use CONSTANT operands (folded at compile time). The SAME shift with
; RUNTIME operands (function parameters) must trap identically — a shift is a shift regardless of
; whether its operands are compile-time-known. The overflow/out-of-range-count check must be emitted
; on the RUNTIME shift path (a guard before wasm's masking `i64.shl`/`i64.shr_s`), not only in the
; constant folder. These runtime companions pin that the two paths AGREE: the seed's const path traps
; (above) but its runtime path silently MASKS the count (mod 64) and WRAPS on overflow — a const-vs-
; runtime divergence and a wrong value for a runtime out-of-range shift.

(case "a runtime left shift by the bit width or more traps"
  (doc    "The 'runtime' companion of `(<< 1 64)`: with constant arguments supplied as parameters, the
           compiler β-reduces by substituting them, turning the body into `(<< 1 64)` — now a provable
           out-of-range shift. The compiler rejects at compile time (CDZ0304) via β-reduction.")
  (input  (do
            (def (sh a b) (<< a b))
            (def (main) (sh 1 64)) (export main)))
  (error  CDZ0304))

(case "a runtime overflowing left shift traps"
  (doc    "The 'runtime' companion of the overflowing left shift: with constant arguments supplied as
           parameters, the compiler β-reduces by substituting them, turning the body into
           `(<< 4611686018427387904 1)` — now a provable overflow. The compiler rejects at compile
           time (CDZ0304) via β-reduction.")
  (input  (do
            (def (sh a b) (<< a b))
            (def (main) (sh 4611686018427387904 1)) (export main)))
  (error  CDZ0304))

; The count = 63 boundary at run time — the shifted operand arrives as a parameter, so nothing folds
; and the emitted guarded `i64.shl` runs. These are the agreement anchors for the two constant `<< 63`
; cases above: `-1 << 63` produces exactly Int64.min (in range) and `1 << 63` overflows and traps, so
; the const fold of each must match (reject the overflow, produce the Int64.min) — not diverge.

(case "a runtime left shift by 63 fits for -1 and overflow-traps for 1"
  (doc    "The runtime companion of the constant `(<< _ 63)` cases: the operand arrives as a parameter,
           so the emitted guarded i64.shl runs rather than folding. Exercised at both boundary operands:
           with x = -1, -1 * 2^63 = Int64.min, which FITS, so the run produces -9223372036854775808 (the
           value the constant fold must also produce); with x = 1, 1 * 2^63 OVERFLOWS Int64 and the
           emitted round-trip overflow guard traps rather than wrapping — the outcome the constant fold
           of the same shift matches by rejecting (CDZ0304), not by producing a wrapped Int64.min.")
  (input  (do (def (main (: x Int64)) (<< x 63)) (export main)))
  (call   main (: -1 Int64))
  (output (: -9223372036854775808 Int64))
  (call   main (: 1 Int64))
  (trap   "integer overflow"))

; The out-of-range shift-count rule (#Overflow Is Defined: "a shift count outside the type's bit
; width has no defined value") applies to the RIGHT shift too, not only the left — the cases above
; all shift LEFT. wasm's i64.shr_s masks the count mod 64 exactly as i64.shl does, so `(>> n 64)`
; would silently become `(>> n 0)` and a negative count would mask into 0..63. A right shift never
; overflows (it only discards low bits), so the ONLY defined-outcome concern for `>>` is the count
; range — these pin that an out-of-range count traps for `>>` as it does for `<<`, at compile time
; and at run time.

(case "a right shift by the bit width or more traps rather than masking"
  (doc    "`(>> 256 64)` has a shift count equal to the bit width — out of range, no defined value.
           The compiler can prove this via constant folding and rejects at compile time (CDZ0304)
           rather than emitting a runtime trap. Pins the out-of-range-count rule for the RIGHT shift.")
  (input  (>> 256 64))
  (error  CDZ0304))

(case "a negative right-shift count traps rather than masking"
  (doc    "`(>> 256 -1)` has a negative count — no defined value. The compiler can prove the count is
           negative via constant folding and rejects at compile time (CDZ0304) rather than emitting a
           runtime trap or masking into a valid range.")
  (input  (>> 256 -1))
  (error  CDZ0304))

(case "a runtime right shift by the bit width or more traps"
  (doc    "The 'runtime' companion of `(>> 256 64)`: with constant arguments supplied as parameters, the
           compiler β-reduces by substituting them, turning the body into `(>> 256 64)` — now a provable
           out-of-range shift. The compiler rejects at compile time (CDZ0304) via β-reduction.")
  (input  (do
            (def (sh a b) (>> a b))
            (def (main) (sh 256 64)) (export main)))
  (error  CDZ0304))

; --- Checked and wrapping arithmetic: the two DEFINED non-trapping overflow outcomes ----------------
; The default `+`/`-`/`*` TRAP on overflow (the checked-Int64 default, above). numeric-model.md #Overflow
; Is Defined admits a defined VALUE outcome too — offered here as explicit Int64 methods that never trap:
;   `Int64.checked-add/sub/mul : (Int64, Int64) -> Option<Int64>` — the exact result when it fits,
;      `(None unit)` on overflow (the fallible companion of the trapping operator);
;   `Int64.wrapping-add/sub/mul : (Int64, Int64) -> Int64` — two's-complement wraparound modulo 2^64.
; Both are OPT-IN by name at the call site (an author who writes `+` still gets the trap), so overflow is
; never silent. The compiler reaches for `wrapping-*` where modular arithmetic is intended (hashing, LEB
; round-trips) and `checked-*` where it must branch on overflow without trapping.

(case "checked addition yields Some of the sum when it fits"
  (doc    "`(Int64.checked-add 20 22)` = `(Some 42)`: the result is in range, so checked addition
           returns it wrapped in `Some` (numeric-model.md #Overflow Is Defined — a defined value
           outcome). The fallible companion of `+`, which would compute the same 42 but trap on
           overflow rather than reporting it.")
  (input  (Int64.checked-add 20 22))
  (output (: (Some 42) (Option Int64))))

(case "checked addition yields None on overflow instead of trapping"
  (doc    "`(Int64.checked-add Int64.max 1)` = `(None unit)`: the sum overflows the Int64 range, so
           checked addition reports the overflow as `None` rather than trapping (contrast the `+`
           default at #overflow traps, `(+ Int64.max 1)` → trap). Pins the defined non-trapping overflow
           outcome numeric-model.md #Overflow Is Defined admits alongside the trap.")
  (input  (Int64.checked-add Int64.max 1))
  (output (: (None unit) (Option Int64))))

(case "checked multiplication reports overflow as None"
  (doc    "`(Int64.checked-mul Int64.max 2)` = `(None unit)`: the product is out of range. Pins checked
           multiplication's overflow detection (distinct from addition's — it is the `r/a != b` check),
           and that a=0 and the a=-1×MIN edge are handled: `(Int64.checked-mul 6 7)` below is `(Some 42)`.")
  (input  (Int64.checked-mul Int64.max 2))
  (output (: (None unit) (Option Int64))))

(case "checked multiplication yields Some when it fits"
  (doc    "`(Int64.checked-mul 6 7)` = `(Some 42)`: the in-range companion the overflow case above needs
           — a correct check must NOT report overflow here (a decline or a wrong `r/a` check would).")
  (input  (Int64.checked-mul 6 7))
  (output (: (Some 42) (Option Int64))))

(case "checked subtraction yields Some of the difference when it fits"
  (doc    "`(Int64.checked-sub 50 8)` = `(Some 42)`: the result is in range, so checked subtraction
           returns it wrapped in `Some` (numeric-model.md #Overflow Is Defined — a defined value
           outcome). The fallible companion of `-`, the third named overflow-fallible form the numeric
           model requires alongside checked add and checked mul.")
  (input  (Int64.checked-sub 50 8))
  (output (: (Some 42) (Option Int64))))

(case "checked subtraction yields None on underflow instead of trapping"
  (doc    "`(Int64.checked-sub Int64.min 1)` = `(None unit)`: the difference underflows the Int64 range,
           so checked subtraction reports the overflow as `None` rather than trapping (contrast the `-`
           default, `(- Int64.min 1)` → trap). Pins subtraction's defined non-trapping overflow outcome —
           the fallible form must exist for subtraction, not only addition and multiplication.")
  (input  (Int64.checked-sub Int64.min 1))
  (output (: (None unit) (Option Int64))))

(case "a checked result is consumed by matching its Option at run time"
  (doc    "The idiom a compiler writes: compute a checked sum of RUNTIME operands and branch on overflow
           without trapping. `(add-or a b d)` returns the sum when it fits, else the default `d`:
           `(add-or 20 22 -1)` = 42 (fits), `(add-or Int64.max 1 -1)` = -1 (overflowed → None → d).
           Their sum is 41. Pins checked arithmetic flowing as a runtime `Option<Int64>` matched by its
           `Some`/`None` arms — the fallible-arithmetic control flow, not just the folded constant.")
  (input  (do
            (def (add-or a b d)
              (match (Int64.checked-add a b)
                ((Some v) v)
                ((None _) d)))
            (def (main) (+ (add-or 20 22 -1) (add-or Int64.max 1 -1))) (export main)))
  (output (: 41 Int64)))

(case "wrapping addition wraps modulo two to the sixty-fourth on overflow"
  (doc    "`(Int64.wrapping-add Int64.max 1)` = Int64.min (-9223372036854775808): wrapping addition does
           NOT trap on overflow — it wraps in two's complement, so MAX + 1 becomes MIN (numeric-model.md
           #Overflow Is Defined, the modular value outcome). Contrast `(+ Int64.max 1)` → trap. This is
           the modular arithmetic a hash or a fixed-width round-trip wants.")
  (input  (Int64.wrapping-add Int64.max 1))
  (output (: -9223372036854775808 Int64)))

(case "wrapping addition of in-range operands is ordinary addition"
  (doc    "`(Int64.wrapping-add 20 22)` = 42: with no overflow, wrapping addition equals `+`. The
           in-range companion pinning that wrapping only differs from `+` at the overflow boundary.")
  (input  (Int64.wrapping-add 20 22))
  (output (: 42 Int64)))

(case "wrapping multiplication wraps rather than trapping"
  (doc    "`(Int64.wrapping-mul Int64.max 2)` = -2: MAX·2 = 2^64−2 ≡ −2 (mod 2^64), so wrapping
           multiplication returns −2 rather than trapping like `*`. Pins wrapping for the multiply, whose
           overflow is more than a single carry bit.")
  (input  (Int64.wrapping-mul Int64.max 2))
  (output (: -2 Int64)))

(case "wrapping subtraction wraps modulo two to the sixty-fourth on underflow"
  (doc    "`(Int64.wrapping-sub Int64.min 1)` = Int64.max (9223372036854775807): wrapping subtraction does
           NOT trap on underflow — it wraps in two's complement, so MIN − 1 becomes MAX (numeric-model.md
           #Overflow Is Defined, the modular value outcome). Contrast `(- Int64.min 1)` → trap. The
           subtraction companion of the wrapping-add wrap, the third named wrapping form the numeric model
           requires.")
  (input  (Int64.wrapping-sub Int64.min 1))
  (output (: 9223372036854775807 Int64)))

(case "wrapping subtraction of in-range operands is ordinary subtraction"
  (doc    "`(Int64.wrapping-sub 50 8)` = 42: with no underflow, wrapping subtraction equals `-`. The
           in-range companion pinning that wrapping only differs from `-` at the overflow boundary.")
  (input  (Int64.wrapping-sub 50 8))
  (output (: 42 Int64)))

(case "wrapping arithmetic algebraic identities hold at the range boundaries"
  (doc    "The wrapping ops fold their algebraic identities — `a +% 0 = a`, `a *% 1 = a`, `a *% 0 = 0` — and
           those folds must be exact at the range boundaries, not just for small values. `(Int64.wrapping-mul
           Int64.max 0)` is 0: multiplying by zero annihilates even Int64.max (and `*%` never traps, so there
           is no overflow to consider). Pins the annihilator identity at the boundary — a fold that wrongly
           kept `a` would yield max instead of 0.")
  (input  (Int64.wrapping-mul Int64.max 0))
  (output (: 0 Int64)))

(case "the wrapping additive and multiplicative identities preserve a boundary operand exactly"
  (doc    "The preserving companions: `(Int64.wrapping-add Int64.min 0)` = Int64.min and `(Int64.wrapping-mul
           Int64.min 1)` = Int64.min — adding zero and multiplying by one are the identity, preserving even
           the extreme Int64.min exactly (a fold that dropped the sign or re-derived the boundary would
           corrupt it). Confirms `a +% 0` and `a *% 1` are true identities across the whole range.")
  (input  (= (Int64.wrapping-add Int64.min 0) (Int64.wrapping-mul Int64.min 1)))
  (output (: true Bool)))

(case "wrapping arithmetic over runtime operands wraps at run time"
  (doc    "The runtime companion: `(w a b)` = `(Int64.wrapping-add a b)` over parameters wraps on the
           i64.add path (wasm's add wraps; no overflow guard), so `(w Int64.max 1)` = Int64.min — the
           same wrap the const fold gives. Pins that wrapping is emitted as the raw i64 op, not the
           checked/trapping one.")
  (input  (do
            (def (w a b) (Int64.wrapping-add a b))
            (def (main) (w Int64.max 1)) (export main)))
  (output (: -9223372036854775808 Int64)))

; The runtime case above passes a CONSTANT `Int64.max` into the wrapping op, so the boundary operand still
; folds before reaching the raw i64 op. These pin the wrapping ops on an operand that CROSSES the boundary
; as a `(call …)` argument — the value cannot be constant-folded, so the emitted raw machine op (no
; overflow guard) is exercised directly — across Int64, and the NARROW widths whose wraparound modulus is
; NOT 2^64 (a UInt8 wraps mod 256, an Int8 at ±128): a narrow wrap that reused the i64 op without masking
; to the type's width would give the un-wrapped value, so these witness the per-width wraparound.

(case "wrapping-add wraps a boundary-crossing runtime Int64 at the max boundary"
  (doc    "`(Int64.wrapping-add x 1)` with `x` a boundary parameter cannot fold; called with Int64.max it
           wraps to Int64.min on the raw i64.add path (wasm's add wraps, no overflow guard), and called with
           5 it is the ordinary 6. Pins wrapping-add on a genuinely runtime operand — the const-fold and the
           earlier constant-arg runtime case never cross the boundary with the max value itself.")
  (input  (do (def (main (: x Int64)) (Int64.wrapping-add x 1)) (export main)))
  (call   main (: 9223372036854775807 Int64)) (output (: -9223372036854775808 Int64))
  (call   main (: 5 Int64)) (output (: 6 Int64)))

(case "wrapping-mul wraps a boundary-crossing runtime Int64"
  (doc    "`(Int64.wrapping-mul x 2)` over a boundary parameter: 2^62 · 2 = 2^63 ≡ Int64.min (mod 2^64) →
           -9223372036854775808, and 3 · 2 = 6. Pins wrapping MULTIPLICATION on a runtime operand (the raw
           i64.mul, no overflow guard) — the multiply's overflow is more than a carry bit, so a runtime
           wrapping-mul is a distinct emit from wrapping-add.")
  (input  (do (def (main (: x Int64)) (Int64.wrapping-mul x 2)) (export main)))
  (call   main (: 4611686018427387904 Int64)) (output (: -9223372036854775808 Int64))
  (call   main (: 3 Int64)) (output (: 6 Int64)))

(case "wrapping-sub wraps a boundary-crossing runtime Int64 at the min boundary"
  (doc    "The runtime companion for the newly-added `wrapping-sub` (the const-fold cases above never
           cross the boundary with a runtime operand): `(Int64.wrapping-sub x 1)` with `x` a boundary
           parameter emits the raw `i64.sub` (wasm's sub wraps, no underflow guard). Called with Int64.min
           it wraps to Int64.max; called with 5 it is the ordinary 4. Pins wrapping SUBTRACTION on a
           genuinely runtime operand — the underflow at the min boundary wraps rather than trapping,
           distinct from `checked-sub` (which yields None / traps via expect), on both backends.")
  (input  (do (def (main (: x Int64)) (Int64.wrapping-sub x 1)) (export main)))
  (call   main (: -9223372036854775808 Int64)) (output (: 9223372036854775807 Int64))
  (call   main (: 5 Int64)) (output (: 4 Int64)))

(case "wrapping-sub of a runtime value from itself is zero at every input"
  (doc    "`(Int64.wrapping-sub x x)` = 0 for any runtime `x` — subtraction of equals is zero and never
           underflows, so wrapping and checked agree here. Confirmed at Int64.min (where a naive
           two-step compute could mis-handle the boundary) and 7. The self-operand companion for
           wrapping-sub, both backends (mirrors the checked `(- x x)` self-operand pin, but on the raw
           wrapping op which emits no guard).")
  (input  (do (def (main (: x Int64)) (Int64.wrapping-sub x x)) (export main)))
  (call   main (: -9223372036854775808 Int64)) (output (: 0 Int64))
  (call   main (: 7 Int64)) (output (: 0 Int64)))

(case "wrapping-add on a runtime UInt8 wraps modulo 256"
  (doc    "`(UInt8.wrapping-add x 1)` on a runtime UInt8: 255 + 1 wraps to 0 (mod 256), 10 + 1 = 11. The
           wraparound modulus is the TYPE's width (2^8), not 2^64 — a narrow wrap that reused the i64 op
           without masking to 8 bits would give 256, which is not even a UInt8. Pins per-width wraparound
           on the unsigned narrow type.")
  (input  (do (def (main (: x UInt8)) (UInt8.wrapping-add x 1)) (export main)))
  (call   main (: 255 UInt8)) (output (: 0 UInt8))
  (call   main (: 10 UInt8)) (output (: 11 UInt8)))

(case "wrapping-mul on a runtime UInt8 wraps modulo 256"
  (doc    "`(UInt8.wrapping-mul x 2)` on a runtime UInt8: 200 · 2 = 400 ≡ 144 (mod 256), 3 · 2 = 6. The
           multiply companion of the narrow wrapping-add — the product is masked to the UInt8 width, so a
           value exceeding 255 wraps into range rather than widening. Pins narrow wrapping multiplication.")
  (input  (do (def (main (: x UInt8)) (UInt8.wrapping-mul x 2)) (export main)))
  (call   main (: 200 UInt8)) (output (: 144 UInt8))
  (call   main (: 3 UInt8)) (output (: 6 UInt8)))

(case "wrapping-add on a runtime Int8 wraps at its signed boundary"
  (doc    "`(Int8.wrapping-add x 1)` on a SIGNED narrow type: 127 (Int8.max) + 1 wraps to -128 (Int8.min),
           -5 + 1 = -4. The signed narrow wraparound folds at the type's ±128 boundary, distinct from the
           unsigned mod-256 wrap above — a signed narrow wrap must sign-extend the wrapped low 8 bits, so
           127+1 is -128, not 128. Pins per-width wraparound on the SIGNED narrow type.")
  (input  (do (def (main (: x Int8)) (Int8.wrapping-add x 1)) (export main)))
  (call   main (: 127 Int8)) (output (: -128 Int8))
  (call   main (: -5 Int8)) (output (: -4 Int8)))

(case "wrapping-sub on a runtime UInt8 wraps modulo 256"
  (doc    "The subtraction companion of the narrow wrapping-add: `(UInt8.wrapping-sub x 1)` on a runtime
           UInt8 wraps at the LOW end — 0 - 1 = 255 (mod 256), 10 - 1 = 9. The wraparound modulus is the
           TYPE's width (2^8); a narrow wrap that reused the i64 op without masking to 8 bits would give
           -1 (not a UInt8). Pins per-width wraparound for the newly-added wrapping-sub on the unsigned
           narrow type.")
  (input  (do (def (main (: x UInt8)) (UInt8.wrapping-sub x 1)) (export main)))
  (call   main (: 0 UInt8)) (output (: 255 UInt8))
  (call   main (: 10 UInt8)) (output (: 9 UInt8)))

(case "wrapping-sub on a runtime Int8 wraps at its signed min boundary"
  (doc    "`(Int8.wrapping-sub x 1)` on a SIGNED narrow type: -128 (Int8.min) - 1 wraps to 127 (Int8.max),
           5 - 1 = 4. The signed narrow underflow wraps at the ±128 boundary and sign-extends the low 8
           bits (min-1's low byte is 0x7F = 127, not +128). The signed-narrow subtraction companion of the
           `wrapping-add on a runtime Int8` case, and the narrow face of the Int64 `wrapping-sub` min-wrap
           above.")
  (input  (do (def (main (: x Int8)) (Int8.wrapping-sub x 1)) (export main)))
  (call   main (: -128 Int8)) (output (: 127 Int8))
  (call   main (: 5 Int8)) (output (: 4 Int8)))

; The runtime narrow-wrap cases above wrap modulo the TYPE's width. Their CONSTANT-fold twins must give the
; IDENTICAL result — a wrapping op has a defined modular outcome regardless of whether its operands are
; constant. A narrow-width const-fold that reused the trapping/checked width-fit gate would REJECT CDZ0302
; ("integer literal does not fit its width") on a result exceeding the width instead of masking it — const
; and runtime would then diverge (the same expression rejects as a constant yet runs as runtime data). These
; pin the const-fold at both the unsigned mod-2^8 and the signed ±128 boundary, matching the runtime twins.

(case "a narrow-width wrapping-mul constant folds by wrapping, not by rejecting"
  (doc    "`(UInt8.wrapping-mul 20 20)` = 400, which exceeds UInt8. A wrapping op's outcome is the value
           MODULO the type width (400 mod 256 = 144), so the const-fold must MASK the result to 8 bits — not
           route it through the checked-op width-fit gate and reject CDZ0302. This is the constant twin of
           `wrapping-mul on a runtime UInt8` above (200·2 → 144): const and runtime must agree.")
  (input  (do (def (main) ((. UInt8 wrapping-mul) 20 20)) (export main)))
  (output (: 144 UInt8)))

(case "a signed narrow-width wrapping-add constant folds by wrapping to the negative outcome"
  (doc    "`(Int8.wrapping-add 100 100)` = 200, which exceeds Int8.max; the wrap sign-extends the low 8 bits,
           so 200 mod 2^8 with the sign bit set is -56. The signed companion of the const-fold above — a
           wrapping fold that width-fit-rejected (CDZ0302) or dropped the sign would diverge from the runtime
           signed narrow wrap. Pins the const-fold's signed masking at the ±128 boundary.")
  (input  (do (def (main) ((. Int8 wrapping-add) 100 100)) (export main)))
  (output (: -56 Int8)))

; ── The width-TRUNCATION conversion `(UInt N).wrap` / `(Int N).wrap` on a RUNTIME value ──────────────
; `.wrap` is the TOTAL width conversion (distinct from the `wrapping-add`/`-mul` ARITHMETIC above): it
; truncates its operand to the low N bits of the TARGET type's width (`UInt8.wrap` IS byte truncation —
; there is no `Int.to-byte`; the width comes from the type). It lowers to a `Core::Convert{op: Wrap}`,
; whose TARGET width is the node's OWN solved type. A constant operand folds; a RUNTIME operand emits the
; machine truncate + (for a signed target) sign-extend. These pin the CONVERSION's observed result at the
; boundary directly — the existing uses feed `.wrap` into a `bin`/`Bytes` construction and check
; downstream, but none pins the wrap's OWN value on a runtime operand across the boundary, both backends.

(case "uint8 wrap truncates a wide runtime value to the low 8 bits"
  (doc    "`(UInt8.wrap n)` over a runtime Int64 keeps only the low 8 bits (mod 256): 300 → 44, 256 → 0,
           and an in-range 200 → 200 (unchanged). A runtime operand exercises the emitted machine truncate
           (a constant would fold), and the result crosses the boundary as a UInt8. Pins the byte-
           truncation conversion's own value — `UInt8.wrap` IS byte truncation (no `Int.to-byte`), the
           width read off the UInt8 target type — on both backends.")
  (input  (do (def (main (: n Int64)) (UInt8.wrap n)) (export main)))
  (call   main (: 300 Int64))
  (output (: 44 UInt8))
  (call   main (: 256 Int64))
  (output (: 0 UInt8))
  (call   main (: 200 Int64))
  (output (: 200 UInt8)))

(case "int8 wrap truncates and sign-extends the low 8 bits of a runtime value"
  (doc    "The SIGNED companion: `(Int8.wrap n)` truncates to the low 8 bits AND sign-extends them to the
           Int8 range — 200's low byte (0xC8) has the sign bit set, so it reads as -56; 127 stays 127.
           Distinct from `UInt8.wrap` (which zero-extends to 0..255): the same low 8 bits become a NEGATIVE
           value under the signed target. Pins that the wrap conversion's sign-extension follows the TARGET
           type's signedness, on both backends.")
  (input  (do (def (main (: n Int64)) (Int8.wrap n)) (export main)))
  (call   main (: 200 Int64))
  (output (: -56 Int8))
  (call   main (: 127 Int64))
  (output (: 127 Int8)))

(case "uint16 wrap truncates a wide runtime value to the low 16 bits"
  (doc    "The wider-narrow face: `(UInt16.wrap n)` keeps the low 16 bits (mod 65536): 70000 → 4464
           (70000 - 65536). Pins that the truncation width is the TARGET type's (16 here, not 8 or 64), so
           the conversion is not hardwired to a byte — the width is read off the solved type, on both
           backends.")
  (input  (do (def (main (: n Int64)) (UInt16.wrap n)) (export main)))
  (call   main (: 70000 Int64))
  (output (: 4464 UInt16)))

(case "greater-than comparison"
  (doc    "The compiler uses > for bounds checking and conditional logic.")
  (input  (> 5 3))
  (output (: true Bool)))

(case "less-than-or-equal"
  (input  (<= 3 3))
  (output (: true Bool)))

(case "greater-than-or-equal"
  (input  (>= 4 3))
  (output (: true Bool)))

; The comparison cases above use small, unequal, positive operands. Two boundaries they cannot witness:
; (1) a STRICT comparison of EQUAL operands — `(< 5 5)` is false, `(> 5 5)` is false — distinguishing
; strict `<`/`>` from the inclusive `<=`/`>=` (a lowering that confused the two would flip these); and
; (2) SIGNED comparison, which the ordering must be (core-semantics.md #Ordering Where Offered Is Total,
; over Int64's signed values). A NEGATIVE operand and, most sharply, the Int64 EXTREMES expose an
; unsigned-comparison miscompile: `(< Int64.min Int64.max)` MUST be true, but Int64.min's two's-complement
; bit pattern is the LARGEST unsigned value, so a naive `i64.lt_u` lowering answers false. These pin the
; ordering as signed and strict-vs-inclusive as distinct.

(case "a strict less-than of equal operands is false"
  (doc    "`(< 5 5)` is false — a strict `<` does not hold between equal values (contrast `(<= 3 3)`
           above, which is true). Pins that `<` is strict, distinct from `<=`, at the equal-operand
           boundary a small-unequal-operand case cannot reach.")
  (input  (< 5 5))
  (output (: false Bool)))

(case "a strict greater-than of equal operands is false"
  (doc    "`(> 5 5)` is false — the strict-`>` companion of the case above. Pins `>` strict vs `>=`.")
  (input  (> 5 5))
  (output (: false Bool)))

(case "less-than compares negative operands by signed order"
  (doc    "`(< -3 -1)` is true: -3 is less than -1 under the signed integer order (core-semantics.md
           #Ordering Where Offered Is Total). Pins that the comparison is signed for negative operands,
           not a magnitude or unsigned comparison (which would rank -3 above -1).")
  (input  (< -3 -1))
  (output (: true Bool)))

(case "less-than at the integer extremes is signed, not unsigned"
  (doc    "`(< -9223372036854775808 9223372036854775807)` (Int64.min < Int64.max) is true — the sharpest
           signed-vs-unsigned discriminator. Int64.min's two's-complement bit pattern
           (0x8000000000000000) is the LARGEST value read as unsigned, so a lowering that emits an
           unsigned compare (i64.lt_u) would wrongly answer false. Pins that the ordering is SIGNED
           (core-semantics.md #Ordering Where Offered Is Total over Int64's signed values).")
  (input  (< -9223372036854775808 9223372036854775807))
  (output (: true Bool)))

(case "greater-than at the integer extremes is signed"
  (doc    "The `>` companion: `(> 9223372036854775807 -9223372036854775808)` (Int64.max > Int64.min) is
           true under signed comparison. Confirms both strict ordering operators use the signed compare
           at the extremes, not only `<`.")
  (input  (> 9223372036854775807 -9223372036854775808))
  (output (: true Bool)))

; The relational cases above are all CONSTANT (they fold), and the runtime relational cases elsewhere use
; the STRICT `<`/`>`. The INCLUSIVE `<=`/`>=` have a distinct emit (`i64.le_s`/`i64.ge_s`) and differ from
; strict precisely at the EQUAL-operand boundary. These pin runtime `<=`/`>=` over boundary parameters (so
; the comparison runs as an emitted instruction, not a fold): the equal-operand case (where inclusive is
; true but strict would be false) and the signed extremes (where an unsigned `le_u` would answer wrong).

(case "a runtime less-than-or-equal holds at the equal-operand boundary"
  (doc    "`(<= a b)` over boundary parameters: true when `a < b` (3 ≤ 5) AND when `a = b` (5 ≤ 5) — the
           equal-operand case is where the INCLUSIVE `<=` differs from the strict `<` (which is false at
           `5 < 5`). a=3,b=5 → 1; a=5,b=5 → 1; a=9,b=5 → 0. Pins that the runtime `<=` emits the inclusive
           signed compare (`i64.le_s`), distinct from the strict `<` a runtime case elsewhere pins.")
  (input  (do (def (main (: a Int64) (: b Int64)) (if (<= a b) 1 0)) (export main)))
  (call   main (: 3 Int64) (: 5 Int64)) (output (: 1 Int64))
  (call   main (: 5 Int64) (: 5 Int64)) (output (: 1 Int64))
  (call   main (: 9 Int64) (: 5 Int64)) (output (: 0 Int64)))

(case "a runtime greater-than-or-equal holds at the equal-operand boundary"
  (doc    "The `>=` companion: `(>= a b)` is true when `a > b` (9 ≥ 5) AND when `a = b` (5 ≥ 5), false when
           `a < b` (3 ≥ 5). The equal-operand case distinguishes inclusive `>=` from strict `>`. Pins the
           runtime inclusive `i64.ge_s` emit.")
  (input  (do (def (main (: a Int64) (: b Int64)) (if (>= a b) 1 0)) (export main)))
  (call   main (: 9 Int64) (: 5 Int64)) (output (: 1 Int64))
  (call   main (: 5 Int64) (: 5 Int64)) (output (: 1 Int64))
  (call   main (: 3 Int64) (: 5 Int64)) (output (: 0 Int64)))

(case "a runtime less-than-or-equal is signed at the integer extremes"
  (doc    "The signed-vs-unsigned discriminator for the inclusive form: `(<= Int64.min Int64.max)` over
           runtime parameters is true under signed order — but Int64.min's two's-complement pattern is the
           LARGEST unsigned value, so an `i64.le_u` lowering would wrongly answer false. Pins that the
           runtime `<=` is SIGNED at the extremes, the inclusive companion of the strict signed-extremes
           cases above.")
  (input  (do (def (main (: a Int64) (: b Int64)) (if (<= a b) 1 0)) (export main)))
  (call   main (: -9223372036854775808 Int64) (: 9223372036854775807 Int64)) (output (: 1 Int64)))

(case "integer to byte (truncate to 0-255)"
  (doc    "The compiler converts integers to single bytes for wasm encoding by TRUNCATING to a UInt8 with
           `UInt8.wrap` — the width-indexed truncating conversion (numeric-model.md #Truncation Is
           Explicit And Total): it keeps the low 8 bits and reinterprets at the target width, never traps,
           and its result type is `UInt8` (the byte type) — the width read from the type, no dedicated
           byte-conversion op needed. `(UInt8.wrap 200)` = 200.")
  (input  ((. UInt8 wrap) 200))
  (output (: 200 UInt8)))

(case "integer to byte wraps on overflow"
  (doc    "Values > 255 wrap to the low 8 bits: `UInt8.wrap` keeps the low 8 bits, so 256 = 0x100 → 0.")
  (input  ((. UInt8 wrap) 256))
  (output (: 0 UInt8)))

(case "negative integer to byte uses two's complement"
  (doc    "`UInt8.wrap` reinterprets the low 8 bits at the target width, so -1 (…11111111) → 255.")
  (input  ((. UInt8 wrap) -1))
  (output (: 255 UInt8)))

; --- The bitwise operators satisfy their defining Boolean-algebra laws --------------------------
; Before composing `&`/`|`/`^` into LEB128 below, pin the identities each satisfies bit-for-bit over an
; Int64's 64 bits (numeric-model.md #Overflow Is Defined names the exact bit operations). AND with all-
; zeros is 0 and with all-ones (`-1`, every bit set in two's complement) is the operand; OR is the dual
; (identity at 0, annihilator at -1); XOR is identity at 0 and self-INVERSE (`x ^ x` clears every bit to
; 0). These are the bit-level analogue of the set-algebra laws (a set is a bit per element), and they
; witness that `-1` is the all-ones mask a byte-masking encoder relies on. The composition cases below
; use these ops in the encoder's arithmetic; these pin what each operator MEANS in isolation first.

(case "bitwise AND with zero is zero (annihilator)"
  (doc    "`(& 42 0)` = 0: AND with all-zero bits clears every bit, so 0 is the annihilator of `&`. Pins
           the zero law of bitwise AND — every bit ANDed with 0 is 0, whatever the other operand.")
  (input  (& 42 0))
  (output (: 0 Int64)))

(case "bitwise AND with negative one is the operand (identity)"
  (doc    "`(& 42 -1)` = 42: `-1` is all-ones in two's complement, so ANDing with it keeps every bit — the
           identity of `&`. Pins that `-1` is the all-ones mask (the complement of the `& _ 0` annihilator),
           the masking identity a byte extractor relies on.")
  (input  (& 42 -1))
  (output (: 42 Int64)))

(case "bitwise OR with zero is the operand (identity)"
  (doc    "`(| 42 0)` = 42: OR with all-zero bits sets no additional bit, so 0 is the identity of `|` — the
           dual of AND's zero-annihilator. Pins the zero law of bitwise OR.")
  (input  (| 42 0))
  (output (: 42 Int64)))

(case "bitwise OR with negative one is all ones (annihilator)"
  (doc    "`(| 42 -1)` = -1: ORing with all-ones sets every bit, so `-1` is the annihilator of `|` — the
           dual of AND's all-ones identity. Pins that OR saturates to -1 against the all-ones mask.")
  (input  (| 42 -1))
  (output (: -1 Int64)))

(case "bitwise XOR with zero is the operand (identity)"
  (doc    "`(^ 42 0)` = 42: XOR with all-zero bits flips nothing, so 0 is the identity of `^`. Pins the
           zero law of bitwise XOR (the third operator's identity, beside AND's -1 and OR's 0).")
  (input  (^ 42 0))
  (output (: 42 Int64)))

(case "bitwise XOR of a value with itself is zero (self-inverse)"
  (doc    "`(^ 42 42)` = 0: XOR of equal bits is 0 at every position, so `x ^ x` clears the whole word —
           XOR is its own inverse. Pins the self-inverse law (distinct from AND/OR which are idempotent,
           `x & x = x` / `x | x = x`); the property a XOR-swap or a running-XOR checksum relies on.")
  (input  (^ 42 42))
  (output (: 0 Int64)))

(case "bitwise XOR with negative one is the bitwise complement"
  (doc    "`(^ 5 -1)` = -6: XOR with all-ones flips every bit, which IS the bitwise complement — `~x` is
           `x ^ -1`. For 5 (…0101) the complement is …1010 = -6 in two's complement. Pins the complement
           idiom (the language spells `~x` as `(^ x -1)` rather than a dedicated operator), so a program
           needing a bit-flip has the operation.")
  (input  (^ 5 -1))
  (output (: -6 Int64)))

; ── RUNTIME bitwise SIMPLIFICATIONS in arith_identity (lower.rs) — Core rewrites both backends inherit ─
; Beyond the zero/identity laws, `arith_identity` recognizes several structural bitwise simplifications on
; RUNTIME operands (a constant folds through the const path, so these emitted-code rewrites need a runtime
; witness). Each is value-exact on both backends, and each that DISCARDS an operand is guarded by
; `is_trap_free` so a defined trap in the discarded operand is still raised:
;  - FULL-MASK elision: `(& b M)` → b when M covers all of unsigned b's value bits (a redundant mask).
;  - OR-SATURATION: `(| b M)` → M when M covers all of b's bits (b adds nothing; DISCARDS b).
;  - OR-THEN-MASK absorption: `(& (| v C1) C2)` → C2 when C2 ⊆ C1 (the inner OR sets every bit the outer
;    mask keeps, so the result is exactly C2, independent of v; DISCARDS v — hence the trap-free guard).
;  - NESTED-OR collapse: `(| (| x C) C)` → `(| x C)` (ORing by the same constant twice is once).

(case "a redundant full-width mask on an unsigned runtime value is elided"
  (doc    "`(& b 255)` where `b : UInt8` lives in [0,255] — the mask 255 covers every bit b can set, so
           the AND is a no-op and `arith_identity` returns b. A runtime b exercises the emitted code:
           b = 200 → 200. Pins the full-mask elision keeps the operand (an unsigned value whose width the
           mask covers), not a constant, on both backends.")
  (input  (do (def (main (: b UInt8)) (& b 255)) (export main)))
  (call   main (: 200 UInt8))
  (output (: 200 UInt8)))

(case "OR-saturation to a full-width mask on an unsigned runtime value"
  (doc    "The OR dual: `(| b 255)` where `b : UInt8` saturates to 255 — 255 already has every bit b
           could set, so the OR adds nothing and the result is exactly the mask (DISCARDING b). b = 200 →
           255. Pins OR-saturation to the full mask on a runtime operand, both backends.")
  (input  (do (def (main (: b UInt8)) (| b 255)) (export main)))
  (call   main (: 200 UInt8))
  (output (: 255 UInt8)))

(case "OR-then-mask absorption folds to the mask constant on a runtime operand"
  (doc    "`(& (| v 15) 15)` → 15 for ANY v: the inner `(| v 15)` sets the low 4 bits, the outer `& 15`
           keeps exactly those 4 bits — all now 1 — so the result is the constant 15 independent of v.
           A runtime v (which the fold DISCARDS) exercises the rewrite: v = 42 → 15, v = 0 → 15. Pins the
           OR-then-mask absorption (C2 ⊆ C1) on a runtime operand, both backends.")
  (input  (do (def (main (: v Int64)) (& (| v 15) 15)) (export main)))
  (call   main (: 42 Int64))
  (output (: 15 Int64))
  (call   main (: 0 Int64))
  (output (: 15 Int64)))

(case "OR-then-mask absorption does not discard a trapping runtime operand"
  (doc    "The trap-preservation face: `(& (| (/ 10 z) 15) 15)` still folds to 15 for the VALUE, but the
           discarded operand `(/ 10 z)` with z = 0 divides by zero and MUST trap — the absorption may drop
           v's value, not its evaluation (the `is_trap_free` guard on the fold). A runtime divisor keeps
           the div out of the const fold. The OR-then-mask companion of the annihilator trap-preservation
           cases, both backends.")
  (input  (do (def (main (: z Int64)) (& (| (/ 10 z) 15) 15)) (export main)))
  (call   main (: 0 Int64))
  (trap   "divide by zero"))

(case "a nested OR by the same constant collapses to a single OR"
  (doc    "`(| (| x 8) 8)` → `(| x 8)`: ORing by the same constant twice is idempotent, so the inner and
           outer ORs collapse to one. Unlike the absorption above this KEEPS x (x's bits still flow), so a
           runtime x threads through: x = 1 → 9 (0b0001 | 0b1000). Pins the nested-OR collapse preserves
           the value on a runtime operand, both backends.")
  (input  (do (def (main (: x Int64)) (| (| x 8) 8)) (export main)))
  (call   main (: 1 Int64))
  (output (: 9 Int64)))

; --- The bitwise/shift/truncation primitives COMPOSE into the LEB128 encoding step ----------
; The cases above exercise `&`, `|`, `>>`, and `UInt8.wrap` INDIVIDUALLY on constant operands. The
; compiler's actual use is to COMPOSE them: one LEB128 byte is `(| (& n 127) 128)` when a continuation
; byte follows (the low 7 bits of n, with bit 7 set), or `(& n 127)` for the final byte, and the next
; group is `(>> n 7)`. Composing the operators exercises their interaction — each intermediate is an
; Int64 fed to the next, in evaluation order — which an isolated single-operator case cannot witness. A
; miscompile in operator interaction (a wrong intermediate type, a mis-sequenced fold) would surface
; here where it hides in the isolated cases. These pin the compiler's own encoding arithmetic
; (numeric-model.md #Overflow Is Defined for the exact bit operations; compiler-pipeline.md relies on
; LEB128 for wasm section sizes).

(case "a LEB128 non-final byte composes mask, continuation bit, and truncation"
  (doc    "One LEB128 continuation byte of 300: `(& 300 127)` = 44 (low 7 bits), `(| 44 128)` = 172 (set
           bit 7), `UInt8.wrap` truncates it to a byte (already in 0..=255, so it is left). The composed
           `(UInt8.wrap (| (& 300 127) 128))` = 172 — the exact byte a LEB128 encoder emits for the first
           group of 300. Pins that the three operators compose to the encoder's non-final byte, not just
           that each works alone. The Int64 intermediate `(| (& 300 127) 128)` truncates to a UInt8.")
  (input  ((. UInt8 wrap) (| (& 300 127) 128)))
  (output (: 172 UInt8)))

(case "a LEB128 final byte is the shifted remainder masked to seven bits"
  (doc    "The final group of 300: `(>> 300 7)` = 2 (the remaining bits after the low 7), and
           `(& 2 127)` = 2 (final byte, continuation bit clear). The composed `(& (>> 300 7) 127)` = 2 —
           the encoder's terminating byte. Together with the case above, `300` encodes as the two LEB128
           bytes 172, 2. Pins the shift-then-mask composition for the final group.")
  (input  (& (>> 300 7) 127))
  (output (: 2 Int64)))

(case "the LEB128 byte composition runs on a runtime operand"
  (doc    "The composition above on a RUNTIME operand, not a constant: `(leb-byte n)` = `(UInt8.wrap (|
           (& n 127) 128))` with `n` a function parameter, so the mask, continuation-bit OR, and the
           truncation are EMITTED (not const-folded). `(leb-byte 300)` = 172, the same non-final byte the
           constant case produces — but reached through the runtime `i64.and`/`i64.or` the encoder actually
           executes when it encodes a value computed at run time (a section length, an operand). Pins
           that runtime bitwise `&`/`|` (and their composition) are emitted and agree with the const
           fold — a self-hosted LEB128 encoder works on the runtime values it is fed, not only on
           literals. The const cases above fold and so cannot witness the emitted bitwise path; this
           one, taking `n` through a parameter, does.")
  (input  (do
            (def (leb-byte n) ((. UInt8 wrap) (| (& n 127) 128)))
            (def (main)       (leb-byte 300)) (export main)))
  (output (: 172 UInt8)))

(case "extracting a high byte composes shift and mask"
  (doc    "`(& (>> 65535 8) 255)` shifts 0xFFFF right by 8 (yielding 0xFF = 255) then masks the low 8
           bits (255) — the byte-extraction the compiler uses to lay out a multi-byte little-endian
           field. Pins that shift and mask compose to select an arbitrary byte, exercising their
           interaction on a value wider than one byte.")
  (input  (& (>> 65535 8) 255))
  (output (: 255 Int64)))

; --- Runtime operands to the full operator set: emitted instructions, not the constant fold ---------
; Every arithmetic case above uses CONSTANT operands, so the compiler folds it at build time — a real
; strength, but it means the EMITTED runtime instruction (the machine op plus its overflow / count /
; sign guards) is never exercised. A value that arrives at RUN TIME (an argument to the exported entry)
; cannot be folded, so the operator is emitted as real instructions. These `(call <export> <arg>…)`
; cases run each operator over runtime Int64 operands and pin that the emitted path AGREES with the
; folded constant cases above (`/ % & | ^ << >>`, and the ordering comparisons). A self-hosted compiler
; runs exactly this — its LEB128/section arithmetic operates on section sizes and operands computed at
; run time, not on literals. The seed realizes runtime Int64 operators, so it runs these.

(case "a runtime division truncates toward zero"
  (doc    "`(def (main (: a Int64) (: b Int64)) (/ a b))` called with (-7, 2). The division cannot fold
           (both operands are runtime), so it is emitted as `i64.div_s`, which truncates toward zero —
           -7/2 = -3, matching the folded `(/ -7 2)` case. Pins the emitted signed-division path.")
  (input  (do (def (main (: a Int64) (: b Int64)) (/ a b)) (export main)))
  (call   main (: -7 Int64) (: 2 Int64))
  (output (: -3 Int64)))

(case "a runtime remainder takes the dividend's sign"
  (doc    "`(% a b)` over runtime operands emits `i64.rem_s`; `(-7, 2)` = -1, the remainder taking the
           dividend's sign, matching the folded `(% -7 2)`. Pins the emitted remainder path.")
  (input  (do (def (main (: a Int64) (: b Int64)) (% a b)) (export main)))
  (call   main (: -7 Int64) (: 2 Int64))
  (output (: -1 Int64)))

(case "a runtime bitwise AND masks bits"
  (doc    "`(& a b)` over runtime operands emits `i64.and`; `(255, 127)` = 127 — the low-7-bit mask a
           LEB128 encoder applies to a value computed at run time. Pins the emitted bitwise-AND path.")
  (input  (do (def (main (: a Int64) (: b Int64)) (& a b)) (export main)))
  (call   main (: 255 Int64) (: 127 Int64))
  (output (: 127 Int64)))

(case "a guard-elided masked add computes inline over runtime operands"
  (doc    "`(+ (& a 7) (& b 7))` over runtime operands: both masked values live in [0,7], so the sum lives
           in [0,14] and provably fits Int64 — the compiler elides the overflow guard. With no guard to
           re-read them, the two masked operands are emitted straight onto the stack (no scratch slots) and
           added. Value parity is the observable proof the elision keeps the exact result: `(255, 250)` =
           (255&7) + (250&7) = 7 + 2 = 9, `(8, 8)` = 0 + 0 = 0. Pins the guard-elided masked-add path — the
           common LEB/bit-packing idiom where the range analysis proves no overflow.")
  (input  (do (def (main (: a Int64) (: b Int64)) (+ (& a 7) (& b 7))) (export main)))
  (call   main (: 255 Int64) (: 250 Int64)) (output (: 9 Int64))
  (call   main (: 8 Int64) (: 8 Int64)) (output (: 0 Int64)))

(case "a runtime bitwise OR combines bits"
  (doc    "`(| a b)` emits `i64.or`; `(42, 128)` = 170 — setting the LEB128 continuation bit on a runtime
           byte. Pins the emitted bitwise-OR path.")
  (input  (do (def (main (: a Int64) (: b Int64)) (| a b)) (export main)))
  (call   main (: 42 Int64) (: 128 Int64))
  (output (: 170 Int64)))

(case "a runtime bitwise XOR toggles bits"
  (doc    "`(^ a b)` emits `i64.xor`; `(12, 10)` = 6. Pins the emitted bitwise-XOR path, the third
           bitwise operator alongside `&`/`|`.")
  (input  (do (def (main (: a Int64) (: b Int64)) (^ a b)) (export main)))
  (call   main (: 12 Int64) (: 10 Int64))
  (output (: 6 Int64)))

(case "a runtime left shift multiplies by a power of two"
  (doc    "`(<< a b)` over runtime operands emits the guarded `i64.shl` (count checked against the width,
           result checked for overflow); `(1, 7)` = 128, matching the folded `(<< 1 7)`. Pins the emitted
           left-shift path — the shift a LEB encoder runs on a runtime group index.")
  (input  (do (def (main (: a Int64) (: b Int64)) (<< a b)) (export main)))
  (call   main (: 1 Int64) (: 7 Int64))
  (output (: 128 Int64)))

(case "a strength-reduced multiply nested as an operand computes in place"
  (doc    "`(+ (* a 2) 1)` over a runtime `a`: the `(* a 2)` strength-reduces to `a << 1` and is the LHS
           OPERAND of the enclosing `+`, so the shift writes the add's operand slot DIRECTLY (its result
           store IS the add's operand store — no intermediate copy), then `+ 1`. `(3)` = 3·2 + 1 = 7, `(0)`
           = 1. Pins that a nested strength-reduced multiply as an operand keeps the checked semantics
           (value + overflow) while the shift result lands straight in the consuming op's slot.")
  (input  (do (def (main (: a Int64)) (+ (* a 2) 1)) (export main)))
  (call   main (: 3 Int64)) (output (: 7 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64)))

(case "a doubly-nested strength-reduced multiply chains shift into shift"
  (doc    "`(* (* x 2) 4)` over a runtime `x`: the inner `(* x 2)` → `x << 1` writes the outer shift's
           operand slot directly, and the outer `(* … 4)` → `… << 2` writes its own result — two shifts
           chained with no copy between them. `(3)` = 3·8 = 24, `(1)` = 8. Pins the nested-operand shift
           threading through a second strength reduction.")
  (input  (do (def (main (: x Int64)) (* (* x 2) 4)) (export main)))
  (call   main (: 3 Int64)) (output (: 24 Int64))
  (call   main (: 1 Int64)) (output (: 8 Int64)))

; ── STRAIGHT-LINE checked arithmetic is NOT reassociated — the inner overflow trap is preserved ──────
; A nested checked `(- (+ x 1) 1)` is value-equal to `x` for a non-overflowing x, but the compiler MUST
; NOT fold it to `x`: at x = Int64.max the INNER `(+ x 1)` overflows, and that trap is observable (the
; overflowing `+`'s value flows into the outer `-`). Reassociating/cancelling would ELIDE the trap — a
; miscompile. Straight-line checked +/* is emitted as WRITTEN, each op keeping its own overflow check
; (numeric-model.md §Overflow Is Defined). This is DISTINCT from the loop-accumulator reassociation
; (accum.rs), which deliberately accepts a trap-TIMING change on an already-overflowing input while
; preserving the final value — a loop-tail transform, not a straight-line cancellation. These pin that a
; non-overflowing input computes the value AND an overflowing inner op still traps, on both backends.

(case "straight-line add-then-subtract keeps the value but is not cancelled to the operand"
  (doc    "`(- (+ x 1) 1)` = x for a non-overflowing x (x = 7 → 7), but it is NOT folded to `x`: at
           x = Int64.max the inner `(+ x 1)` overflows and MUST trap — cancelling the +1/-1 would elide
           that defined trap. Pins both faces: the value at 7, and the inner-overflow trap at Int64.max.
           A runtime x keeps this out of the constant fold; straight-line checked `+` is emitted as
           written, each op range-checked, so the reassociation is never applied (unlike the loop
           accumulator, which accepts a trap-timing change).")
  (input  (do (def (main (: x Int64)) (- (+ x 1) 1)) (export main)))
  (call   main (: 7 Int64))
  (output (: 7 Int64))
  (call   main (: 9223372036854775807 Int64))
  (trap   "integer overflow"))

(case "a nested constant multiply preserves the inner overflow rather than combining the factors"
  (doc    "`(* (* x 2) 3)` = 6x for a small x (x = 5 → 30), but the factors are NOT combined to `* 6`:
           at x = 5e18 the inner `(* x 2)` = 1e19 already overflows Int64 (max ≈ 9.22e18) and MUST trap —
           combining to `(* x 6)` would compute 3e19 in one step and, more to the point, a fold that
           skipped the inner check would change WHERE (or whether) the overflow is observed. Pins the
           value at 5 and the inner-overflow trap at 5e18, both backends.")
  (input  (do (def (main (: x Int64)) (* (* x 2) 3)) (export main)))
  (call   main (: 5 Int64))
  (output (: 30 Int64))
  (call   main (: 5000000000000000000 Int64))
  (trap   "integer overflow"))

(case "a runtime arithmetic right shift preserves the sign"
  (doc    "`(>> a b)` on a signed runtime operand emits `i64.shr_s` (arithmetic, sign-extending);
           `(-256, 7)` = -2, matching the folded `(>> -256 7)`. A logical shift would answer a large
           positive value — pins that the emitted `>>` is the arithmetic shift signed LEB128 relies on.")
  (input  (do (def (main (: a Int64) (: b Int64)) (>> a b)) (export main)))
  (call   main (: -256 Int64) (: 7 Int64))
  (output (: -2 Int64)))

(case "a runtime greater-than compares by signed order"
  (doc    "`(> a b)` over runtime operands emits `i64.gt_s`; `(5, 3)` = true. Pins the emitted signed
           ordering comparison (a bounds check a compiler runs on a runtime length).")
  (input  (do (def (main (: a Int64) (: b Int64)) (> a b)) (export main)))
  (call   main (: 5 Int64) (: 3 Int64))
  (output (: true Bool)))

(case "a runtime less-than at the integer extremes is signed"
  (doc    "`(< a b)` over runtime operands is `i64.lt_s`, not `i64.lt_u`: `(Int64.min, Int64.max)` = true.
           Int64.min's bit pattern is the largest UNSIGNED value, so an unsigned compare would wrongly
           answer false — pins that the emitted comparison is SIGNED for a signed type, the runtime dual
           of the folded `(< -9223372036854775808 9223372036854775807)` case.")
  (input  (do (def (main (: a Int64) (: b Int64)) (< a b)) (export main)))
  (call   main (: -9223372036854775808 Int64) (: 9223372036854775807 Int64))
  (output (: true Bool)))

; ═══ Width-indexed integers: (Int N) / (UInt N) over a compile-time width N in 1..=64 ═════
; numeric-model.md #An Integer Type Is Indexed By A Compile-Time Width: an integer type is identified
; by a signedness and a bit width, resolved from a COMPILE-TIME value (never runtime data), with an
; out-of-range width rejected at compile time. The concrete form is pinned at options/numeric-model/
; (explicit-checked): the two width-indexed type constructors `Int` and `UInt`, applied to a width N in
; 1..=64 — `(Int 64)`, `(UInt 8)`, `(UInt 48)`, `(UInt 62)` — each a distinct CHECKED type (traps on
; overflow of its own range, numeric-model.md #Overflow Is Defined). `Int8/16/32/64` and `UInt8/16/32/64`
; are ALIASES for the eight aliased widths (`Int64` ≡ `(Int 64)`), not separate primitives; those eight
; are the ones with a boundary representation, so a non-aliased width like `(UInt 48)` is internal-only.
; `Int64` stays the type a bare literal takes (the cases above); these witness the other widths, the
; alias equivalence, the width constraint (CDZ0302), and the two explicit conversion forms. A
; generation realizing only the 64-bit checked Int64 core
; (options/realized-capability-set/) lowers every integer as an i64 with no width-indexed types, so
; it DECLINES these; the M4 generation that realizes width-indexed integers (riding on generics —
; a compile-time width as a type-constructor argument) runs them. A compiler needs UInt8 (a module's
; bytes) and UInt32 (section sizes, LEB128 operands, table and memory indices); an unusual width like
; `(UInt 48)` (a packed timestamp) is a first-class type the compiler COMPUTES rather than a wrapper the
; author hand-writes — the point of indexing the width rather than fixing a set of primitives.

; --- Construction and per-width bounds ---------------------------------------------------

(case "an annotated value takes a narrower integer width"
  (doc    "`(: 200 UInt8)` is the unsigned 8-bit value 200 — an annotation reaches a width other than
           the default Int64 (numeric-model.md #Integer Types Have Fixed Widths). Its canonical value
           form is the integer 200 at type UInt8; the boundary maps UInt8 to the component model's u8
           (options/numeric-model/ boundary mapping).")
  (input  (: 200 UInt8))
  (output (: 200 UInt8)))

(case "the maximum unsigned 8-bit value is its per-width bound"
  (doc    "`UInt8.max` is 255 — the largest value UInt8 holds, the per-width analogue of Int64.max.
           Each fixed-width type carries its own bounds (numeric-model.md #Integer Types Have Fixed
           Widths); a compiler laying out a byte reaches for exactly this bound.")
  (input  UInt8.max)
  (output (: 255 UInt8)))

(case "the minimum signed 8-bit value is its per-width bound"
  (doc    "`Int8.min` is -128 — the smallest value the two's-complement Int8 holds (its range is
           -128..=127, asymmetric like every signed two's-complement width). Pins that a signed narrow
           width carries its own signed bounds.")
  (input  Int8.min)
  (output (: -128 Int8)))

(case "a UInt64 holds a value above the signed 64-bit maximum"
  (doc    "`UInt64.max` is 18446744073709551615 = 2^64 - 1, above Int64.max (2^63 - 1) — the value that
           distinguishes UInt64 from Int64. It is a well-typed UInt64 value, not an out-of-range
           literal, because the annotation names the unsigned width. The boundary maps it to the
           component model's u64.")
  (input  UInt64.max)
  (output (: 18446744073709551615 UInt64)))

(case "a high UInt64 literal operand takes UInt64 from context, not Int64"
  (doc    "A bare integer literal in [2^63, 2^64-1] as an OPERAND of a binary op whose other operand is a
           UInt64 value takes UInt64 from that operand — the type constraint (numeric-model.md §An
           Explicit Type Annotation Or Other Constraint On An Integer Literal Takes Precedence). A LOW
           literal already does (`(& x 255)` infers UInt64), and UInt8/UInt32 high literals already do;
           the gap was UInt64-only because only it has representable values above Int64.max, so a
           full-width literal `18446744073709551615` (2^64-1) was fit-checked against the signed-64
           DEFAULT and rejected CDZ0201 before the UInt64 context propagated. `(& x 0xFFFF…FFFF)` masks
           the low 64 bits — returns x unchanged — so main(0) => 0. Pins that the binary-op sibling's
           concrete unsigned type constrains the literal, closing the gap the annotation/`.max` workarounds
           had to route around.")
  (input  (do
            (def (main (: x UInt64)) (& x 18446744073709551615))
            (export main)))
  (call   main (: 0 UInt64))
  (output (: 0 UInt64)))

; --- Checked overflow per width (numeric-model.md #Overflow Is Defined, at each width) ----

(case "unsigned 8-bit addition that overflows its width is rejected at compile time"
  (doc    "`(+ (: 255 UInt8) (: 1 UInt8))` = 256, one past UInt8.max, so it overflows the checked UInt8
           range — the per-width analogue of `(+ Int64.max 1)`. Both operands are compile-time constants,
           so the compiler PROVES the overflow and REJECTS the build (CDZ0304, the same code the wide
           `(+ Int64.max 1)` gets), rather than deferring to a runtime trap — reject-don't-miscompile
           (numeric-model.md #Overflow Is Defined). Each fixed-width type is checked at its OWN range,
           not only at 64 bits; a naive lowering that computed in i32 and kept 256 would produce a value
           outside UInt8. (A RUNTIME UInt8 addition whose operands are unknown until run time still traps
           at run time — the checked-arith path emits the width range-check; only the both-constant case
           is proven and rejected up front.)")
  (input  (+ (: 255 UInt8) (: 1 UInt8)))
  (error  CDZ0304))

(case "unsigned subtraction below zero is rejected at compile time"
  (doc    "`(- (: 0 UInt8) (: 1 UInt8))` would be -1, which UInt8 cannot represent (its range is
           0..=255), so the subtraction overflows the unsigned range. Both operands are constants, so the
           compiler PROVES the underflow and rejects the build (CDZ0304) rather than deferring to a
           runtime trap. The unsigned-underflow companion of the overflow case: a checked unsigned type
           traps below zero, it does not wrap to 255.")
  (input  (- (: 0 UInt8) (: 1 UInt8)))
  (error  CDZ0304))

(case "signed 8-bit addition that overflows its width is rejected at compile time"
  (doc    "`(+ (: 127 Int8) (: 1 Int8))` = 128, one past Int8.max (127), so it overflows the checked
           Int8 range. Both operands are constants, so the compiler PROVES the overflow and rejects the
           build (CDZ0304). Pins that the narrow SIGNED width is checked at its own boundary too — a wrap
           would give -128 (Int8.min), the classic signed-overflow wrong value.")
  (input  (+ (: 127 Int8) (: 1 Int8)))
  (error  CDZ0304))

; --- No silent promotion ACROSS widths or signedness (numeric-model.md #Numeric ... Promote) --
; The no-promotion rule the `(+ 2 2.0)` case pins for Int64/Float64 applies equally to two integer
; types of different WIDTH or SIGNEDNESS: they are distinct types, so mixing them without an explicit
; conversion is rejected (CDZ0301), exactly as an Int/Float mix is. A lowering that silently widened
; the UInt8 to Int32 (or reinterpreted signedness) would be the implicit widening the author did not
; write.

(case "mixing two integer widths without a conversion does not silently promote"
  (doc    "`(+ (: 1 UInt8) (: 2 Int32))` mixes a UInt8 and an Int32 — two distinct integer types — so
           it is rejected (CDZ0301) rather than silently widening the UInt8 to Int32. The width analogue
           of the `(+ 2 2.0)` Int/Float no-promotion case (numeric-model.md #Numeric Types Do Not
           Silently Promote).")
  (input  (+ (: 1 UInt8) (: 2 Int32)))
  (error  CDZ0301))

(case "mixing signed and unsigned of the same width does not silently promote"
  (doc    "`(+ (: 1 Int32) (: 2 UInt32))` mixes Int32 and UInt32 — same width, different signedness,
           still distinct types — so it is rejected (CDZ0301). Signedness is not silently reinterpreted;
           the author must convert one side explicitly. Pins that no-promotion holds across signedness,
           not only across width.")
  (input  (+ (: 1 Int32) (: 2 UInt32)))
  (error  CDZ0301))

; --- Explicit conversions: T.of is checked, T.wrap truncates (options/numeric-model/) --------
; A conversion between integer types is always explicit (numeric-model.md #Integer Types Have Fixed
; Widths). `T.of x` is range-CHECKED — it traps when x does not fit T. `T.wrap x` TRUNCATES — it keeps
; the low bits under T's two's-complement representation. `UInt8.wrap` is the byte-truncation the
; compiler's own LEB128 encoder uses; there is no separate byte-conversion op.

(case "a checked integer conversion that fits succeeds"
  (doc    "`(UInt8.of (: 200 Int32))` converts the Int32 200 to UInt8 — 200 is within 0..=255, so the
           checked conversion succeeds and yields the UInt8 200. Pins that T.of is the explicit,
           range-checked conversion the no-silent-promotion rule requires between widths.")
  (input  (UInt8.of (: 200 Int32)))
  (output (: 200 UInt8)))

(case "a checked integer conversion of an out-of-range CONSTANT is rejected at compile time"
  (doc    "`(UInt8.of (: 256 Int32))` converts 256 to UInt8, but 256 is outside 0..=255. Because the
           operand is a COMPILE-TIME CONSTANT, the compiler already knows at const-fold that it cannot
           fit, so it REJECTS the conversion CDZ0302 (integer does not fit the target width) — consistent
           with `(: 128 Int8)` → CDZ0302 and `(/ 1 0)` → CDZ0304, rather than emitting a runtime trap for
           a statically-impossible conversion. (A RUNTIME `T.of` whose value is unknown until run time
           still range-checks and traps at run time; only a compile-time-known out-of-range constant is
           rejected up front.) Contrast UInt8.wrap below, which keeps the low bits and never rejects.")
  (input  (UInt8.of (: 256 Int32)))
  (error  CDZ0302))

(case "a checked conversion of a negative CONSTANT into an unsigned type is rejected"
  (doc    "`(UInt8.of (: -1 Int32))` converts -1 to UInt8, but UInt8 has no negative values. The constant
           -1 provably does not fit at compile time, so the checked conversion is REJECTED CDZ0302 (not a
           runtime trap). Contrast `(UInt8.wrap -1)` = 255 below. Pins that T.of checks the sign boundary,
           not only the magnitude boundary, and rejects an out-of-range constant up front.")
  (input  (UInt8.of (: -1 Int32)))
  (error  CDZ0302))

(case "a truncating conversion keeps the low bits rather than trapping"
  (doc    "`(UInt8.wrap (: 256 Int32))` = 0: the truncating conversion keeps the low 8 bits of 256
           (0x100 -> 0x00), so it yields 0 rather than trapping. This is the byte-truncation
           the LEB128 encoder uses (06-numeric #integer to byte wraps on overflow).
           Pins T.wrap as the low-bits conversion distinct from the checked T.of.")
  (input  (UInt8.wrap (: 256 Int32)))
  (output (: 0 UInt8)))

(case "a truncating conversion of a negative value uses two's complement"
  (doc    "`(UInt8.wrap (: -1 Int32))` = 255: truncating keeps the low 8 bits of -1's two's-complement
           representation (all ones), so it yields 255
           (06-numeric #negative integer to byte uses two's complement).
           Pins that T.wrap reinterprets the low bits, where T.of would trap on the negative value.")
  (input  (UInt8.wrap (: -1 Int32)))
  (output (: 255 UInt8)))

; --- Signedness selects the operation: unsigned compare and unsigned right shift ------------
; The width family's SIGNEDNESS is observable, not just a label: an unsigned type's ordering and right
; shift are UNSIGNED where the signed type's are signed. The sharpest witness is a bit pattern that is
; negative read as signed but large read as unsigned — the dual of the `(< Int64.min Int64.max)` case
; that pins Int64's ordering as SIGNED.

(case "unsigned comparison orders by magnitude, not by signed interpretation"
  (doc    "`(< (: 0 UInt64) UInt64.max)` is true: UInt64.max = 2^64 - 1 is the LARGEST unsigned value,
           above 0, so the unsigned ordering ranks 0 below it. Read as a SIGNED Int64, UInt64.max's bit
           pattern is -1 (which would rank below 0) — so this pins that UInt64's ordering is UNSIGNED
           (i64.lt_u), the dual of `(< Int64.min Int64.max)` which pins Int64's ordering as signed
           (i64.lt_s). Signedness selects the compare.")
  (input  (< (: 0 UInt64) UInt64.max))
  (output (: true Bool)))

(case "an unsigned right shift fills with zeros, not the sign bit"
  (doc    "`(>> UInt8.max (: 1 UInt8))` = 127: a UInt8 right shift is LOGICAL (zero-filling), so shifting
           255 (0xFF) right by 1 yields 127 (0x7F). The signed `>>` on Int64 sign-extends (06-numeric
           #arithmetic right shift preserves the sign bit — `(>> -256 7)` = -2); on an UNSIGNED type the
           same operator is the logical shift (i64.shr_u). Pins that signedness selects arithmetic vs
           logical shift, the property signed/unsigned LEB128 both depend on.")
  (input  (>> UInt8.max (: 1 UInt8)))
  (output (: 127 UInt8)))

; --- The compiler's own use: a UInt32 wasm section size and a UInt8 module byte -------------
; These pin the family at the exact widths the self-hosting compiler needs — a section size / index is
; a UInt32, a module byte is a UInt8 — so the M4 generation's own encoding arithmetic is well-typed
; rather than an untyped i64 masked by hand (options/numeric-model/ #Why these choices).

(case "a UInt32 holds a wasm section size at the width boundary"
  (doc    "`UInt32.max` = 4294967295 = 2^32 - 1, the largest 32-bit unsigned value — the width a wasm
           section size, LEB128 operand, and table/memory index occupy. Pins UInt32 at its boundary; a
           compiler that carried this as an i64 would lose the width the module format actually uses.")
  (input  UInt32.max)
  (output (: 4294967295 UInt32)))

(case "a UInt32 addition that overflows the 32-bit width is rejected at compile time"
  (doc    "`(+ UInt32.max (: 1 UInt32))` = 2^32, one past UInt32.max, so it overflows the checked UInt32
           range — even though 2^32 fits comfortably in the i64 the seed would use. Both operands are
           constants, so the compiler PROVES the overflow and rejects the build (CDZ0304). Pins that
           UInt32 is checked at 32 bits, not at 64: a compiler computing a section size must catch a
           32-bit overflow, not silently carry a value the wasm format cannot encode.")
  (input  (+ UInt32.max (: 1 UInt32)))
  (error  CDZ0304))

; --- A narrow binary op's operand widths reconcile regardless of operand ORDER --------------
; A narrow-width binary op (its width fixed by a narrow-typed variable operand) must emit BOTH operands
; at the narrow (i32) representation — including a bare integer LITERAL operand, whichever side it is on.
; `(+ 1 n)` and `(+ n 1)` with `n : UInt8` are the same well-typed program and must both compile+run;
; the operator's shared width/sign is a unification VARIABLE that a concrete operand binds, so a deferred
; literal on EITHER side reconciles to the variable operand's width. (Binding that shared variable to the
; deferred literal FIRST — when the literal was the LEFT operand — froze it, so the narrow variable's
; width was ignored and the literal emitted as an i64 constant beside the i32 variable → INVALID WASM
; "expected i64, found i32". The const-RIGHT form always worked; these pin operand-order independence
; across arithmetic, bitwise, shift, AND comparison, at the narrow widths the self-hosting compiler uses.)

(case "a narrow addition reconciles a constant LEFT operand to the variable's width"
  (doc    "`(+ 1 n)` with `n : UInt8` adds a bare literal (LEFT) to a narrow variable (RIGHT); the result
           is UInt8. With n = 5 it is 6. Both operands emit at the narrow i32 representation — the literal
           is reconciled to `n`'s width regardless of its (left) position, exactly as the const-RIGHT
           `(+ n 1)` form is. Was INVALID WASM (the left literal emitted as an i64 constant beside the i32
           variable). Pins operand-width reconciliation is position-independent.")
  (input  (do (def (main (: n UInt8)) (+ 1 n)) (export main)))
  (call   main (: 5 UInt8))
  (output (: 6 UInt8)))

(case "a narrow multiplication reconciles a constant LEFT operand to the variable's width"
  (doc    "The multiplication companion: `(* 2 n)` with `n : UInt8`, n = 3, is 6. Confirms the const-LEFT
           reconciliation is not addition-specific — every narrow binary op narrows a bare literal
           operand to the op's width on either side. Was invalid wasm.")
  (input  (do (def (main (: n UInt8)) (* 2 n)) (export main)))
  (call   main (: 3 UInt8))
  (output (: 6 UInt8)))

(case "a narrow comparison reconciles a constant LEFT operand to the variable's width"
  (doc    "The bug hit every narrow binary op, comparisons included. `(< 1 n)` with `n : UInt8`, n = 5, is
           true. Both operands of the comparison emit at the i32 representation; the bare literal LEFT
           operand is reconciled to `n`'s width (a comparison's result is Bool, so its operand widths are
           reconciled at emit, not through a shared result type). Was invalid wasm; the const-RIGHT `(< n
           1)` always worked. Pins the reconciliation covers comparisons (and by the same rule `- & | ^ <<
           >> =`).")
  (input  (do (def (main (: n UInt8)) (if (< 1 n) 1 0)) (export main)))
  (call   main (: 5 UInt8))
  (output (: 1 Int64)))

; --- The named widths are ALIASES for the width-indexed constructors ------------------------
; `Int8/16/32/64` and `UInt8/16/32/64` are ordinary aliases for `(Int 8)`…`(UInt 64)`, not distinct
; primitives (options/numeric-model/ #Integers are width-indexed). The alias and its expansion name the
; SAME type, so a value annotated one way equals the same value annotated the other and the two
; annotations do not conflict. These pin that `UInt8` is nothing more than `(UInt 8)`.

(case "a named width alias and its width-indexed expansion name the same type"
  (doc    "`(: 200 (UInt 8))` is the unsigned 8-bit value 200, exactly as `(: 200 UInt8)` is — `UInt8` is
           the alias `(UInt 8)`. The value form is the integer 200 at the unsigned 8-bit type either way;
           the canonical output is written with the aliased name. Pins that the width-indexed constructor
           applied to 8 is the same type the alias names, not a distinct one.")
  (input  (: 200 (UInt 8)))
  (output (: 200 UInt8)))

(case "the width-indexed and aliased annotations of one value do not conflict"
  (doc    "`(: (: 5 (UInt 32)) UInt32)` annotates a value first as `(UInt 32)` then as `UInt32` — the
           same type under two names, so the annotations agree and the value is well-typed (NOT a CDZ0203
           annotation conflict). Pins the alias equivalence through the annotation-conflict checker: an
           alias and its expansion are interchangeable, never contradictory.")
  (input  (: (: 5 (UInt 32)) UInt32))
  (output (: 5 UInt32)))

; --- A NON-ALIASED in-range width is an equally first-class, internal-only type -------------
; The whole point of indexing the width: `(UInt 48)`, `(UInt 62)`, `(UInt 12)` are ordinary types with
; no shorter name, whose bounds, mask, and operations the compiler COMPUTES from N — not wrappers the
; author hand-writes (options/numeric-model/ #Why these choices). They have no boundary representation
; (only the eight aliased widths do), so they are internal-only; these cases construct and overflow one
; inside a program, where the packing wins live.

(case "an unusual in-range width is a first-class type"
  (doc    "`(: 281474976710655 (UInt 48))` is `(UInt 48).max` = 2^48 - 1, the largest value a 48-bit
           unsigned integer holds — a packed-timestamp width with no aliased name. It is an ordinary
           well-typed value of `(UInt 48)`, its bound computed from N=48 (numeric-model.md #An Integer
           Type Is Indexed By A Compile-Time Width). Pins that a non-aliased in-range width is
           first-class, not a special case the language lacks.")
  (input  (: 281474976710655 (UInt 48)))
  (output (: 281474976710655 (UInt 48))))

(case "an unusual-width value that overflows its computed range is rejected at compile time"
  (doc    "`(+ (: 281474976710655 (UInt 48)) (: 1 (UInt 48)))` = 2^48, one past `(UInt 48).max`, so it
           overflows the checked 48-bit range — the overflow bound is COMPUTED from N=48, not drawn from
           a fixed width table. Both operands are constants, so the compiler PROVES the overflow and
           rejects the build (CDZ0304). Pins that a non-aliased width is checked at its own width exactly
           as an aliased one is; a naive lowering that only checked the 8/16/32/64 boundaries would carry
           2^48 as a wrong value.")
  (input  (+ (: 281474976710655 (UInt 48)) (: 1 (UInt 48))))
  (error  CDZ0304))

(case "a truncating conversion to an unusual width keeps that width's low bits"
  (doc    "`((UInt 48).wrap (: -1 Int64))` = 281474976710655: the truncating conversion keeps the low 48
           bits of -1's two's-complement representation (48 ones) = 2^48 - 1. Pins that `T.wrap` computes
           its mask from N for a non-aliased width too — the low-N-bits rule is uniform across every
           width, aliased or not (options/numeric-model/ #Conversions are explicit).")
  (input  ((UInt 48).wrap (: -1 Int64)))
  (output (: 281474976710655 (UInt 48))))

; --- The width is a COMPILE-TIME constant in 1..=64; an out-of-range width is CDZ0302 -------
; numeric-model.md #An Integer Type Is Indexed By A Compile-Time Width: the width is resolved from a
; compile-time value, and a width outside the admitted range is rejected at compile time with the
; unsatisfied-width-constraint diagnostic (CDZ0302, options/diagnostics-schema/) — the same
; compile-time-constraint rejection any generic instantiation gets (type-system.md #A Generic Constraint
; Is A Compile-Time Predicate Over Type-Values), NOT a runtime trap. The 1..=64 ceiling is the range a
; single core-wasm register holds; a width above 64 is reserved to the big-integer layer, so it too is
; a CDZ0302 rejection today, not a valid type.

(case "a zero-bit integer width is rejected"
  (doc    "`(: 0 (UInt 0))` names a zero-bit integer, which holds no values — a width of 0 is outside the
           admitted 1..=64 range, so the type is rejected at compile time (CDZ0302), the width analogue of
           any generic instantiation whose argument fails its constraint. Not a runtime trap: the type
           itself is ill-formed.")
  (input  (: 0 (UInt 0)))
  (error  CDZ0302))

(case "a negative integer width is rejected"
  (doc    "`(: 5 (Int -8))` names an integer of width -8 — a negative width, which the CDZ0302 registry
           entry lists explicitly ('a negative width'). It is outside the admitted 1..=64 range and
           rejected at compile time exactly as `(UInt 0)` is. A negative width MUST NOT be silently
           dropped so the literal keeps its default Int64 — the ill-formed width is the rejection, not a
           footnote to a value the annotation is ignored to produce.")
  (input  (: 5 (Int -8)))
  (error  CDZ0302))

(case "a negative unsigned integer width is rejected"
  (doc    "`(: 5 (UInt -1))` — the unsigned companion, width -1. Same negative-width rejection (CDZ0302)
           as the signed case; the sign of the constructor does not make a negative width admissible.")
  (input  (: 5 (UInt -1)))
  (error  CDZ0302))

(case "a negative width is not honored as a narrow type"
  (doc    "`(: 300 (Int -8))` — the discriminating case: if `(Int -8)` were (wrongly) honored as some
           narrow width, 300 would overflow it; if the width were silently dropped to Int64, 300 fits and
           the program returns 300. Neither is correct — the ill-formed negative width is rejected
           (CDZ0302) before any literal-fit check, so the outcome is the rejection, not 300.")
  (input  (: 300 (Int -8)))
  (error  CDZ0302))

(case "a boolean in integer-width position is rejected"
  (doc    "`(: 300 (Int true))` puts a Bool where a compile-time natural width belongs — a non-natural
           width (the CDZ0302 registry entry covers 'a negative width, or a non-natural width'). Rejected
           at compile time, not silently degraded to Int64. A non-integer value in width position is
           ill-formed exactly as a negative one is.")
  (input  (: 300 (Int true)))
  (error  CDZ0302))

(case "a float in integer-width position is rejected"
  (doc    "`(: 300 (Int 8.0))` puts a Float where a natural width belongs — a non-natural width, CDZ0302.
           `(Int 8)` would be a valid 8-bit type in which 300 overflows; the float `8.0` is neither
           accepted-as-8 nor overflow-checked — it is an ill-formed width, rejected.")
  (input  (: 300 (Int 8.0)))
  (error  CDZ0302))

(case "a type-value in integer-width position is rejected"
  (doc    "`(: 300 (Int Int64))` puts a type-value where a width natural belongs — a non-natural width,
           CDZ0302. Completes the non-natural-width family (negative / bool / float / type) that shares
           one rule: a width the compiler cannot read as a natural in 1..=64 is rejected, never dropped.")
  (input  (: 300 (Int Int64)))
  (error  CDZ0302))

(case "an integer width above the 64-bit ceiling is rejected"
  (doc    "`(: 5 (UInt 65))` names a 65-bit integer, one past the 1..=64 ceiling — a width a single
           core-wasm register cannot hold. It is rejected at compile time (CDZ0302); a fixed-size integer
           wider than 64 bits is reserved to the opt-in big-integer layer, not the width-indexed
           constructor (options/numeric-model/ #Widths above 64 are reserved). Pins the upper boundary of
           the width constraint.")
  (input  (: 5 (UInt 65)))
  (error  CDZ0302))

(case "an over-ceiling integer width in an unused parameter is rejected, like a used one"
  (doc    "`(UInt 65)` as the type of a PARAMETER of a private def that is never called with a literal —
           `(def (f (: x (UInt 65))) x)` with `f` unused — is rejected CDZ0302, exactly as the value
           annotation `(: 5 (UInt 65))` is. Well-formedness is TOTAL: it holds over every definition,
           reachable or not (an unbound name in the same unused def is CDZ0101), so an ill-formed integer
           width must be rejected wherever the annotation appears — not only where a literal is fit-checked
           against it or the def is exported. Pins that the width constraint is checked at the annotation
           itself, closing the escape where a private unconstrained-parameter type carried a width with no
           valid representation into a compiled artifact.")
  (input  (do
            (def (f (: x (UInt 65))) x)
            (def (main) 0)
            (export main)))
  (error  CDZ0302))

(case "a malformed integer width — negative or non-natural — is rejected at compile time"
  (doc    "`(: 5 (Int -8))` names a NEGATIVE width. A bit width is a compile-time NATURAL number in
           1..=64; a negative (or fractional, or non-numeric — a bool/type-value in width position) width
           is not a natural number at all, so it is ill-formed and rejected CDZ0302 — the companion of the
           over-ceiling case above (which names a natural width past 64). Well-formedness is TOTAL
           (numeric-model.md: a bit width outside the range the model admits MUST be rejected at compile
           time), so this is caught at the annotation in the shared front-end, not left for each backend to
           catch independently at selection. Before, a non-natural width silently slipped past `cdz check`
           (which exited 0) while the value ran with its default width — a check-vs-emit gap this closes.
           Pins the LOWER/non-natural boundary of the width constraint.")
  (input  (: 5 (Int -8)))
  (error  CDZ0302))

(case "a wide fixed-size integer width is reserved, not yet a valid type"
  (doc    "`(: 5 (UInt 128))` names a 128-bit integer — beyond the 1..=64 register-width ceiling, so it is
           rejected (CDZ0302) today rather than silently accepted. The notation is reserved: a later
           optional increment MAY realize wide fixed-size integers as a multi-word representation and lift
           the ceiling, at which point `(UInt 128)` becomes valid with no surface-syntax change. Until
           then, more than 64 bits uses the big-integer type. Pins that the ceiling is a constraint, not a
           parse error.")
  (input  (: 5 (UInt 128)))
  (error  CDZ0302))

(case "an ill-formed integer width NESTED in a compound annotation is rejected"
  (doc    "`(: (list 1) (List (Int -8)))` carries the ill-formed width `-8` one level down, inside a
           `List` element type rather than at the top of the annotation. Well-formedness is structural: a
           bit width outside 1..=64 (or non-natural) is rejected wherever it appears in a type expression,
           not only when it is the whole annotation. `(List (Int -8))` reduces to a well-formed container of
           an ill-formed element type, so the top-level type LOOKS valid and the annotation slipped past
           `cdz check` (which exited 0) while the value ran with a default width — the same check-vs-emit gap
           the bare `(Int -8)` case closes, one nesting level deeper. The front-end descends the compound
           annotation and rejects CDZ0302 at the nested width, exactly as if it were written bare. Companion
           to the bare negative-width and the parameter-position cases above.")
  (input  (: (list 1) (List (Int -8))))
  (error  CDZ0302))

(case "an ill-formed integer width in a type-declaration payload is rejected"
  (doc    "`(type T (Mk (Int -8)))` puts the ill-formed width `-8` in a variant payload field of a type
           declaration — a type-expression position the shared front-end validates, not a value annotation.
           A width outside 1..=64 is rejected CDZ0302 at the declaration, before any value of `T` is
           constructed, exactly as the same width in a value or parameter annotation is. Pins that the width
           constraint is TOTAL over every type-expression position — declaration payloads included — so an
           ill-formed width cannot enter a compiled artifact through a type definition either.")
  (input  (do
            (type T (Mk (Int -8)))
            (def (main) 0)
            (export main)))
  (error  CDZ0302))

(case "an integer width from runtime data is rejected, keeping widths non-dependent"
  (doc    "`(UInt n)` with `n` a runtime function parameter puts a runtime value in a type-determining
           position, which the type system forbids (numeric-model.md #An Integer Type Is Indexed By A
           Compile-Time Width: the width MUST be resolved from a compile-time value and MUST NOT be
           determined by runtime data; type-system.md #Generics Are Type-Valued Parameters). It is
           rejected at compile time (CDZ0302), keeping the feature at indexed types over compile-time
           naturals rather than dependent types — no runtime value ever determines a type. A width the
           compiler cannot resolve to a compile-time natural — negative, non-natural, OR runtime — reduces
           to the invalid sentinel width 0 and is rejected at the annotation (CDZ0302), never dropped so
           the literal falls back to its default type. (An earlier annotation SKIPPED this case and hid
           the miscompile — the seed ran `(mk 8)` to 5; run unconditionally + fixed, it rejects.)")
  (input  (do
            (def (mk n) (: 5 (UInt n)))
            (def (main) (mk 8)) (export main)))
  (error  CDZ0302))

(case "an absurd runtime width is rejected, not silently ignored"
  (doc    "The sharper witness: `(: 5 (UInt n))` with `n` supplied as a RUNTIME argument (99, an
           out-of-range width). As a constant `(UInt 99)` names a width past the 64 ceiling; as runtime
           data the width is not a compile-time natural at all. Either way it MUST reject (CDZ0302), not
           be accepted-and-ignored — the width reader resolves neither, so the annotation reduces to the
           sentinel width 0 and rejects. Pins that the runtime branch rejects like the negative/non-
           natural branches, closing the drop-instead-of-reject family (no runtime value in a type
           position).")
  (input  (do
            (def (mk (: n Int64)) (: 5 (UInt n)))
            (def (main (: k Int64)) (mk k)) (export main)))
  (call   main (: 99 Int64))
  (error  CDZ0302))

; --- Negation `(- 0 a)` overflows only at the type's MIN -------------------------------------------
; Negating a two's-complement integer overflows at exactly ONE input: the type's minimum, whose
; magnitude has no positive counterpart in the range (numeric-model.md #Overflow Is Defined — the
; checked form traps rather than wrapping). Every other value negates cleanly. The backend specializes
; the `(- 0 a)` negation overflow guard to the single test `a == MIN` (rather than the general
; subtraction round-trip); these cases pin that the guard fires at MIN and NOWHERE else, and — the sharp
; part — that MIN is the WIDTH'S min, so a narrow `Int8`/`Int16` negation traps at -128 / -32768 and not
; only at the Int64 min (a specialization that compared against the 64-bit min would silently wrap the
; narrow overflow to a truncated value instead of trapping).

(case "negating a runtime Int64 near the boundary is exact just above the min"
  (doc    "`(- 0 n)` is negation. At n = Int64.min + 1 it is Int64.max, at Int64.max it is Int64.min + 1,
           and ordinary values negate cleanly — every input EXCEPT the min negates without overflow. Pins
           the value side of the `(- 0 a)` negation: the guard must NOT over-fire on the values adjacent
           to the min (a live Pass guard; the min-input overflow is the companion trap case below).")
  (input  (do (def (main (: n Int64)) (- 0 n)) (export main)))
  (call   main (: -9223372036854775807 Int64))
  (output (: 9223372036854775807 Int64))
  (call   main (: 9223372036854775807 Int64))
  (output (: -9223372036854775807 Int64))
  (call   main (: 5 Int64))
  (output (: -5 Int64))
  (call   main (: 0 Int64))
  (output (: 0 Int64)))

(case "negating a runtime Int64 overflows at Int64.min"
  (doc    "The companion of the boundary case above: at n = Int64.min (-9223372036854775808) the result
           +9223372036854775808 has no Int64 representation, so the checked negation TRAPS rather than
           wrapping to the min (numeric-model.md #Overflow Is Defined). Pins that the `(- 0 a)` overflow
           guard FIRES at the single input where negation overflows — a `(- 0 min)` that ran to a value
           would be the miscompile this catches.")
  (input  (do (def (main (: n Int64)) (- 0 n)) (export main)))
  (call   main (: -9223372036854775808 Int64))
  (trap   "integer overflow"))

(case "negating a runtime narrow integer just above its min is exact"
  (doc    "`(- 0 n)` over `Int8` gives 127 at n = -127 and negates ordinary values cleanly. The value side
           of the narrow-width negation: a live Pass guard that the `(- 0 a)` guard does not over-fire on
           an Int8 value adjacent to its min. The min-input overflow is the companion trap case below.")
  (input  (do (def (main (: n Int8)) (- 0 n)) (export main)))
  (call   main (: -127 Int8))
  (output (: 127 Int8))
  (call   main (: 100 Int8))
  (output (: -100 Int8)))

(case "negating a runtime narrow integer traps at the width's own min, not the Int64 min"
  (doc    "`(- 0 n)` over `Int8` traps at n = -128 (Int8.min, whose negation 128 is out of the -128..127
           range). The overflow is at the WIDTH'S min, so a negation-overflow guard that specialized
           against the 64-bit min would MISS this and wrap -(-128) to a truncated value instead of
           trapping. Pins that the `(- 0 a)` guard uses the operand's own width for MIN — the sharp edge
           of the specialized negation overflow guard.")
  (input  (do (def (main (: n Int8)) (- 0 n)) (export main)))
  (call   main (: -128 Int8))
  (trap   "integer overflow"))

; --- A narrow op with a control-flow (if/let) operand: the WIDTH is decided by the operands, not by an
; --- unrelated narrow param in the CONDITION. These pin the boundary that a narrow op whose deferred-width
; --- branch is wrapped down to the op's width KEEPS its overflow guard (the wrap-down reconciliation does
; --- NOT drop the range-check), and — conversely — that an op whose operands are all deferred-width
; --- literals is genuinely Int64 and its wide result is CORRECT (not a leaked narrow overflow). ------------

(case "an if-operand arithmetic op with deferred-width branches is Int64, and its wide result is correct"
  (doc    "`(def (main (: n Int8)) (+ (if (< n 5) 100 0) 100))` — the `Int8` param `n` appears ONLY in the
           condition `(< n 5)`; the `+`'s two operands are the `if` (whose branches are bare deferred-width
           literals) and the bare literal `100`, so both default to Int64 and the `+` is an Int64 add. With
           n=3 the then-branch 100 is selected and 100 + 100 = 200 — a perfectly representable Int64. The
           result is 200, NOT a trap: there is no Int8 constraint on the arithmetic to overflow. (A narrow
           param in a nearby CONDITION must not be mistaken for a width constraint on the enclosing op.)")
  (input  (do (def (main (: n Int8)) (+ (if (< n 5) 100 0) 100)) (export main)))
  (call   main (: 3 Int8))
  (output (: 200 Int64)))

(case "a genuinely-narrow op with a wrapped-down if-operand still traps on overflow"
  (doc    "When the op IS constrained narrow — here by a return annotation `(: (+ …) Int8)` — the deferred-
           width `if`-branch operand (an i64 slot) is wrapped down to the op's Int8 width, and the op's
           overflow range-check MUST survive that wrap-down. With n=3 the branch is 100 and 100 + 100 = 200
           overflows Int8 (max 127), so it TRAPS — exactly as the plain `(: (+ 100 100) Int8)` would. Pins
           that the wrap-down reconciliation of a control-flow operand does not drop the narrow guard.")
  (input  (do (def (main (: n Int8)) (: (+ (if (< n 5) 100 0) 100) Int8)) (export main)))
  (call   main (: 3 Int8))
  (trap   "integer overflow"))

(case "a genuinely-narrow op with a wrapped-down if-operand yields the exact value when in range"
  (doc    "The value companion of the trap above: the SAME `(: (+ (if (< n 5) 100 0) 100) Int8)` with n=9
           takes the else-branch 0, so 0 + 100 = 100 fits Int8 and the result is 100 — the wrap-down guard
           does not over-fire on an in-range result. Together the two cases pin the narrow overflow guard
           surviving the if-operand wrap-down in BOTH directions (overflow traps, in-range is exact).")
  (input  (do (def (main (: n Int8)) (: (+ (if (< n 5) 100 0) 100) Int8)) (export main)))
  (call   main (: 9 Int8))
  (output (: 100 Int8)))

(case "a genuinely-narrow op with a wrapped-down MATCH-operand yields the exact value"
  (doc    "The `match` analogue of the if-operand wrap-down: `(: (+ (match n (0 5) (_ 1)) 2) Int8)` — the
           `match` arm bodies are deferred-width (Int64-defaulting) literals, so the whole `match` is Int64;
           the enclosing narrow `+` must WRAP IT DOWN to Int8 before adding. With n=9 the wildcard arm gives
           1, 1 + 2 = 3 fits Int8 → 3. Pins that a `match`-operand takes the same narrow wrap-down an
           if-operand does — on every backend (the Rust backend omitted it, emitting an i64 `match` into an
           i8 add → rustc E0308; the wrap-down `as i8` on the match sub-expression fixes it, matching wasm).")
  (input  (do (def (main (: n Int8)) (: (+ (match n (0 5) (_ 1)) 2) Int8)) (export main)))
  (call   main (: 9 Int8))
  (output (: 3 Int8)))



(case "a runtime BigInt accumulator threaded through recursion grows past Int64"
  (doc    "`(loop 70 (BigInt.of 1))` doubles a BigInt accumulator 70 times — `2^70` =
           1180591620717411303424, a value FAR beyond Int64.max (~9.2e18). The accumulator is a runtime
           BigInt threaded through the recursion (a param at every level), multiplied on the runtime limb
           library each step, and the result crosses to the host via the value-encode walker. Pins the
           real BigInt use case (an exponentiation/factorial accumulator) end to end: the initial
           constant `(BigInt.of 1)` argument materializes as a heap handle (not a raw i64 — the fixed
           call-arg-to-a-BigInt-param miscompile), the recursion threads the handle, and the unbounded
           result never overflows.")
  (input  (do
            (def (loop (: n Int64) (: acc BigInt))
              (if (= n 0) acc (loop (- n 1) (* acc (BigInt.of 2)))))
            (def (main) (loop 70 (BigInt.of 1)))
            (export main)))
  (output (: 1180591620717411303424 BigInt)))

(case "a BigInt factorial accumulator computes 25! exactly"
  (doc    "THE canonical BigInt program: `fac(25)` with a BigInt accumulator — `(* acc (BigInt.of n))` at
           each level, `n` a RUNTIME loop variable widened to BigInt — computes 25! =
           15511210043330985984000000, a 26-digit value ~1.7 million× beyond Int64.max. Every intermediate
           (13!… onward already exceeds Int64) is carried exactly on the runtime limb library, the
           accumulator threaded through the recursion as a heap handle, and the exact result crosses to the
           host. The definitive end-to-end proof of unbounded BigInt arithmetic in the factorial idiom —
           the same program over Int64 would trap at the first intermediate past 2^63.")
  (input  (do
            (def (fac (: n Int64) (: acc BigInt))
              (if (= n 0) acc (fac (- n 1) (* acc (BigInt.of n)))))
            (def (main) (fac 25 (BigInt.of 1)))
            (export main)))
  (output (: 15511210043330985984000000 BigInt)))

(case "a BigInt is usable as a set element, deduplicated by its arbitrary-precision value"
  (doc    "`(Set.len (Set.of (list (BigInt.of 5) (BigInt.of 5) (BigInt.of 7))))` = 2: a set of BigInt
           elements DEDUPLICATES by value — the two `5`s collapse to one (the CHAMP set hashes/compares
           each element over its canonical sign-magnitude bytes, `champ_hash`/`champ_eq`, the same raw-byte
           basis as a Bytes/String element), leaving `{5, 7}` of size 2. The set companion of the
           BigInt-map-key case; pins that a BigInt is a first-class set element (a constant element
           materializes as a heap handle at the insert site, not a raw i64).")
  (input  (Set.len (Set.of (list (BigInt.of 5) (BigInt.of 5) (BigInt.of 7)))))
  (output (: 2 Int64)))

(case "a BigInt is usable as a map key, matched by its arbitrary-precision value"
  (doc    "`(Map.lookup (Map.insert (Map.insert Map.empty (BigInt.of 100) 1) (BigInt.of 200) 2) (BigInt.of
           200))` = `Some 2`: a BigInt KEY is inserted and looked up by VALUE — the CHAMP map hashes and
           compares it over its canonical sign-magnitude bytes (`champ_hash`/`champ_eq`, the same raw-byte
           basis as a Bytes/String key), so the second key `200` finds its stored value 2. Pins that a
           BigInt is a first-class map key. (A constant BigInt key materializes as a heap handle at the
           insert/lookup site — the `Core::ConstInt`-typed-BigInt emit routes through `bigint-of-i64` —
           rather than a raw i64, which would be an invalid module.)")
  (input  (match (Map.lookup
                   (Map.insert (Map.insert Map.empty (BigInt.of 100) 1) (BigInt.of 200) 2)
                   (BigInt.of 200))
                 ((Some v) v)
                 ((None) 0)))
  (output (: 2 Int64)))

(case "a match on a BigInt with a single catch-all binder binds and uses it"
  (doc    "`(match (BigInt.of n) (z (* z z)))` with n=6 → 36: a match whose ONLY arm is a plain binder
           `z` binds the (runtime) BigInt scrutinee to `z` and yields `(* z z)` — it inspects no structure,
           so it needs no probe chain and no heap walk, and lowers straight to the body (the `bigint-mul`).
           Pins that a catch-all binding match works over a BigInt scrutinee (before, the match engine
           rejected a non-scalar scrutinee `matching a compound value needs a heap walk` even for a
           bare-binder arm that never looks at it).")
  (input  (do
            (def (main (: n Int64)) (Int64.of (match (BigInt.of n) (z (* z z)))))
            (export main)))
  (call   main (: 6 Int64))
  (output (: 36 Int64)))

(case "narrowing a BigInt-valued if back to Int64 works over both branches"
  (doc    "`(Int64.of (if (= (BigInt.of a) (BigInt.of 0)) (BigInt.of 1) (BigInt.of a)))` — an `if` whose
           BOTH branches yield a BigInt, narrowed back to Int64: a=0 → 1, a=5 → 5. `Int64.of`
           (`bigint-to-i64-checked`) BORROWS its operand, so the emit drops each owned-temporary branch
           result; each branch is a constant BigInt that materializes to a fresh owned handle, so the
           if-operand's ownership is provable (both branches Owned) — before, `Int64.of` of a BigInt-valued
           `if` declined `ownership … cannot yet prove` because a constant-BigInt branch was unclassified.")
  (input  (do
            (def (main (: a Int64))
              (Int64.of (if (= (BigInt.of a) (BigInt.of 0)) (BigInt.of 1) (BigInt.of a))))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 1 Int64)))

(case "two BigInt accumulators co-threaded through recursion compute a fibonacci"
  (doc    "`loop(n, a, b) = if n=0 then a else loop(n-1, b, a+b)` with TWO BigInt accumulators threaded
           side by side computes fib(10) = 55. Exercises multiple runtime-BigInt params co-threaded through
           one recursion (each a heap handle at every level, `a+b` a runtime `bigint-add`), a distinct shape
           from the single-accumulator factorial/doubling loops — pins that several BigInt params materialize
           + thread correctly at once. fib grows exponentially, so a larger n crosses Int64 (this n keeps
           the check readable).")
  (input  (do
            (def (loop (: n Int64) (: a BigInt) (: b BigInt))
              (if (= n 0) a (loop (- n 1) b (+ a b))))
            (def (main) (Int64.of (loop 10 (BigInt.of 0) (BigInt.of 1))))
            (export main)))
  (output (: 55 Int64)))

(case "a runtime Rational built from a parameter compares by its exact value"
  (doc    "`(< (Rational.of a 3) (Rational.of 1 2))` with runtime `a` builds a Rational from a runtime
           numerator (`Rational.of a 3` → widen `a` + `3` to BigInt, `rational-of` normalizes) and compares
           it exactly via the runtime `rational-cmp`: a=1 → 1/3 < 1/2 → true → 1. THE runtime-Rational path
           (R3b): the constant fold does not apply (the numerator is a parameter), so the compiler emits
           `bigint-of-i64` + `rational-of` + `rational-cmp` on the runtime limb library. The a=2 companion
           (2/3 < 1/2 → false) pins the other direction.")
  (input  (do
            (def (main (: a Int64)) (if (< (Rational.of a 3) (Rational.of 1 2)) 1 0))
            (export main)))
  (call   main (: 1 Int64))
  (output (: 1 Int64)))

(case "a runtime Rational comparison of a parameter is false when it should be"
  (doc    "The false-direction companion of the runtime-Rational compare: `(< (Rational.of a 3) (Rational.of
           1 2))` with a=2 → 2/3 < 1/2 → false → 0. Same runtime `rational-cmp` path, the greater operand.")
  (input  (do
            (def (main (: a Int64)) (if (< (Rational.of a 3) (Rational.of 1 2)) 1 0))
            (export main)))
  (call   main (: 2 Int64))
  (output (: 0 Int64)))

(case "runtime Rational arithmetic adds two parameter-built fractions exactly"
  (doc    "`(< (+ (Rational.of a b) (Rational.of 1 2)) (Rational.of 1 1))` with a=1,b=3: the sum
           1/3 + 1/2 = 5/6 is computed by the runtime `rational-add` (both operands built from runtime ints
           via `rational-of`, so the constant fold does not apply), then compared `< 1` → 5/6 < 1 → true →
           1. Pins runtime rational ADDITION on the limb library (the emitted ops are `bigint-of-i64` +
           `rational-of` + `rational-add` + `rational-cmp`) — exact, no rounding.")
  (input  (do
            (def (main (: a Int64) (: b Int64))
              (if (< (+ (Rational.of a b) (Rational.of 1 2)) (Rational.of 1 1)) 1 0))
            (export main)))
  (call   main (: 1 Int64) (: 3 Int64))
  (output (: 1 Int64)))

(case "a Rational accumulator threaded through recursion sums exactly"
  (doc "Threading a Rational through a recursive accumulator sums 1/2 three times to 3/2, then compares 3/2 < 2/1 as true, exercising borrow/consume drop discipline across the recursion.")
  (input (do (def (loop (: n Int64) (: acc Rational)) (if (= n 0) acc (loop (- n 1) (+ acc (Rational.of 1 2))))) (def (main) (if (< (loop 3 (Rational.of 0 1)) (Rational.of 2 1)) 1 0)) (export main)))
  (output (: 1 Int64)))

(case "a runtime-computed Rational crosses the host boundary as its exact value"
  (doc    "`(Rational.of-int (Int64.of (* (BigInt.of 1000000) (BigInt.of 1000000))))` — a Rational built
           from a RUNTIME value (the BigInt product 1e12 does not fold, so `Rational.of-int` of the
           narrowed result is a runtime Rational, NOT a constant) crosses to the host rendered `1000000000000/1
           : Rational`. THE runtime-Rational boundary escape (R3c): the compiler routes the nullary-export
           Rational result through the runtime `value-encode` walker (a `Shape::Rational` descriptor, tag
           18), which reads the 2-BigInt-handle node and formats the `num/den` name leaf — the same value
           form a constant Rational bakes. Mirrors the runtime-BigInt boundary escape.")
  (input  (Rational.of-int (Int64.of (* (BigInt.of 1000000) (BigInt.of 1000000)))))
  (output (: 1000000000000/1 Rational)))

(case "a Rational is usable as a map key, matched by its exact value"
  (doc    "`(Map.lookup (Map.insert (Map.insert Map.empty (Rational.of 1 2) 10) (Rational.of 2 3) 20)
           (Rational.of 1 2))` = `Some 10`: a Rational KEY is inserted and looked up by VALUE — the CHAMP
           map hashes/compares it via `champ_hash`/`champ_eq` descending the two BigInt child leaves (the
           normalized 2-handle node), so `1/2` finds its stored value 10. Pins that a Rational is a
           first-class map key (a constant Rational key materializes as a heap handle at the insert/lookup
           site — `box_op_ty`/`get_op_ty` treat it as already-a-handle, like BigInt).")
  (input  (match (Map.lookup
                   (Map.insert (Map.insert Map.empty (Rational.of 1 2) 10) (Rational.of 2 3) 20)
                   (Rational.of 1 2))
                 ((Some v) v)
                 ((None) 0)))
  (output (: 10 Int64)))

(case "a Rational set deduplicates by exact value regardless of how each was written"
  (doc    "`(Set.len (Set.of (list (Rational.of 1 2) (Rational.of 2 4) (Rational.of 1 3))))` = 2: `1/2` and
           `2/4` normalize to the SAME rational (one node shape), so the set collapses them — leaving
           `{1/2, 1/3}` of size 2. Confirms a Rational set element deduplicates by its normalized value
           (the CHAMP descends the two BigInt children), the set companion of the Rational-map-key case.")
  (input  (Set.len (Set.of (list (Rational.of 1 2) (Rational.of 2 4) (Rational.of 1 3)))))
  (output (: 2 Int64)))

(case "a parameterized export returns a runtime BigInt computed from its argument"
  (doc    "A `main` that TAKES a parameter and RETURNS a BigInt crosses the host boundary as a resource
           whose `make` forwards the argument (`make(a) -> own<t>`), so the host computes the value from
           its input: `main(5000000000) = 5000000000 * 3 = 15000000000`, a value beyond Int64 that no
           constant fold could produce. Closes the last cross-cutting heap-return limit (a List/BigInt/
           Rational from a parameterized export all declined identically before).")
  (input  (do (def (main (: a Int64)) (* (BigInt.of a) (BigInt.of 3))) (export main)))
  (call   main (: 5000000000 Int64))
  (output (: 15000000000 BigInt)))

(case "a parameterized export returns a runtime Rational computed from its argument"
  (doc    "The Rational companion: `main(1) = 1/6 + 1/6 = 1/3` exactly, the runtime rational built from
           the argument crossing the boundary via the param-forwarding resource escape.")
  (input  (do (def (main (: a Int64)) (+ (Rational.of a 6) (Rational.of 1 6))) (export main)))
  (call   main (: 1 Int64))
  (output (: 1/3 Rational)))

(case "a RUNTIME-built Rational is usable as a map key, matched by its exact value"
  (doc    "The runtime companion of the constant Rational-map-key case: `(Rational.of a 2)` with a
           RUNTIME `a` builds the key via the `rational-of` runtime op (widen `a`+`2` to BigInt, then
           `rational-of`) rather than the constant-fold path, so the key is a live handle threaded into
           `Map.insert`/`Map.lookup`. `main(1)` inserts `1/2 -> 10`, looks up `1/2`, finds `Some 10` —
           the CHAMP still hashes/compares it by descending the two BigInt children. Pins that the
           RUNTIME materialization (distinct lowering from the constant `ConstRational`) also lands in
           a map-key slot as a proper heap handle.")
  (input  (do (def (main (: a Int64))
                (match (Map.lookup (Map.insert Map.empty (Rational.of a 2) 10) (Rational.of a 2))
                       ((Some v) v)
                       ((None) 0)))
              (export main)))
  (call   main (: 1 Int64))
  (output (: 10 Int64)))

(case "a RUNTIME-built Rational set deduplicates by exact value"
  (doc    "The runtime companion of the constant Rational-set case: with a RUNTIME `a`, the three
           elements `(Rational.of a 2)`/`(Rational.of a 4)`/`(Rational.of a 3)` are built via the
           `rational-of` op (not folded). `main(2)` = `{2/2, 2/4, 2/3}` = `{1/1, 1/2, 2/3}`, three
           distinct normalized rationals, so `Set.len` = 3 — confirming a runtime-materialized Rational
           set element deduplicates by its normalized value through the CHAMP's two-BigInt-child walk.")
  (input  (do (def (main (: a Int64))
                (Set.len (Set.of (list (Rational.of a 2) (Rational.of a 4) (Rational.of a 3)))))
              (export main)))
  (call   main (: 2 Int64))
  (output (: 3 Int64)))

; --- Runtime overflow guards at exact boundaries the fold never sees (adversarial anchors) --------
; The `(- 0 min)` negation trap above pins one boundary; these pin the remaining single-input
; boundaries where a checked op's guard must fire (or hold) at RUN time — operands arrive as
; parameters, so nothing folds and the emitted guard itself is exercised. Each pairs the largest
; passing input with the smallest trapping one, so a guard that is off by even one unit fails.

(case "a runtime increment traps exactly at the maximum integer"
  (doc    "`(+ x 1)` with x a parameter: at x = Int64.max - 1 (9223372036854775806) the sum is Int64.max
           — the largest representable value, in range → 9223372036854775807; at x = Int64.max the sum
           +2^63 has no Int64 representation and the emitted add-overflow guard TRAPS (numeric-model.md
           #Overflow Is Defined). The one-unit pair pins the guard boundary exactly: a guard testing
           `>= max` (instead of `> max`) breaks the passing call, one testing wraparound-after-the-fact
           breaks the trapping call.")
  (input  (do (def (main (: x Int64)) (+ x 1)) (export main)))
  (call   main (: 9223372036854775806 Int64))
  (output (: 9223372036854775807 Int64))
  (call   main (: 9223372036854775807 Int64))
  (trap   "integer overflow"))

(case "a runtime square multiply traps exactly past the Int64 square-root boundary"
  (doc    "`(* x x)` at the integer square-root boundary of Int64.max: isqrt(2^63 - 1) = 3037000499,
           whose square 9223372030926249001 FITS (Int64.max - 5926526806 above it); the next integer
           3037000500 squares to 9223372037000250000 > Int64.max, so the multiply-overflow guard TRAPS.
           A multiplication guard built from a 64-bit high-half check (or a division-based check with a
           truncation bias) is most likely to be wrong within one unit of this boundary — the adjacent
           pass/trap pair pins it from both sides.")
  (input  (do (def (main (: x Int64)) (* x x)) (export main)))
  (call   main (: 3037000499 Int64))
  (output (: 9223372030926249001 Int64))
  (call   main (: 3037000500 Int64))
  (trap   "integer overflow"))

(case "an absolute-value branch traps on the minimum integer it cannot represent"
  (doc    "The abs idiom `(if (< x 0) (- 0 x) x)`: at x = Int64.min the negative branch is taken and
           `(- 0 min)` = +2^63 has no Int64 representation → the negation's overflow guard traps, exactly
           as the direct `(- 0 n)` case above. Pins that the guard survives INSIDE a conditional branch —
           an if→select conversion that evaluates both arms speculatively, or a branch-local guard
           elision, would either trap the WRONG inputs or wrap this one. The control x = -5 takes the
           same branch and yields 5 (the guard holds for every negation but min's).")
  (input  (do (def (main (: x Int64)) (if (< x 0) (- 0 x) x)) (export main)))
  (call   main (: -5 Int64))
  (output (: 5 Int64))
  (call   main (: -9223372036854775808 Int64))
  (trap   "integer overflow"))

; --- Runtime checked arithmetic guards AT the narrow width (adversarial boundary pairs) -----------
; The constant-fold cases above PROVE a narrow overflow and reject (CDZ0304); their doc notes the
; runtime path "still traps at run time" — these grade that promise. A UInt8 rides in a wide slot, so
; the checked op must range-check at width 8, not 64: computing 256 in the wide slot and keeping it
; would be a value outside the type. Each case pairs the largest passing operands with the smallest
; trapping ones, pinning the guard boundary exactly (off-by-one either way fails one call).

(case "a runtime unsigned-byte addition traps exactly past its width maximum"
  (doc    "`(+ x 1)` over a runtime `x : UInt8`: at x = 254 the sum 255 = UInt8.max FITS → 255; at
           x = 255 the sum 256 exceeds the 8-bit range and the checked add TRAPS — at the NARROW width,
           though the carrying i64 slot holds 256 comfortably (numeric-model.md #Overflow Is Defined at
           each width). The runtime companion the constant `(+ (: 255 UInt8) (: 1 UInt8))` → CDZ0304
           case promises in prose.")
  (input  (do (def (main (: x UInt8)) (+ x (: 1 UInt8))) (export main)))
  (call   main (: 254 UInt8))
  (output (: 255 UInt8))
  (call   main (: 255 UInt8))
  (trap   "integer overflow"))

(case "a runtime unsigned-byte multiplication traps exactly past its width maximum"
  (doc    "`(* x y)` over runtime UInt8 operands: 15 × 17 = 255 = UInt8.max FITS; 16 × 16 = 256 TRAPS.
           The multiply companion — a lowering that checked the product against the i64 (or even i32)
           range instead of the 8-bit range would let 256 through as a UInt8-typed value.")
  (input  (do (def (main (: x UInt8) (: y UInt8)) (* x y)) (export main)))
  (call   main (: 15 UInt8) (: 17 UInt8))
  (output (: 255 UInt8))
  (call   main (: 16 UInt8) (: 16 UInt8))
  (trap   "integer overflow"))

(case "a runtime unsigned-byte subtraction traps below zero"
  (doc    "`(- x y)` over runtime UInt8 operands: 1 - 1 = 0 FITS (the range floor); 0 - 1 = -1 has no
           unsigned representation and TRAPS. An unsigned checked subtract implemented as the signed
           i64 subtract (where -1 is representable) with only an upper-bound check would return a
           negative value reinterpreted as a huge UInt8. The floor companion of the two ceiling cases.")
  (input  (do (def (main (: x UInt8) (: y UInt8)) (- x y)) (export main)))
  (call   main (: 1 UInt8) (: 1 UInt8))
  (output (: 0 UInt8))
  (call   main (: 0 UInt8) (: 1 UInt8))
  (trap   "integer overflow"))

(case "a checked addition of two runtime-wrapped bytes still guards at the narrow width"
  (doc    "`(+ (UInt8.wrap a) (UInt8.wrap b))` — wrap is TOTAL (truncates to the low byte), but the
           checked `+` over the wrapped results must STILL range-check at width 8: a = 511 wraps to 255,
           so +1 overflows → trap, while +0 is 255 → passes. Pins that a totalizing wrap feeding a
           checked op does not launder the width guard away (the wrapped value is a genuine UInt8, its
           arithmetic checked like any other), and that the guard reads the MASKED value, not the wide
           pre-wrap slot (511 + 0 in the wide slot is 511 — a guard on the raw slot would wrongly trap
           the passing call).")
  (input  (do (def (main (: a Int64) (: b Int64)) (+ (UInt8.wrap a) (UInt8.wrap b))) (export main)))
  (call   main (: 511 Int64) (: 1 Int64))
  (trap   "integer overflow")
  (call   main (: 511 Int64) (: 0 Int64))
  (output (: 255 UInt8)))

(case "a runtime byte truncation of the sign extremes keeps exactly the low byte"
  (doc    "`(UInt8.wrap n)` at the i64 sign extremes: n = -256 (low byte 0x00) → 0 and n = Int64.min
           (= -2^63, low byte 0x00) → 0. The -1 → 255 case above pins the all-ones low byte; these pin
           the all-zeros low byte reached from a NEGATIVE wide value — a wrap emitted as a signed
           modulo/remainder (rather than a bit mask) yields 0 for -256 but a sign-flipped remainder for
           other negatives, and Int64.min is the operand where signed-magnitude tricks (negate-then-mask)
           themselves overflow. Total, no trap, exactly the low 8 bits.")
  (input  (do (def (main (: n Int64)) (UInt8.wrap n)) (export main)))
  (call   main (: -256 Int64))
  (output (: 0 UInt8))
  (call   main (: -9223372036854775808 Int64))
  (output (: 0 UInt8)))

(case "a runtime nibble truncation keeps the low four bits"
  (doc    "`((UInt 4).wrap n)` — wrap at a NON-BYTE width (the width the bin bit-field segments take):
           n = 17 = 0b10001 keeps the low nibble 0b0001 → 1; n = 15 = 0b1111 fits whole → 15. Pins that
           the truncation masks at the type's OWN width (a byte-mask reused for every narrow width would
           keep 17). The (UInt 4) companion of the UInt8 wrap cases.")
  (input  (do (def (main (: n Int64)) ((. (UInt 4) wrap) n)) (export main)))
  (call   main (: 17 Int64))
  (output (: 1 (UInt 4)))
  (call   main (: 15 Int64))
  (output (: 15 (UInt 4))))

; --- Unary negation: prefix `-<expr>` is the arity-1 subtraction `(- e)` -----------------------
; The ML surface `-x` (prefix minus applied to an expression, not a bare literal) canonicalizes to the
; ONE-operand subtraction `(- e)`, negation. It is `0 - e` at the operand's numeric type — closed over
; every numeric type, with the integer `MIN`-overflow trap the binary subtraction already carries — and
; is NOT the wrong-arity error a bare binary operator otherwise is. Negating a non-numeric value is a
; type error (CDZ0201), the unary twin of arithmetic-on-a-non-number. A negative LITERAL (`-1`, `-1.5`)
; lexes as a signed literal, a separate path, so these witness negation of an EXPRESSION specifically.

(case "unary negation of a bound integer name yields its opposite"
  (doc    "`(- x)` with x a let-bound Int64 is negation — `0 - x` at Int64 — so `-5` = -5. Witnesses the
           arity-1 subtraction as negation, not a wrong-arity `- takes exactly 2 operands` error.")
  (input  (let ((x 5)) (- x)))
  (output (: -5 Int64)))

(case "unary negation binds tighter than binary addition"
  (doc    "The ML surface `-x + 1` parses as `(+ (- x) 1)` — prefix negation binds tighter than every
           infix operator — so with x = 5 it is `-5 + 1` = -4, not `-(x + 1)` = -6. Pins negation as a
           tight prefix over its single operand.")
  (input  (let ((x 5)) (+ (- x) 1)))
  (output (: -4 Int64)))

(case "unary negation of a parenthesized sum negates the whole expression"
  (doc    "`-(x + 1)` (ML) canonicalizes to `(- (+ x 1))`: the parenthesized sum is the single operand, so
           with x = 5 the result is -(5 + 1) = -6. The companion of the tighter-than-`+` case above.")
  (input  (let ((x 5)) (- (+ x 1))))
  (output (: -6 Int64)))

(case "unary negation of a runtime integer parameter negates at run time"
  (doc    "`(- n)` with n a parameter emits the runtime `0 - n` (the checked subtract): n = 7 → -7. At
           n = Int64.min the negation `-(-2^63)` = +2^63 has no Int64 representation and the subtract's
           overflow guard TRAPS, exactly as `(- 0 n)` does — negation inherits the binary subtraction's
           `x == MIN` trap. Pins that negation is emitted (not only constant-folded) and traps correctly.")
  (input  (do (def (main (: n Int64)) (- n)) (export main)))
  (call   main (: 7 Int64))
  (output (: -7 Int64))
  (call   main (: -9223372036854775808 Int64))
  (trap   "integer overflow"))

(case "unary negation in a lambda body negates the argument"
  (doc    "A negating function `(def (neg x) (- x))` applied to 7 yields -7 — prefix negation in a
           function body over its (inferred-numeric) parameter. Witnesses `-x` in the `fn`/`def`-body
           position the workaround `0 - x` was reached for.")
  (input  (do (def (neg x) (- x)) (def (main) (neg 7)) (export main)))
  (output (: -7 Int64)))

(case "unary negation of a float flips the sign, preserving signed zero"
  (doc    "`(- x)` with x a Float64 is negation at the float type — emitted as `-1.0 * x`, NOT `0.0 - x`
           (IEEE `0.0 - (+0.0)` = +0.0, but negation must flip a zero's sign). So `-(0.0)` = -0.0, which
           is distinct from +0.0 by the canonical byte form (core-semantics.md §Floating-Point Equality
           Follows The Canonical Byte Form). Pins float negation and its signed-zero correctness.")
  (input  (let ((x 0.0)) (- x)))
  (output (: -0.0 Float64)))

(case "unary negation of a nonzero float"
  (doc    "`(- x)` with x = 5.0 negates to -5.0 — the ordinary (nonzero) float-negation case.")
  (input  (let ((x 5.0)) (- x)))
  (output (: -5.0 Float64)))

(case "unary negation of an exact rational negates the numerator"
  (doc    "`(- r)` with r = 1/4 is exact rational negation (`0 - 1/4`), yielding -1/4 — the sign lives on
           the numerator of the normalized form. Witnesses negation over the exact `Rational` type.")
  (input  (let ((r (Rational.of 1 4))) (- r)))
  (output (: -1/4 Rational)))

(case "unary negation of an arbitrary-precision integer"
  (doc    "`(- b)` with b a BigInt is unbounded negation (`0 - b` via the runtime bigint-sub), yielding
           -5 — never overflowing (the point of the type). Witnesses negation over BigInt.")
  (input  (let ((b (BigInt.of 5))) (- b)))
  (output (: -5 BigInt)))

(case "unary negation of a quantity preserves its unit"
  (doc    "`(- q)` with q = 5 meter negates the erased magnitude while keeping the dimension: -5 meter.
           Witnesses that negation is defined over a `Qty` and does not strip its unit (units-of-measure.md
           — the running arithmetic is the inner numeric operation, the unit is carried through).")
  (input  (let ((q (Qty.of 5 (Unit.base #"meter")))) (- q)))
  (output (: (Qty.of -5 (Unit.base #"meter")) (Qty Int64 (Unit.base #"meter")))))

(case "unary negation of a non-numeric value is a type error"
  (doc    "`(- s)` with s a String is rejected (CDZ0201): negation is not defined on a non-numeric value,
           the unary twin of arithmetic-on-a-non-number. Cadenza never coerces a String to a number. A
           generation that does not yet cover unary negation declines (reject-don't-miscompile).")
  (input  (let ((s "hi")) (- s)))
  (error  CDZ0201))
