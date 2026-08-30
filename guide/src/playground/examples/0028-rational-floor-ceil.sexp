(example
  (id "rational-floor-ceil")
  (name "Rational floor & ceil")
  (theme "numbers")
  (surface "sexpr")
  (source (do
  (pragma default-fraction Rational)

  (def
    (main)
    #tuple(((. Rational floor) (/ 7 2))
      ((. Rational ceil) (/ 7 2))
      ((. Rational floor) (/ -7 2))
      ((. Rational ceil) (/ -7 2))))

  (export main)))
  (expected (: #tuple(3 4 -4 -3) (Tuple Int64 Int64 Int64 Int64))))
