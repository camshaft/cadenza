(example
  (id "mutual-recursion")
  (name "Mutual recursion (even & odd)")
  (theme "algorithms")
  (surface "sexpr")
  (source (do
  (def (is-even n) (if (= n 0) true (is-odd (- n 1))))

  (def (is-odd n) (if (= n 0) false (is-even (- n 1))))

  (def
    (count-evens n i acc)
    (if (= i n) acc (count-evens n (+ i 1) (if (is-even i) (+ acc 1) acc))))

  (def (main) (count-evens 10 0 0))

  (export main)))
  (expected 5))
