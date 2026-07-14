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
           Float64.of-int is written explicitly, then the two Float64 operands are added with the FLOAT
           operator `+.` (numeric-model.md #A Floating-Point Operation Uses A Floating-Point Operator) —
           the integer `+` would reject even two Float64 operands, since `+` is int-only.")
  (input  (+. (Float64.of-int 1) 2.0))
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
           the FLOAT operator `+.` — 0.1 and 0.2 are Float64, and `+.` is the float addition distinct
           from the integer `+` (numeric-model.md #A Floating-Point Operation Uses A Floating-Point
           Operator). The famous non-exact sum: 0.1 + 0.2 rounds to 0.30000000000000004, not 0.3.")
  (input  (+. 0.1 0.2))
  (output (: 0.30000000000000004 Float64)))

; --- Float arithmetic uses the DISTINCT float operators `+.` `-.` `*.` `/.` --------------------------
; numeric-model.md #A Floating-Point Operation Uses A Floating-Point Operator: float arithmetic is
; spelled distinctly from integer arithmetic, so no operator silently mixes an integer and a float
; operand. The integer `+`/`-`/`*`/`/` are int-only (a float operand → CDZ0301, the mixing cases above);
; the float `+.`/`-.`/`*.`/`/.` are float-only (an integer operand → CDZ0301). These pin the float
; operators over Float64 operands, and that an integer operand to a float operator is rejected.

