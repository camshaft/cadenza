(example
  (id "gcd-lcm-euclid")
  (name "GCD and LCM (Euclid)")
  (theme "algorithms")
  (surface "sexpr")
  (source (do
  (def (gcd a b) (if (= b 0) a (gcd b (% a b))))

  (def (lcm a b) (/ (* a b) (gcd a b)))

  (def (main) (lcm 12 18))

  (export main)))
  (expected 36))
