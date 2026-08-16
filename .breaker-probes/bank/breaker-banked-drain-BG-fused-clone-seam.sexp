; breaker probe C — the identity-check stress: a fused match (scrutinee = call result) whose arm
; body calls a small helper that ITSELF matches on a sum built FROM the fused binder (the helper
; inlines → its match's SumPayload binder is β-pinned with scrutinee ≠ fused_scrut but WITHIN the
; cloned arm), while the same arm ALSO reads an enclosing match binder (a genuine capture to SHARE).
; Hand-derived: main 7: mk 7 → 7>5 → (Hi 7). outer match xs=(list 3 4): c=3.
;   fused arm (Hi h): helper (flip (Hi h)) → matches (Hi v) → (Lo (* v 10)) = (Lo 70);
;   then match that: (Lo l) → l + c = 70 + 3 = 73.
; main 2: mk 2 → (Lo 2). arm (Lo g): g + c = 2 + 3 = 5.

(case "inlined helper match on a sum built from the fused binder plus an enclosing capture"
  (input  (do
            (type T (Hi Int64) (Lo Int64))
            (def (mk x) (if (> x 5) (Hi x) (Lo x)))
            (def (flip a) (match a ((Hi v) (Lo (* v 10))) ((Lo w) (Hi (* w 10)))))
            (def (main (: n Int64))
              (let ((xs (list 3 4)))
                (match xs
                  ((list c .. t)
                    (match (mk n)
                      ((Hi h) (match (flip (Hi h))
                                ((Lo l) (+ l c))
                                ((Hi z) (- 0 z))))
                      ((Lo g) (+ g c))))
                  ((list) -1))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 73 Int64))
  (call   main (: 2 Int64)) (output (: 5 Int64)))
