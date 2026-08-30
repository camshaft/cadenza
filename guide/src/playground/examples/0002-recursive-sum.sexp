(example
  (id "recursive-sum")
  (name "A recursive sum")
  (theme "basics")
  (surface "sexpr")
  (source (do
  (def (sm n) (if (= n 0) 0 (+ n (sm (- n 1)))))

  (def (main) (sm 5))

  (export main)))
  (expected 15))
