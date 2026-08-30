(example
  (id "exact-rational-arithmetic")
  (name "Exact rational arithmetic")
  (theme "numbers")
  (surface "sexpr")
  (source (do
  (pragma default-fraction Rational)

  (def (sum) (+ (+ (/ 1 2) (/ 1 3)) (/ 1 6)))

  (def (main) (sum))

  (export main)))
  (expected (: 1/1 Rational)))
