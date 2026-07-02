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

(case "a floating-point literal"
  (input  3.5)
  (output (: 3.5 Float64)))

(case "negative zero is distinct in the canonical value form"
  (doc    "The canonical value form distinguishes -0.0 from 0.0; a canonical NaN is separate.")
  (input  -0.0)
  (output (: -0.0 Float64)))

(case "the boolean literals"
  (input  true)
  (output (: true Bool)))

(case "a string literal"
  (input  "hello")
  (output (: "hello" String)))

(case "a string literal is normalized to the canonical text form"
  (doc    "Stored in the pinned text normalization form (options/hashing-and-encoding/),
           so two literals differing only in normalization are one value.")
  (input  "café")
  (output (: "café" String)))
