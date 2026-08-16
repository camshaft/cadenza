; breaker probe Y — a BigInt LITERAL as a match-arm PATTERN over a runtime multi-limb scrutinee:
; the literal-pattern equality must run BigInt content-eq (limb-array compare), not a truncated
; i64 compare. A multi-limb value whose LOW LIMB equals a small literal is the trap: 2^64+5 has
; low limb 5 — an i64-truncating pattern compare would match the (5N) arm.
; Hand-derived: mk k = (* (+ 18446744073709551615N 1N) k) + 5N = k*2^64 + 5.
;   k=1 → 2^64+5: arm (5N)? NO (multi-limb ≠ 5) → arm catch-all → 0... encode: match → (5N → 1) (_ → 0).
;   k=0 → 0*2^64+5 = 5N exactly → 1.
;   main = 10*(mk 0 match) + (mk 1 match) = 10*1 + 0 = 10.

(case "a BigInt literal pattern compares full content, not a truncated low limb"
  (input  (do
            (def (mk (: k Int64)) (+ (* (+ 18446744073709551615N 1N) (BigInt.of k)) 5N))
            (def (is5 b) (match b ((5N) 1) (_ 0)))
            (def (main)
              (+ (* 10 (is5 (mk 0))) (is5 (mk 1))))
            (export main)))
  (output (: 10 Int64)))
