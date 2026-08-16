(case "comparing Bytes with String is a type mismatch even though both now offer orders"
  (doc    "The cross-type guard the #1120 Bytes blessing must NOT relax: Bytes and String EACH offer a
           total order now, but `(< bytes \"hello\")` is still a CDZ0203 type mismatch — an order is
           offered per-TYPE, never across types (no implicit byte-view of a String or text-view of
           Bytes at the comparison boundary; the text/bytes distinction the two decode/encode pins
           guard elsewhere). A blessing implemented as 'both sides byte-like → compare bytes' would
           wrongly accept this; the reject pins the boundary.")
  (input  (do
            (def (main (: x UInt8))
              (if (< (Bytes.of (list x)) "hello") 1 0))
            (export main)))
  (call   main (: 5 UInt8))
  (error  CDZ0203))

(case "empty Bytes orders before every non-empty Bytes under the blessed order"
  (doc    "The degenerate boundary of the blessed lexicographic order: the EMPTY byte sequence is a
           prefix of everything, so `[] < [x]` is true for every x — including x=0 (10s digit: [] <
           [0], the face a length-first-then-content order and a content-first order agree on but a
           'skip empty operands' shortcut breaks) — and `[] < []` is false (irreflexive, 1s digit) →
           10. Runtime x defeats folding.")
  (input  (do
            (def (main (: x UInt8))
              (+ (* 10 (if (< (Bytes.of (list)) (Bytes.of (list x))) 1 0))
                 (if (< (Bytes.of (list)) (Bytes.of (list))) 1 0)))
            (export main)))
  (call   main (: 0 UInt8)) (output (: 10 Int64)))
