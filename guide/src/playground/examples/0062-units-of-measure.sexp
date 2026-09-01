(example
  (id "units-of-measure")
  (name "Units of measure")
  (theme "numbers")
  (surface "sexpr")
  (source (do
  (def (main) (Qty.of 5.0 (Unit.prefix kilo (Unit.base #"meter"))))

  (export main)))
  (expected (: (Qty.of 5000.0 (Unit.base #"meter")) (Qty Float64 (Unit.base #"meter")))))
