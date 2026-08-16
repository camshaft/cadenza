(do
  (effect A (op geta (-> Unit Int64)))
  (effect B (op getb (-> Unit Int64)))
  (effect In (op go (-> Unit Int64)))
  (def (main (: n Int64))
    (handle A n
      ((geta (u) s (resume s (+ s 1))))
      (handle B 100
        ((getb (u) t (resume t (+ t 10))))
        (handle In 0
          ((go (u) w (resume (+ (A.geta) (B.getb)) w)))
          (+ (* 1000 (In.go)) (In.go))))))
  (export main))
