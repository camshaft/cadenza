(example
  (id "collatz-orbit")
  (name "Collatz orbit length")
  (theme "algorithms")
  (surface "sexpr")
  (source (do
  (def
    (collatz n steps)
    (if
      (= n 1)
      steps
      (if (= (% n 2) 0) (collatz (/ n 2) (+ steps 1)) (collatz (+ (* 3 n) 1) (+ steps 1)))))

  (def (main) (collatz 27 0))

  (export main)))
  (expected 111))
