; breaker probe B — the a5f7cfafb regression shape DOUBLED: an enclosing list-rest match binder
; (a genuine capture, must SHARE) read inside a fused arm which ALSO contains a nested match whose
; own binder must COPY/re-resolve; the fused match's scrutinee comes from an inlined callee.
; Hand-derived: xs=(list 10 20 30): c=10 t=(20 30); classify 10 → Big? 10>5 → (Big 10);
;   inner match (Big b) → b + first-of-t(20) = 30. main → 30.
; xs=(list 1): c=1 t=(); classify → (Small 1); (Small s) arm → s * 100 = 100 + 0 (t empty → 0) = 100.

(case "enclosing rest-capture read inside a fused arm holding a nested match"
  (input  (do
            (type Sz (Big Int64) (Small Int64))
            (def (classify x) (if (> x 5) (Big x) (Small x)))
            (def (hd ys) (match ys ((list h .. r) h) ((list) 0)))
            (def (main (: xs (List Int64)))
              (match xs
                ((list c .. t)
                  (match (classify c)
                    ((Big b) (+ b (hd t)))
                    ((Small s) (+ (* s 100) (hd t)))))
                ((list) -1)))
            (export main)))
  (call   main (list 10 20 30)) (output (: 30 Int64))
  (call   main (list 1)) (output (: 100 Int64)))
