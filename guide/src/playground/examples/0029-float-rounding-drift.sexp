(example
  (id "float-rounding-drift")
  (name "Float rounding drift")
  (theme "numbers")
  (surface "sexpr")
  (source (do
  (def (add-tenths (: n Int64) (: acc Float64)) (if (= n 0) acc (add-tenths (- n 1) (+ acc 0.1))))

  (def (main) (add-tenths 10 0.0))

  (export main)))
  (expected 0.9999999999999999))
