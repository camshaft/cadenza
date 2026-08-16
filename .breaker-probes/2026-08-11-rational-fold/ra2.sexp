(case "ra2 runtime Rationals as Map keys after arithmetic derivation (1/2 via three routes)"
  (input  (do
            (def (main (: n Int64))
              (do
                (def m (Map.insert Map.empty (Rational.of 1 2) 42))
                (+ (* 100 (match (Map.lookup m (+ (Rational.of 1 6) (Rational.of 1 3))) ((Some v) (/ v 10)) ((None _u) -1)))
                   (+ (* 10 (match (Map.lookup m (* (Rational.of 2 3) (Rational.of 3 4))) ((Some v) (/ v 10)) ((None _u) -1)))
                      (match (Map.lookup m (Rational.of n (* n 2))) ((Some v) (/ v 10)) ((None _u) -1))))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 444 Int64)))
