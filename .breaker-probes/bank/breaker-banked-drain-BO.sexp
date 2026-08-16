; breaker probe T — ABORTIVE perform inside a FUSED-match arm: the match scrutinee is a call
; result (fusion candidate); one arm's body performs an abortive Bail carrying the payload binder,
; abandoning the WHOLE handle body (including the pending outer addition); the other arm returns
; normally. The fused clone must keep the abort's br-out-of-block correct in BOTH branch copies,
; and the payload binder must flow into the abort argument.
; Hand-derived: k=7: mk→Hi 7 → arm performs (Bail.bail (* h 10)) → handle value = 70 (the +1000
;   pending addition is abandoned). k=2: mk→Lo 2 → normal arm → w*100 = 200 → + 1000 = 1200.

(case "an abortive perform in a fused-match arm carries the payload binder out and abandons the rest"
  (input  (do
            (effect Bail (op bail (-> Int64 Int64)))
            (type Sz (Hi Int64) (Lo Int64))
            (def (mk x) (if (> x 5) (Hi x) (Lo x)))
            (def (main (: k Int64))
              (handle Bail 0 ((bail (n) s n))
                (+ (match (mk k)
                     ((Hi h) (Bail.bail (* h 10)))
                     ((Lo w) (* w 100)))
                   1000)))
            (export main)))
  (call   main (: 7 Int64)) (output (: 70 Int64))
  (call   main (: 2 Int64)) (output (: 1200 Int64)))
