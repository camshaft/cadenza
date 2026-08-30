(example
  (id "function-composition")
  (name "Function composition")
  (theme "basics")
  (surface "sexpr")
  (source (do
  (def (compose f g) (fn (x) (f (g x))))

  (def (double x) (* x 2))

  (def (inc x) (+ x 1))

  (def (main) ((compose double inc) 20))

  (export main)))
  (expected 42))
