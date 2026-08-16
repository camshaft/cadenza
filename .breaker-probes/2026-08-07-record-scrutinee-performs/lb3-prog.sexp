(do
  (effect St (op next (-> Unit Int64)))
  (def (main (: n Int64))
    (handle St n
      ((next (u) s (resume s (+ s 1))))
      (let ((r (tuple 3 4)))
        (match r ((tuple x y) (+ (* 10 x) y))))))
  (export main))
