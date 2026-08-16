(case "nn2 a MAP with Rational VALUES as a map key normalizes the value leaves for the key hash"
  (input  (do
            (def (main (: n Int64))
              (do
                (def stored (Map.insert (Map.insert Map.empty 1 (Rational.of 1 2)) 2 (Rational.of n 3)))
                (def probe (Map.insert (Map.insert Map.empty 1 (Rational.of 3 6)) 2 (Rational.of (* n 3) 9)))
                (match (Map.lookup (Map.insert Map.empty stored 42) probe)
                  ((Some v) v) ((None _u) -1))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 42 Int64)))
