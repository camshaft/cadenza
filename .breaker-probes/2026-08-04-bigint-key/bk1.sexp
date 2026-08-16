(case "bk1 a BigInt computed across the limb boundary keys a Map like its literal twin"
  (input  (do
            (def (main (: n Int64))
              (do
                (def grown (* (BigInt.of 9223372036854775807) (BigInt.of (+ n 1))))
                (match (Map.lookup (Map.insert Map.empty 18446744073709551614N 7) grown)
                  ((Some v) v) ((None _u) -1))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 7 Int64))
  (call   main (: 2 Int64)) (output (: -1 Int64)))
