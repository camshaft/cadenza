; breaker probe P — a CLOSURE built inside a fused-match arm CAPTURING the arm's SumPayload binder,
; invoked AFTER the match completes. Fusion clones the arm into each branch of the callee's `if`;
; the closure's env must capture the CLONED binder's value (re-resolved against the branch value),
; not the original now-detached switch — the deferred-read twin of the direct-read probes (BG).
; Hand-derived: mk 7 → Hi 7 → arm builds f = fn(d) 10*h + d, captured h=7; g = fn(d) 100*w+d not built.
;   after match: (f 3) = 73. k=2: mk→Lo 2 → g = fn(d) 100*2+d → (g 3) = 203.

(case "a closure built in a fused-match arm captures the payload binder and reads it after the match"
  (input  (do
            (type Sz (Hi Int64) (Lo Int64))
            (def (mk x) (if (> x 5) (Hi x) (Lo x)))
            (def (main (: k Int64))
              (let ((f (match (mk k)
                         ((Hi h) (fn ((: d Int64)) (+ (* 10 h) d)))
                         ((Lo w) (fn ((: d Int64)) (+ (* 100 w) d))))))
                (f 3)))
            (export main)))
  (call   main (: 7 Int64)) (output (: 73 Int64))
  (call   main (: 2 Int64)) (output (: 203 Int64)))
