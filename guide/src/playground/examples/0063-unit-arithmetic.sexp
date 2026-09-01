(example
  (id "unit-arithmetic")
  (name "Adding mixed units")
  (theme "numbers")
  (surface "sexpr")
  (source (do
  (def (main) (Qty.value (+ (Qty.of 1.0 (Unit.of #"km")) (Qty.of 500.0 (Unit.of #"m")))))

  (export main)))
  (expected 1500.0))
