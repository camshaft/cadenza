(case "nn1 a LIST of Rationals as a map key normalizes every element for the key hash"
  (input  (do
            (def (main (: n Int64))
              (do
                (def stored (list (Rational.of 1 2) (Rational.of n 3) (Rational.of 3 4)))
                (def probe (list (Rational.of 2 4) (Rational.of (* n 2) 6) (Rational.of 9 12)))
                (match (Map.lookup (Map.insert Map.empty stored 42) probe)
                  ((Some v) v) ((None _u) -1))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 42 Int64)))
