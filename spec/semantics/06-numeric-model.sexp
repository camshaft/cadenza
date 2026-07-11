; Numeric model — witnesses numeric-model.md. The primary clause is the recorded oracle: a well-typed
; program's terminal value or trap, or — for an ill-typed program — its (error <CODE>) rejection, because
; an ill-typed program has no run and therefore no terminal value. For a type rule a generation does not
; yet cover it DECLINES rather than running the program (reject-don't-miscompile); the gate scores a
; decline as todo, not disagreement. (needs numeric-model) marks the extended numerics a later generation
; realizes (the seed realizes only the checked Int64 core and Float64 literals/equality —
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
           Float64.of-int is written explicitly. Needs the numeric-model capability (conversions +
           float arithmetic), which the seed does not realize.")
  (needs numeric-model)
  (input  (+ (Float64.of-int 1) 2.0))
  (output (: 3.0 Float64)))

(case "wrapping arithmetic uses the distinct wrapping type"
  (doc    "Witnesses numeric-model.md #Overflow Is Defined via the distinct wrapping type.")
  (needs numeric-model)
  (input  (+% Wrapping64.max 1))
  (output (: -9223372036854775808 Wrapping64)))

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
; rejects). The written value form is `n/d` in lowest terms. `(needs numeric-model)`: the seed realizes
; only the checked Int64 core and Float64 literals/equality.

