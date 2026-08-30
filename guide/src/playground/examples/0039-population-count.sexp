(example
  (id "population-count")
  (name "Population count (bits)")
  (theme "algorithms")
  (surface "sexpr")
  (source (do
  (def (popcount n acc) (if (= n 0) acc (popcount (/ n 2) (+ acc (% n 2)))))

  (def (main) (popcount 2730 0))

  (export main)))
  (expected 6))
