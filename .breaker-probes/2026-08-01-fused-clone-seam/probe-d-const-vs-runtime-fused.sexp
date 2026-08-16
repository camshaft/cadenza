; breaker probe D — const-vs-runtime divergence over the fused-match seam: the SAME chain
; computed with a compile-time constant (const-foldable end-to-end) and threaded via a runtime
; param must agree.
; Hand-derived: step (Okv 5) = Okv 6; double → Okv 12; step → Okv 13 → 13. Both must be 13; diff = 0.

(case "fused chain result agrees between const-fold and runtime threading"
  (input  (do
            (type R (Okv Int64) (Errv Int64))
            (def (step a) (match a ((Okv v) (Okv (+ v 1))) ((Errv e) (Errv e))))
            (def (chain m)
              (match (step (match (step (Okv m))
                             ((Okv v) (Okv (* v 2)))
                             ((Errv e) (Errv e))))
                ((Okv v) v)
                ((Errv e) e)))
            (def (main (: n Int64)) (- (chain n) (chain 5)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 0 Int64))
  (call   main (: 9 Int64)) (output (: 8 Int64)))