(case "exact rational arithmetic is exact and normalized"
  (doc    "Witnesses numeric-model.md #Exact Arithmetic Is Exact; reduced to lowest terms per
           options/numeric-model/. `(+ (Rational.of 1 3) (Rational.of 1 6))` = 1/3 + 1/6 = 1/2 exactly,
           the canonical rational value form (no float rounding — a Float64 sum would not be exact).")
  (needs numeric-model)
  (input  (+ (Rational.of 1 3) (Rational.of 1 6)))
  (output (: 1/2 Rational)))

(case "a rational is normalized to lowest terms on construction"
  (doc    "`(Rational.of 2 4)` reduces to 1/2 — a Rational is kept in lowest terms (numerator and
           denominator share no common factor), so 2/4 and 1/2 are ONE value with one canonical byte
           form (numeric-model.md #An Exact Rational Has A Canonical Normalized Form). Normalization is a
           function of the number, not of how it was written.")
  (needs numeric-model)
  (input  (Rational.of 2 4))
  (output (: 1/2 Rational)))

(case "two rationals denoting the same number are equal regardless of how they were written"
  (doc    "`(= (Rational.of 2 4) (Rational.of 1 2))` is true: because both normalize to 1/2, rational
           equality is structural over the normalized pair (deterministic-value-form.md #A Value Has One
           Canonical Byte Form). Pins that equality compares canonical forms, not the raw numerator/
           denominator a program supplied.")
  (needs numeric-model)
  (input  (= (Rational.of 2 4) (Rational.of 1 2)))
  (output (: true Bool)))

(case "a rational's sign is normalized onto the numerator"
  (doc    "`(Rational.of 1 -2)` normalizes to -1/2 — the fixed sign convention puts the sign on the
           numerator and keeps the denominator strictly positive (numeric-model.md #An Exact Rational
           Has A Canonical Normalized Form). So `(Rational.of 1 -2)`, `(Rational.of -1 2)`, and
           `(Rational.of -1 -2)`'s companions all resolve to one signed canonical form; here the result
           is negative.")
  (needs numeric-model)
  (input  (Rational.of 1 -2))
  (output (: -1/2 Rational)))

(case "a rational with numerator and denominator both negative normalizes to positive"
  (doc    "`(Rational.of -1 -2)` = 1/2: the two negatives cancel under the sign convention (denominator
           forced strictly positive), so a both-negative pair is a positive rational. Companion of the
           sign-on-numerator case, pinning that sign normalization is by the number's sign, not by which
           component carried the minus.")
  (needs numeric-model)
  (input  (Rational.of -1 -2))
  (output (: 1/2 Rational)))

(case "exact rational division is total and exact for a nonzero divisor"
  (doc    "`(/ (Rational.of 1 2) (Rational.of 3 4))` = (1/2)/(3/4) = 4/6 = 2/3 exactly. Rational `/` by
           a NONZERO rational is total and exact — it neither truncates (as integer `/` does) nor rounds
           (as float `/` does) — and the result is normalized to lowest terms. This is the exactness the
           type is opted into for.")
  (needs numeric-model)
  (input  (/ (Rational.of 1 2) (Rational.of 3 4)))
  (output (: 2/3 Rational)))

(case "a whole rational carries a denominator of one"
  (doc    "`(Rational.of-int 5)` is the whole rational 5/1 : Rational — a DISTINCT type from `5 : Int64`.
           Crossing between the integer and the rational is explicit (`Rational.of-int` in), never an
           implicit promotion, the same no-promotion discipline the integer widths obey. Its canonical
           written form is 5/1.")
  (needs numeric-model)
  (input  (Rational.of-int 5))
  (output (: 5/1 Rational)))

(case "constructing a rational with a zero denominator traps"
  (doc    "`(Rational.of 1 0)` denotes no number — a zero denominator has no rational value — so it
           traps (numeric-model.md #A Rational With A Zero Denominator Is Not A Value), the rational
           analogue of integer division by zero. A defined runtime trap, distinct from producing an
           unspecified value.")
  (needs numeric-model)
  (input  (Rational.of 1 0))
  (trap   "rational with zero denominator"))

(case "a rational operation does not silently promote an integer operand"
  (doc    "`(+ (Rational.of 1 2) 1)` mixes a Rational and an Int64 — two distinct numeric types — so it
           is rejected (CDZ0301) rather than promoting the 1 to 1/1, exactly as an Int64/Float64 mix is
           (numeric-model.md #Numeric Types Do Not Silently Promote). To add the integer, a program
           writes the conversion explicitly: `(+ (Rational.of 1 2) (Rational.of-int 1))`.")
  (needs numeric-model)
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
; `(needs numeric-model)`: the seed realizes only the checked Int64 core.

(case "an arbitrary-precision integer multiplication does not overflow"
  (doc    "`(* (BigInt.of 9223372036854775807) (BigInt.of 9223372036854775807))` multiplies two values
           each equal to Int64.max — a product far beyond 64 bits — and yields the exact BigInt
           85070591730234615847396907784232501249, NOT a trap and NOT a wrap (numeric-model.md #An
           Arbitrary-Precision Integer Has Unbounded Range). The same product over Int64 traps
           (`integer overflow`); BigInt's representation grows instead. THE reason the type exists.")
  (needs numeric-model)
  (input  (* (BigInt.of 9223372036854775807) (BigInt.of 9223372036854775807)))
  (output (: 85070591730234615847396907784232501249 BigInt)))

(case "an arbitrary-precision literal beyond 64 bits is an exact BigInt"
  (doc    "`(: 100000000000000000000 BigInt)` annotates a literal larger than Int64.max as a BigInt — an
           exact value with no width worry. Pins that BigInt carries a magnitude the fixed-width family
           cannot (this literal would not fit any `(Int N)`/`(UInt N)` with N ≤ 64) and that its
           canonical written form is the ordinary decimal.")
  (needs numeric-model)
  (input  (: 100000000000000000000 BigInt))
  (output (: 100000000000000000000 BigInt)))

(case "converting a fixed-width integer to BigInt is explicit"
  (doc    "`(BigInt.of 42)` converts the Int64 42 up to the BigInt 42 — the explicit widening into the
           unbounded type. A BigInt and the Int64 42 are DISTINCT types with distinct canonical forms;
           the conversion is always written, never implicit.")
  (needs numeric-model)
  (input  (BigInt.of 42))
  (output (: 42 BigInt)))

(case "converting a BigInt back to a fixed width is checked and traps when out of range"
  (doc    "`((UInt 8).of (BigInt.of 300))` converts a BigInt down to `(UInt 8)`, whose range is 0..=255,
           so 300 does not fit and it TRAPS (numeric-model.md #A Conversion Between Integer Types Is
           Explicit — the checked form), exactly as `((UInt 8).of 300)` on an Int64 does. Pins that
           narrowing OUT of BigInt is checked, not a silent truncation.")
  (needs numeric-model)
  (input  ((UInt 8).of (BigInt.of 300)))
  (trap   "integer overflow"))

(case "a BigInt operation does not silently promote a fixed-width operand"
  (doc    "`(+ (BigInt.of 1) 1)` mixes a BigInt and an Int64 — two distinct numeric types — rejected
           (CDZ0301) rather than absorbing the Int64 1 into BigInt (numeric-model.md #Numeric Types Do
           Not Silently Promote). The unbounded type does not swallow a fixed-width operand; to add, a
           program writes `(+ (BigInt.of 1) (BigInt.of 1))`.")
  (needs numeric-model)
  (input  (+ (BigInt.of 1) 1))
  (error  CDZ0301))

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
; default. Resolved at compile time, then types erase — zero ABI impact. `(needs numeric-model)`.

(case "a default-integer pragma makes a bare literal take the declared integer type"
  (doc    "The `crypto` module declares `(pragma default-integer BigInt)`, so the bare literal 2 in
           `double`'s body is a BigInt, x is a BigInt, and `(double (BigInt.of 21))` = 42 : BigInt — the
           ergonomic escape hatch: a bignum-heavy module writes bare literals without `(BigInt.of …)`
           around each. Pins that the declared default is the type an unconstrained literal takes.")
  (needs numeric-model)
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
  (needs numeric-model)
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
  (needs numeric-model)
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
  (needs numeric-model)
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
  (needs numeric-model)
  (input  (do
            (module m
              (pragma default-integer Float64)
              (def (x) 5))
            ((. m x) unit)))
  (error  CDZ0303))
; (The general pragma mechanism — an unrecognized key rejected CDZ0601, malformed args CDZ0602 — is
;  witnessed in 11-modules.sexp under `needs module-pragmas`; here we pin only the numeric-domain
;  behavior of the `default-integer` key.)

(case "floating-point uses the fixed rounding mode"
  (doc    "The round-to-nearest-even sum under the pinned deterministic float mode
           (contracts/determinism-and-fuel.md); byte-identical on every conforming runtime. The seed
           realizes Float64 literals and equality, not floating-point arithmetic.")
  (needs numeric-model)
  (input  (+ 0.1 0.2))
  (output (: 0.30000000000000004 Float64)))

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

(case "wrapping arithmetic over runtime operands wraps at run time"
  (doc    "The runtime companion: `(w a b)` = `(Int64.wrapping-add a b)` over parameters wraps on the
           i64.add path (wasm's add wraps; no overflow guard), so `(w Int64.max 1)` = Int64.min — the
           same wrap the const fold gives. Pins that wrapping is emitted as the raw i64 op, not the
           checked/trapping one.")
  (input  (do
            (def (w a b) (Int64.wrapping-add a b))
            (def (main) (w Int64.max 1)) (export main)))
  (output (: -9223372036854775808 Int64)))

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
  (doc    "The compiler needs to convert integers to single bytes for wasm encoding.
           Int.to-byte truncates to the low 8 bits (0-255).")
  (input  (Int.to-byte 200))
  (output (: 200 Int64)))

(case "integer to byte wraps on overflow"
  (doc    "Values > 255 wrap to low 8 bits. The compiler uses this for byte encoding.")
  (input  (Int.to-byte 256))
  (output (: 0 Int64)))

(case "negative integer to byte uses two's complement"
  (input  (Int.to-byte -1))
  (output (: 255 Int64)))

; --- The bitwise/shift/to-byte primitives COMPOSE into the LEB128 encoding step ----------
; The cases above exercise `&`, `|`, `>>`, and `Int.to-byte` INDIVIDUALLY on constant operands. The
; compiler's actual use is to COMPOSE them: one LEB128 byte is `(| (& n 127) 128)` when a continuation
; byte follows (the low 7 bits of n, with bit 7 set), or `(& n 127)` for the final byte, and the next
; group is `(>> n 7)`. Composing the operators exercises their interaction — each intermediate is an
; Int64 fed to the next, in evaluation order — which an isolated single-operator case cannot witness. A
; miscompile in operator interaction (a wrong intermediate type, a mis-sequenced fold) would surface
; here where it hides in the isolated cases. These pin the compiler's own encoding arithmetic
; (numeric-model.md #Overflow Is Defined for the exact bit operations; compiler-pipeline.md relies on
; LEB128 for wasm section sizes).

(case "a LEB128 non-final byte composes mask, continuation bit, and to-byte"
  (doc    "One LEB128 continuation byte of 300: `(& 300 127)` = 44 (low 7 bits), `(| 44 128)` = 172 (set
           bit 7), `Int.to-byte` leaves it (already in 0..=255). The composed
           `(Int.to-byte (| (& 300 127) 128))` = 172 — the exact byte a LEB128 encoder emits for the
           first group of 300. Pins that the three operators compose to the encoder's non-final byte,
           not just that each works alone.")
  (input  (Int.to-byte (| (& 300 127) 128)))
  (output (: 172 Int64)))

(case "a LEB128 final byte is the shifted remainder masked to seven bits"
  (doc    "The final group of 300: `(>> 300 7)` = 2 (the remaining bits after the low 7), and
           `(& 2 127)` = 2 (final byte, continuation bit clear). The composed `(& (>> 300 7) 127)` = 2 —
           the encoder's terminating byte. Together with the case above, `300` encodes as the two LEB128
           bytes 172, 2. Pins the shift-then-mask composition for the final group.")
  (input  (& (>> 300 7) 127))
  (output (: 2 Int64)))

(case "the LEB128 byte composition runs on a runtime operand"
  (doc    "The composition above on a RUNTIME operand, not a constant: `(leb-byte n)` = `(Int.to-byte (|
           (& n 127) 128))` with `n` a function parameter, so the mask, continuation-bit OR, and to-byte
           are EMITTED (not const-folded). `(leb-byte 300)` = 172, the same non-final byte the constant
           case produces — but reached through the runtime `i64.and`/`i64.or` the encoder actually
           executes when it encodes a value computed at run time (a section length, an operand). Pins
           that runtime bitwise `&`/`|` (and their composition) are emitted and agree with the const
           fold — a self-hosted LEB128 encoder works on the runtime values it is fed, not only on
           literals. The const cases above fold and so cannot witness the emitted bitwise path; this
           one, taking `n` through a parameter, does.")
  (input  (do
            (def (leb-byte n) (Int.to-byte (| (& n 127) 128)))
            (def (main)       (leb-byte 300)) (export main)))
  (output (: 172 Int64)))

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
; run time, not on literals. CORE cases (no `(needs …)`): the seed realizes runtime Int64 operators.

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
; alias equivalence, the width constraint (CDZ0302), and the two explicit conversion forms. All carry
; (needs numeric-model): the seed realizes only the 64-bit checked Int64 core
; (options/realized-capability-set/) and lowers every integer as an i64 with no width-indexed types, so
; it SKIPS these; the M4 generation that realizes width-indexed integers (riding on generics —
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
  (needs  numeric-model)
  (input  (: 200 UInt8))
  (output (: 200 UInt8)))

(case "the maximum unsigned 8-bit value is its per-width bound"
  (doc    "`UInt8.max` is 255 — the largest value UInt8 holds, the per-width analogue of Int64.max.
           Each fixed-width type carries its own bounds (numeric-model.md #Integer Types Have Fixed
           Widths); a compiler laying out a byte reaches for exactly this bound.")
  (needs  numeric-model)
  (input  UInt8.max)
  (output (: 255 UInt8)))

(case "the minimum signed 8-bit value is its per-width bound"
  (doc    "`Int8.min` is -128 — the smallest value the two's-complement Int8 holds (its range is
           -128..=127, asymmetric like every signed two's-complement width). Pins that a signed narrow
           width carries its own signed bounds.")
  (needs  numeric-model)
  (input  Int8.min)
  (output (: -128 Int8)))

(case "a UInt64 holds a value above the signed 64-bit maximum"
  (doc    "`UInt64.max` is 18446744073709551615 = 2^64 - 1, above Int64.max (2^63 - 1) — the value that
           distinguishes UInt64 from Int64. It is a well-typed UInt64 value, not an out-of-range
           literal, because the annotation names the unsigned width. The boundary maps it to the
           component model's u64.")
  (needs  numeric-model)
  (input  UInt64.max)
  (output (: 18446744073709551615 UInt64)))

; --- Checked overflow per width (numeric-model.md #Overflow Is Defined, at each width) ----

(case "unsigned 8-bit addition that overflows its width traps"
  (doc    "`(+ (: 255 UInt8) (: 1 UInt8))` = 256, one past UInt8.max, so it overflows the checked UInt8
           range and MUST trap — the per-width analogue of `(+ Int64.max 1)`. Each fixed-width type is
           checked at its OWN range, not only at 64 bits; a naive lowering that computed in i32 and
           kept 256 would produce a value outside UInt8.")
  (needs  numeric-model)
  (input  (+ (: 255 UInt8) (: 1 UInt8)))
  (trap   "integer overflow"))

(case "unsigned subtraction below zero traps rather than wrapping"
  (doc    "`(- (: 0 UInt8) (: 1 UInt8))` would be -1, which UInt8 cannot represent (its range is
           0..=255), so the subtraction overflows the unsigned range and MUST trap. The unsigned-
           underflow companion of the overflow case: a checked unsigned type traps below zero, it does
           not wrap to 255.")
  (needs  numeric-model)
  (input  (- (: 0 UInt8) (: 1 UInt8)))
  (trap   "integer overflow"))

(case "signed 8-bit addition that overflows its width traps"
  (doc    "`(+ (: 127 Int8) (: 1 Int8))` = 128, one past Int8.max (127), so it overflows the checked
           Int8 range and MUST trap. Pins that the narrow SIGNED width is checked at its own boundary
           too — a wrap would give -128 (Int8.min), the classic signed-overflow wrong value.")
  (needs  numeric-model)
  (input  (+ (: 127 Int8) (: 1 Int8)))
  (trap   "integer overflow"))

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
  (needs  numeric-model)
  (input  (+ (: 1 UInt8) (: 2 Int32)))
  (error  CDZ0301))

(case "mixing signed and unsigned of the same width does not silently promote"
  (doc    "`(+ (: 1 Int32) (: 2 UInt32))` mixes Int32 and UInt32 — same width, different signedness,
           still distinct types — so it is rejected (CDZ0301). Signedness is not silently reinterpreted;
           the author must convert one side explicitly. Pins that no-promotion holds across signedness,
           not only across width.")
  (needs  numeric-model)
  (input  (+ (: 1 Int32) (: 2 UInt32)))
  (error  CDZ0301))

; --- Explicit conversions: T.of is checked, T.wrap truncates (options/numeric-model/) --------
; A conversion between integer types is always explicit (numeric-model.md #Integer Types Have Fixed
; Widths). `T.of x` is range-CHECKED — it traps when x does not fit T. `T.wrap x` TRUNCATES — it keeps
; the low bits under T's two's-complement representation. These systematize the seed's `Int.to-byte`
; (which is exactly UInt8.wrap) under one naming rule.

(case "a checked integer conversion that fits succeeds"
  (doc    "`(UInt8.of (: 200 Int32))` converts the Int32 200 to UInt8 — 200 is within 0..=255, so the
           checked conversion succeeds and yields the UInt8 200. Pins that T.of is the explicit,
           range-checked conversion the no-silent-promotion rule requires between widths.")
  (needs  numeric-model)
  (input  (UInt8.of (: 200 Int32)))
  (output (: 200 UInt8)))

(case "a checked integer conversion that does not fit traps"
  (doc    "`(UInt8.of (: 256 Int32))` converts 256 to UInt8, but 256 is outside 0..=255, so the CHECKED
           conversion MUST trap rather than silently truncate to 0 (numeric-model.md #Integer Types Have
           Fixed Widths — a checked conversion traps on an out-of-range value). Contrast UInt8.wrap
           below, which keeps the low bits.")
  (needs  numeric-model)
  (input  (UInt8.of (: 256 Int32)))
  (trap   "integer overflow"))

(case "a checked conversion of a negative value into an unsigned type traps"
  (doc    "`(UInt8.of (: -1 Int32))` converts -1 to UInt8, but UInt8 has no negative values, so the
           checked conversion MUST trap. Contrast `(UInt8.wrap -1)` = 255 below. Pins that T.of checks
           the sign boundary, not only the magnitude boundary.")
  (needs  numeric-model)
  (input  (UInt8.of (: -1 Int32)))
  (trap   "integer overflow"))

(case "a truncating conversion keeps the low bits rather than trapping"
  (doc    "`(UInt8.wrap (: 256 Int32))` = 0: the truncating conversion keeps the low 8 bits of 256
           (0x100 -> 0x00), so it yields 0 rather than trapping. This is exactly the seed's
           `Int.to-byte` on 256 (06-numeric #integer to byte wraps on overflow), now typed as UInt8.
           Pins T.wrap as the low-bits conversion distinct from the checked T.of.")
  (needs  numeric-model)
  (input  (UInt8.wrap (: 256 Int32)))
  (output (: 0 UInt8)))

(case "a truncating conversion of a negative value uses two's complement"
  (doc    "`(UInt8.wrap (: -1 Int32))` = 255: truncating keeps the low 8 bits of -1's two's-complement
           representation (all ones), so it yields 255 — exactly the seed's `(Int.to-byte -1)` = 255
           (06-numeric #negative integer to byte uses two's complement), now the typed UInt8.wrap.
           Pins that T.wrap reinterprets the low bits, where T.of would trap on the negative value.")
  (needs  numeric-model)
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
  (needs  numeric-model)
  (input  (< (: 0 UInt64) UInt64.max))
  (output (: true Bool)))

(case "an unsigned right shift fills with zeros, not the sign bit"
  (doc    "`(>> UInt8.max (: 1 UInt8))` = 127: a UInt8 right shift is LOGICAL (zero-filling), so shifting
           255 (0xFF) right by 1 yields 127 (0x7F). The signed `>>` on Int64 sign-extends (06-numeric
           #arithmetic right shift preserves the sign bit — `(>> -256 7)` = -2); on an UNSIGNED type the
           same operator is the logical shift (i64.shr_u). Pins that signedness selects arithmetic vs
           logical shift, the property signed/unsigned LEB128 both depend on.")
  (needs  numeric-model)
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
  (needs  numeric-model)
  (input  UInt32.max)
  (output (: 4294967295 UInt32)))

(case "a UInt32 addition that overflows the 32-bit width traps"
  (doc    "`(+ UInt32.max (: 1 UInt32))` = 2^32, one past UInt32.max, so it overflows the checked UInt32
           range and MUST trap — even though 2^32 fits comfortably in the i64 the seed would use. Pins
           that UInt32 is checked at 32 bits, not at 64: a compiler computing a section size must trap
           on a 32-bit overflow, not silently carry a value the wasm format cannot encode.")
  (needs  numeric-model)
  (input  (+ UInt32.max (: 1 UInt32)))
  (trap   "integer overflow"))

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
  (needs  numeric-model)
  (input  (: 200 (UInt 8)))
  (output (: 200 UInt8)))

(case "the width-indexed and aliased annotations of one value do not conflict"
  (doc    "`(: (: 5 (UInt 32)) UInt32)` annotates a value first as `(UInt 32)` then as `UInt32` — the
           same type under two names, so the annotations agree and the value is well-typed (NOT a CDZ0203
           annotation conflict). Pins the alias equivalence through the annotation-conflict checker: an
           alias and its expansion are interchangeable, never contradictory.")
  (needs  numeric-model)
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
  (needs  numeric-model)
  (input  (: 281474976710655 (UInt 48)))
  (output (: 281474976710655 (UInt 48))))

(case "an unusual-width value that overflows its computed range traps"
  (doc    "`(+ (: 281474976710655 (UInt 48)) (: 1 (UInt 48)))` = 2^48, one past `(UInt 48).max`, so it
           overflows the checked 48-bit range and MUST trap — the overflow bound is COMPUTED from N=48,
           not drawn from a fixed width table. Pins that a non-aliased width is checked at its own width
           exactly as an aliased one is; a naive lowering that only checked the 8/16/32/64 boundaries
           would carry 2^48 as a wrong value.")
  (needs  numeric-model)
  (input  (+ (: 281474976710655 (UInt 48)) (: 1 (UInt 48))))
  (trap   "integer overflow"))

(case "a truncating conversion to an unusual width keeps that width's low bits"
  (doc    "`((UInt 48).wrap (: -1 Int64))` = 281474976710655: the truncating conversion keeps the low 48
           bits of -1's two's-complement representation (48 ones) = 2^48 - 1. Pins that `T.wrap` computes
           its mask from N for a non-aliased width too — the low-N-bits rule is uniform across every
           width, aliased or not (options/numeric-model/ #Conversions are explicit).")
  (needs  numeric-model)
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
  (needs  numeric-model)
  (input  (: 0 (UInt 0)))
  (error  CDZ0302))

(case "an integer width above the 64-bit ceiling is rejected"
  (doc    "`(: 5 (UInt 65))` names a 65-bit integer, one past the 1..=64 ceiling — a width a single
           core-wasm register cannot hold. It is rejected at compile time (CDZ0302); a fixed-size integer
           wider than 64 bits is reserved to the opt-in big-integer layer, not the width-indexed
           constructor (options/numeric-model/ #Widths above 64 are reserved). Pins the upper boundary of
           the width constraint.")
  (needs  numeric-model)
  (input  (: 5 (UInt 65)))
  (error  CDZ0302))

(case "a wide fixed-size integer width is reserved, not yet a valid type"
  (doc    "`(: 5 (UInt 128))` names a 128-bit integer — beyond the 1..=64 register-width ceiling, so it is
           rejected (CDZ0302) today rather than silently accepted. The notation is reserved: a later
           optional increment MAY realize wide fixed-size integers as a multi-word representation and lift
           the ceiling, at which point `(UInt 128)` becomes valid with no surface-syntax change. Until
           then, more than 64 bits uses the big-integer type. Pins that the ceiling is a constraint, not a
           parse error.")
  (needs  numeric-model)
  (input  (: 5 (UInt 128)))
  (error  CDZ0302))

(case "an integer width from runtime data is rejected, keeping widths non-dependent"
  (doc    "`(UInt n)` with `n` a runtime function parameter puts a runtime value in a type-determining
           position, which the type system forbids (numeric-model.md #An Integer Type Is Indexed By A
           Compile-Time Width: the width MUST be resolved from a compile-time value and MUST NOT be
           determined by runtime data; type-system.md #Generics Are Type-Valued Parameters). It is
           rejected at compile time (CDZ0302), keeping the feature at indexed types over compile-time
           naturals rather than dependent types — no runtime value ever determines a type.")
  (needs  numeric-model)
  (input  (do
            (def (mk n) (: 5 (UInt n)))
            (def (main) (mk 8)) (export main)))
  (error  CDZ0302))
