(do
  (effect Amb (op pick (-> Unit Int64)))
  (def (main (: n Int64))
    (handle Amb n
      ((pick (u) s (+ (resume (+ s 1) s) (resume 2 s))))
      (let ((x (Amb.pick)))
        (+ (* 10 x) x))))
  (export main))
