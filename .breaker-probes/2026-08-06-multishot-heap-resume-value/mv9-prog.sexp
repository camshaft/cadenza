(do
  (effect Amb (op pick (-> Unit Int64)))
  (def (main (: n Int64))
    (handle Amb 0
      ((pick (u) s (+ (resume (+ n 1) s) (resume 2 s))))
      (+ (* 10 (Amb.pick)) (Amb.pick))))
  (export main))
