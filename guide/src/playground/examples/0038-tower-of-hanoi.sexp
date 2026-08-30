(example
  (id "tower-of-hanoi")
  (name "Tower of Hanoi (move count)")
  (theme "algorithms")
  (surface "sexpr")
  (source (do
  (def (hanoi n) (if (= n 0) 0 (+ (+ (hanoi (- n 1)) 1) (hanoi (- n 1)))))

  (def (main) (hanoi 10))

  (export main)))
  (expected 1023))
