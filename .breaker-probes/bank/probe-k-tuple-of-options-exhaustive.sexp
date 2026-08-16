; breaker probe K — exhaustiveness over a TUPLE-OF-SUMS scrutinee: all four Some/None combinations
; present, NO wildcard. The tuple-of-bools saturation pin (Inc-20) covers Bool; a sum-payload
; product must also be recognized as total (2x2 variants). A checker that only saturates literals
; would spuriously CDZ0210; a dispatcher that mis-pairs arms miscomputes.
; Hand-derived: (2,3) → (Some 2, Some 3) → 2+3=5; (1,0) → (Some 1, None) → 10; (0,3) → (None, Some 3)
;   → 300; (0,0) → (None,None) → -1.

(case "a match on a tuple of two Options with all four variant pairs is exhaustive"
  (input  (do
            (def (mk (: v Int64)) (if (> v 0) (Some v) (None)))
            (def (main (: p Int64) (: q Int64))
              (match (tuple (mk p) (mk q))
                ((tuple (Some x) (Some y)) (+ x y))
                ((tuple (Some x) (None u)) (* x 10))
                ((tuple (None u) (Some y)) (* y 100))
                ((tuple (None u) (None v)) -1)))
            (export main)))
  (call   main (: 2 Int64) (: 3 Int64)) (output (: 5 Int64))
  (call   main (: 1 Int64) (: 0 Int64)) (output (: 10 Int64))
  (call   main (: 0 Int64) (: 3 Int64)) (output (: 300 Int64))
  (call   main (: 0 Int64) (: 0 Int64)) (output (: -1 Int64)))
