(do
  (effect Amb (op pick (-> Unit Int64)))
  (def (helper (: k Int64)) (+ k 1))
  (def (main (: n Int64))
    (handle Amb 0
      ((pick (u) s (+ (resume (+ n 1) s) (resume 2 s))))
      (let ((x (Amb.pick)))
        (helper x))))
  (export main))