(case "float multiplication uses the float operator"
  (doc    "`(*. 6.0 7.0)` = 42.0 : Float64 — float multiplication under `*.`, the float companion of the
           integer `*`. Result is a Float64 (42.0), not the Int64 42 the integer `(* 6 7)` gives.")
  (input  (*. 6.0 7.0))
  (output (: 42.0 Float64)))

(case "float subtraction uses the float operator"
  (doc    "`(-. 5.5 2.0)` = 3.5 : Float64 — float subtraction under `-.`.")
  (input  (-. 5.5 2.0))
  (output (: 3.5 Float64)))

(case "float division rounds under the fixed mode"
  (doc    "`(/. 1.0 4.0)` = 0.25 : Float64 — float division under `/.`, which ROUNDS (unlike integer `/`
           which truncates and rational `/` which is exact). 1/4 is exactly representable, so 0.25.")
  (input  (/. 1.0 4.0))
  (output (: 0.25 Float64)))

(case "float division that does not divide evenly rounds to nearest"
  (doc    "`(/. 1.0 3.0)` = 0.3333333333333333 : Float64 — 1/3 is not exactly representable in binary64,
           so the quotient rounds to the nearest representable value under the fixed round-to-nearest-even
           mode. Pins that float `/.` rounds deterministically.")
  (input  (/. 1.0 3.0))
  (output (: 0.3333333333333333 Float64)))

(case "the float operator rejects an integer operand and does not promote it"
  (doc    "`(+. 2 2.0)` supplies an Int64 `2` where the float `+.` wants a Float64 — the operand types do
           not unify, so it is rejected (CDZ0301), NOT promoted to `2.0` (numeric-model.md #Numeric Types
           Do Not Silently Promote, and #A Floating-Point Operation Uses A Floating-Point Operator). The
           dual of `(+ 2 2.0)` (an int operator rejecting a float operand): neither operator coerces.")
  (input  (+. 2 2.0))
  (error  CDZ0301))

; --- Runtime float operands: the EMITTED machine op, not the constant fold -----------------------
; The float-arithmetic cases above use CONSTANT operands, so the compiler folds them at build time. A
; value that arrives at RUN TIME (an argument to the exported entry) cannot be folded, so the float
; operator is emitted as a real machine instruction (`f64.add`/…). These `(call <export> <arg>…)` cases
; run each float operator over runtime Float64 operands and pin that the emitted path AGREES with the
; folded constant cases. Unlike the integer arithmetic these emit NO overflow guard — a float op never
; traps (IEEE overflow → inf). CORE cases (the seed realizes runtime Float64 operators).

(case "a runtime float addition emits the machine add"
  (doc    "`(def (main (: a Float64) (: b Float64)) (+. a b))` called with (0.1, 0.2). The addition cannot
           fold (both operands are runtime), so it is emitted as `f64.add` — the non-exact IEEE sum
           0.30000000000000004, matching the folded `(+. 0.1 0.2)` case. Pins the emitted float-add path.")
  (input  (do (def (main (: a Float64) (: b Float64)) (+. a b)) (export main)))
  (call   main (: 0.1 Float64) (: 0.2 Float64))
  (output (: 0.30000000000000004 Float64)))

(case "a runtime float multiplication emits the machine mul"
  (doc    "`(*. a b)` over runtime Float64 operands emits `f64.mul`; `(6.0, 7.0)` = 42.0, matching the
           folded `(*. 6.0 7.0)`. Pins the emitted float-multiply path.")
  (input  (do (def (main (: a Float64) (: b Float64)) (*. a b)) (export main)))
  (call   main (: 6.0 Float64) (: 7.0 Float64))
  (output (: 42.0 Float64)))

(case "a runtime float division rounds under the fixed mode"
  (doc    "`(/. a b)` over runtime operands emits `f64.div`, which rounds under the fixed round-to-
           nearest-even mode; `(1.0, 3.0)` = 0.3333333333333333, matching the folded `(/. 1.0 3.0)`. Pins
           the emitted float-divide path and that it rounds deterministically (not a trap on inexactness).")
  (input  (do (def (main (: a Float64) (: b Float64)) (/. a b)) (export main)))
  (call   main (: 1.0 Float64) (: 3.0 Float64))
  (output (: 0.3333333333333333 Float64)))

(case "a runtime integer converts to a float with the machine convert"
  (doc    "`(Float64.of-int n)` over a runtime Int64 `n` emits `f64.convert_i64_s`; `(of-int 42)` = 42.0.
           The explicit int→float conversion (numeric-model.md #A Conversion Involving A Floating-Point
           Type Is Explicit) is TOTAL — an integer always has a float image (a large magnitude rounds to
           the nearest representable float, it does not trap). Pins the emitted int→float convert path,
           the runtime dual of the folded `(Float64.of-int 1)` inside the `(+. …)` case above.")
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
  (doc    "`(+. (: 1.5 Float32) (: 2.0 Float64))` mixes a Float32 and a Float64 — two distinct float
           types — so it is rejected (CDZ0301) rather than silently widening the Float32 to Float64
           (numeric-model.md #Numeric Types Do Not Silently Promote; #A Conversion Involving A
           Floating-Point Type Is Explicit). The float-width analogue of the integer-width no-promotion
           case; to add them a program converts one side (`(Float64.of …)`).")
  (input  (+. (: 1.5 Float32) (: 2.0 Float64)))
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
  (doc    "The no-promotion rejection `(+. (: 1.5 Float32) (: 2.0 Float64))` is resolved by converting
           the Float32 up explicitly: `(+. (Float64.of (: 1.5 Float32)) 2.0)` = 3.5 : Float64 — both
           operands are now Float64, so `+.` type-checks and adds. Pins that the explicit conversion is
           the sanctioned way to combine two float widths.")
  (input  (+. (Float64.of (: 1.5 Float32)) 2.0))
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

(case "a wide fixed-size integer width is reserved, not yet a valid type"
  (doc    "`(: 5 (UInt 128))` names a 128-bit integer — beyond the 1..=64 register-width ceiling, so it is
           rejected (CDZ0302) today rather than silently accepted. The notation is reserved: a later
           optional increment MAY realize wide fixed-size integers as a multi-word representation and lift
           the ceiling, at which point `(UInt 128)` becomes valid with no surface-syntax change. Until
           then, more than 64 bits uses the big-integer type. Pins that the ceiling is a constraint, not a
           parse error.")
  (input  (: 5 (UInt 128)))
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
