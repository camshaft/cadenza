(do
  (effect St (op next (-> Unit Int64)))
  (def (fold3 (: f (-> Int64 Int64 Int64)) (: a Int64) (: b Int64) (: c Int64)) (f (f a b) c))
  (def (main (: n Int64))
    (handle St n
      ((next (u) s (resume s (+ s 1))))
      (let ((w (St.next)))
        (fold3 (fn ((: x Int64) (: y Int64)) (+ (* x w) y)) 1 2 3))))
  (export main))
