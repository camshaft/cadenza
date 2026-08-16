; breaker probe A — match-of-call-of-match chain: two fusion opportunities stacked.
; Expected hand-derived: main 5 → step (Ok 5) = Ok 6 → double → Ok 12 → step → Ok 13 → 13.
; main -3 → step (Err path never taken; Ok -3) = Ok -2 → Ok -4 → Ok -3 → -3.

(case "double fused match chain through a shared callee"
  (input  (do
            (type R (Okv Int64) (Errv Int64))
            (def (step a) (match a ((Okv v) (Okv (+ v 1))) ((Errv e) (Errv e))))
            (def (main (: n Int64))
              (match (step (match (step (Okv n))
                             ((Okv v) (Okv (* v 2)))
                             ((Errv e) (Errv e))))
                ((Okv v) v)
                ((Errv e) e)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 13 Int64))
  (call   main (: -3 Int64)) (output (: -3 Int64)))
