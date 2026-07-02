; Numeric model — witnesses the behavioral requirements of numeric-model.md:
; no implicit promotion, defined overflow, exact arithmetic, deterministic float.
; A rejected program records its diagnostic code; a runtime halt records a trap.

(case "arithmetic within one integer type"
  (input  (+ 2 3))
  (output (: 5 Int64)))

(case "mixing two numeric types without an explicit conversion is rejected"
  (doc    "Witnesses numeric-model.md #Numeric Types Do Not Silently Promote — there is
           no implicit widening from Int64 to Float64.")
  (input  (+ 2 2.0))
  (error  CDZ0301))

(case "an explicit conversion makes the operation well-typed"
  (input  (+ (Float64.of-int 1) 2.0))
  (output (: 3.0 Float64)))

(case "overflow of the default integer traps deterministically"
  (doc    "Witnesses numeric-model.md #Overflow Is Defined with the checked-and-trapping
           default pinned in options/numeric-model/.")
  (input  (+ Int64.max 1))
  (trap   "integer overflow"))

(case "wrapping arithmetic uses the distinct wrapping type"
  (input  (+% Wrapping64.max 1))
  (output (: -9223372036854775808 Wrapping64)))

(case "exact rational arithmetic is exact and normalized"
  (doc    "Witnesses numeric-model.md #Exact Arithmetic Is Exact; the result is reduced
           to lowest terms per options/numeric-model/.")
  (input  (+ (Rational.of 1 3) (Rational.of 1 6)))
  (output (: (Rational.of 1 2) Rational)))

(case "floating-point uses the fixed rounding mode"
  (doc    "The round-to-nearest-even sum under the pinned deterministic float mode
           (contracts/determinism-and-fuel.md); byte-identical on every conforming runtime.")
  (input  (+ 0.1 0.2))
  (output (: 0.30000000000000004 Float64)))
