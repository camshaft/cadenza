(example
  (id "count-primes")
  (name "Count primes (trial division)")
  (theme "algorithms")
  (surface "sexpr")
  (source (do
  (def (isprime-from d n) (if (> (* d d) n) true (if (= (% n d) 0) false (isprime-from (+ d 1) n))))

  (def (isprime n) (if (< n 2) false (isprime-from 2 n)))

  (def (count k n acc) (if (> k n) acc (count (+ k 1) n (if (isprime k) (+ acc 1) acc))))

  (def (main) (count 2 100 0))

  (export main)))
  (expected 25))
