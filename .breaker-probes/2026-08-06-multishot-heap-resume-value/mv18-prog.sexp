(do
  (effect Amb (op pick (-> Unit Int64)))
  (def (main (: n Int64))
    (handle Amb 0
      ((pick (u) s (+ (resume (+ n 1) s) (resume 2 s))))
      (match (Amb.pick) (v (+ (* 10 v) v)))))
  (export main))
