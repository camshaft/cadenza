(example
  (id "rational-parts")
  (name "Rational numerator & denominator")
  (theme "numbers")
  (surface "sexpr")
  (source (do
  (pragma default-fraction Rational)

  (def (total) (+ (+ (/ 1 2) (/ 1 3)) (/ 1 12)))

  (def (main) #tuple(((. Rational numerator) (total)) ((. Rational denominator) (total))))

  (export main)))
  (expected (: #tuple(11 12) (Tuple BigInt BigInt))))
