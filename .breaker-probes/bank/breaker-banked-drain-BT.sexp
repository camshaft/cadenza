; breaker probe Y2 — the = twin of the literal-pattern probe: runtime multi-limb BigInt equality
; against a small literal whose value equals the LOW LIMB. 2^64+5 = 5N must be FALSE.
; k=0 → 5N = 5N true → 1; k=1 → 2^64+5 = 5N false → 0. main = 10*1 + 0 = 10.

(case "multi-limb BigInt equality against a low-limb-equal small literal is false"
  (input  (do
            (def (mk (: k Int64)) (+ (* (+ 18446744073709551615N 1N) (BigInt.of k)) 5N))
            (def (main)
              (+ (* 10 (if (= (mk 0) 5N) 1 0)) (if (= (mk 1) 5N) 1 0)))
            (export main)))
  (output (: 10 Int64)))
