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

(case "overflow of the default integer traps deterministically"
  (doc    "Witnesses numeric-model.md #Overflow Is Defined with the checked-and-trapping default
           pinned in options/numeric-model/. The seed realizes checked Int64.")
  (input  (+ Int64.max 1))
  (trap   "integer overflow"))

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

(case "exact rational arithmetic is exact and normalized"
  (doc    "Witnesses numeric-model.md #Exact Arithmetic Is Exact; reduced to lowest terms per
           options/numeric-model/. The output is the canonical rational value form 1/2.")
  (needs numeric-model)
  (input  (+ (Rational.of 1 3) (Rational.of 1 6)))
  (output (: 1/2 Rational)))

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

(case "multiplication"
  (input  (* 6 7))
  (output (: 42 Int64)))

(case "integer division truncates toward zero"
  (input  (/ 7 2))
  (output (: 3 Int64)))

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
  (doc    "The contrast to the case above: `(/ -9223372036854775808 -1)` = +2^63, which is out of the
           Int64 range, so it overflows and traps (numeric-model.md #Overflow Is Defined) — division
           DOES form the quotient, so it overflows where modulo does not. Pins that the trap belongs to
           `/`, not to `%`, at Int64.min / -1.")
  (input  (/ -9223372036854775808 -1))
  (trap   "integer overflow"))

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

(case "arithmetic right shift"
  (doc    "The compiler needs right shift for LEB128 encoding: (>> n 7) shifts n right by 7 bits,
           extracting the next group. Arithmetic shift preserves sign for signed LEB128.")
  (input  (>> 256 7))
  (output (: 2 Int64)))

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
           multiplication by a power of two, so `(<< 4611686018427387904 1)` =
           4611686018427387904 * 2 = 2^63, which overflows the checked Int64 default and traps —
           exactly as the sibling `(* 4611686018427387904 2)` already does. The compiler MUST NOT
           emit a shift that silently wraps to -9223372036854775808 (numeric-model.md
           §\"The compiler MUST NOT emit an integer operation whose overflow behavior is
           undefined\").")
  (input  (<< 4611686018427387904 1))
  (trap   "integer overflow"))

(case "a left shift by the bit width or more traps rather than wrapping"
  (doc    "`1 << 64` is 2^64, which overflows Int64. A shift count equal to or beyond the type's
           bit width is out of range; the result MUST be the defined outcome fixed by the numeric
           model (a trap, matching the checked default) rather than wasm's masked i64.shl, which
           treats a shift of 64 as a shift of 0 and answers 1. Witnesses #Overflow Is Defined.")
  (input  (<< 1 64))
  (trap   "integer overflow"))

(case "a negative shift count traps rather than masking"
  (doc    "A negative shift count has no defined value. wasm's i64.shl masks it into 0..63 — -1
           becomes 63 — silently answering `1 << 63` = -9223372036854775808. The compiler MUST NOT
           emit an operation whose behavior is undefined (numeric-model.md #Overflow Is Defined); a
           negative shift count is out of range and traps rather than wrapping to a value.")
  (input  (<< 1 -1))
  (trap   "integer overflow"))

; The three shift cases above use CONSTANT operands (folded at compile time). The SAME shift with
; RUNTIME operands (function parameters) must trap identically — a shift is a shift regardless of
; whether its operands are compile-time-known. The overflow/out-of-range-count check must be emitted
; on the RUNTIME shift path (a guard before wasm's masking `i64.shl`/`i64.shr_s`), not only in the
; constant folder. These runtime companions pin that the two paths AGREE: the seed's const path traps
; (above) but its runtime path silently MASKS the count (mod 64) and WRAPS on overflow — a const-vs-
; runtime divergence and a wrong value for a runtime out-of-range shift.

(case "a runtime left shift by the bit width or more traps"
  (doc    "The runtime companion of `(<< 1 64)`: with the shift emitted for parameter operands, a
           count equal to the bit width MUST trap (numeric-model.md #Overflow Is Defined), exactly as
           the constant fold does. The seed's runtime path masks the count (64 mod 64 = 0) and answers
           1 — a wrong value, and a divergence from the constant case above which traps.")
  (input  (module m
            (def (sh a b) (<< a b))
            (def (main) (sh 1 64))))
  (trap   "integer overflow"))

(case "a runtime overflowing left shift traps"
  (doc    "The runtime companion of the overflowing left shift: `(sh 4611686018427387904 1)` =
           2^62 << 1 = 2^63, which overflows Int64 and MUST trap, exactly as the constant
           `(<< 4611686018427387904 1)` does. The seed's runtime path silently wraps to
           -9223372036854775808. Pins that the runtime shift path enforces #Overflow Is Defined too.")
  (input  (module m
            (def (sh a b) (<< a b))
            (def (main) (sh 4611686018427387904 1))))
  (trap   "integer overflow"))

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
