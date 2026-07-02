; Numeric model — witnesses numeric-model.md. The interpreter terminal clause is the oracle; inline
; (compiler …) marks where a typed generation rejects instead of running, and (needs numeric-model)
; marks the extended numerics a later generation realizes (the seed realizes only the checked Int64
; core and Float64 literals/equality — options/realized-capability-set/). A rejected program's code is
; from options/diagnostics-schema/.

(case "arithmetic within one integer type"
  (input  (+ 2 3))
  (output (: 5 Int64)))

(case "mixing two numeric types without an explicit conversion does not silently promote"
  (doc    "Witnesses numeric-model.md #Numeric Types Do Not Silently Promote. The dynamic interpreter
           (oracle) does not coerce Int64 to Float64 — with no static type to reject, it traps at
           runtime. A typed generation instead rejects the program at compile time (CDZ0301); it never
           silently promotes either way.")
  (input    (+ 2 2.0))
  (trap     "numeric type mismatch")
  (compiler (error CDZ0301)))

(case "overflow of the default integer traps deterministically"
  (doc    "Witnesses numeric-model.md #Overflow Is Defined with the checked-and-trapping default
           pinned in options/numeric-model/. The seed realizes checked Int64.")
  (input  (+ Int64.max 1))
  (trap   "integer overflow"))

(case "an explicit conversion makes the operation well-typed"
  (doc    "Witnesses numeric-model.md #Exact Arithmetic Is Exact (2nd sentence): the conversion
           Float64.of-int is written explicitly. Needs the numeric-model capability (conversions +
           float arithmetic), which the seed does not realize.")
  (needs  numeric-model)
  (input  (+ (Float64.of-int 1) 2.0))
  (output (: 3.0 Float64)))

(case "wrapping arithmetic uses the distinct wrapping type"
  (doc    "Witnesses numeric-model.md #Overflow Is Defined via the distinct wrapping type.")
  (needs  numeric-model)
  (input  (+% Wrapping64.max 1))
  (output (: -9223372036854775808 Wrapping64)))

(case "exact rational arithmetic is exact and normalized"
  (doc    "Witnesses numeric-model.md #Exact Arithmetic Is Exact; reduced to lowest terms per
           options/numeric-model/. The output is the canonical rational value form 1/2.")
  (needs  numeric-model)
  (input  (+ (Rational.of 1 3) (Rational.of 1 6)))
  (output (: 1/2 Rational)))

(case "floating-point uses the fixed rounding mode"
  (doc    "The round-to-nearest-even sum under the pinned deterministic float mode
           (contracts/determinism-and-fuel.md); byte-identical on every conforming runtime. The seed
           realizes Float64 literals and equality, not floating-point arithmetic.")
  (needs  numeric-model)
  (input  (+ 0.1 0.2))
  (output (: 0.30000000000000004 Float64)))
