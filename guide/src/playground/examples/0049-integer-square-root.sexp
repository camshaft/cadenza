(example
  (id "integer-square-root")
  (name "Integer square root")
  (theme "algorithms")
  (surface "sexpr")
  (source (do
  (def (isqrt-from n g) (if (> (* g g) n) (- g 1) (isqrt-from n (+ g 1))))

  (def (isqrt n) (isqrt-from n 1))

  (def (main) (isqrt 144))

  (export main)))
  (expected 12))
