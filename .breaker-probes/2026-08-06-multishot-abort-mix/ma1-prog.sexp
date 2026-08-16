(do
  (effect Amb (op pick (-> Unit Int64)))
  (effect Bail (op stop (-> Int64 Int64)))
  (def (main (: n Int64))
    (handle Bail 0
      ((stop (v) s (* v 3)))
      (+ 1000
         (handle Amb 0
           ((pick (u) s (+ (resume 1 s) (resume 2 s))))
           (+ (* 10 (Amb.pick)) (Bail.stop (Amb.pick)))))))
  (export main))
