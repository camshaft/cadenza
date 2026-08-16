; breaker probe S — PURE guards over a FUSED match (call-result scrutinee) whose guards read BOTH
; the arm's SumPayload binder AND an enclosing let-capture: the fused arm-clone must keep the
; guard's binder reads coherent (guard reads h from the clone, lim from the enclosing frame).
; Hand-derived: lim=8. k=9: mk→Hi 9; arm1 guard (> h lim): 9>8 YES → 90.
;   k=7: mk→Hi 7; arm1 guard 7>8 NO → arm2 (Hi h2) unguarded → h2 = 7.
;   k=2: mk→Lo 2; arm3 (Lo w) guard (< w lim): 2<8 YES → w*100 = 200.

(case "pure guards on fused-match arms read the payload binder and an enclosing capture"
  (input  (do
            (type Sz (Hi Int64) (Lo Int64))
            (def (mk x) (if (> x 5) (Hi x) (Lo x)))
            (def (main (: k Int64))
              (let ((lim 8))
                (match (mk k)
                  ((guard (Hi h) (> h lim)) (* h 10))
                  ((Hi h2) h2)
                  ((guard (Lo w) (< w lim)) (* w 100))
                  (_ -999))))
            (export main)))
  (call   main (: 9 Int64)) (output (: 90 Int64))
  (call   main (: 7 Int64)) (output (: 7 Int64))
  (call   main (: 2 Int64)) (output (: 200 Int64)))
