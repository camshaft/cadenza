(case "nn3 a Set of BigInts spanning limbs as a map key matches its arithmetic twin"
  (input  (do
            (def big (BigInt.of 9223372036854775807))
            (def (main (: n Int64))
              (do
                (def stored (Set.of (list (* big (BigInt.of 2)) (BigInt.of n))))
                (def probe (Set.of (list (BigInt.of n) (* (BigInt.of 2) big))))
                (match (Map.lookup (Map.insert Map.empty stored 42) probe)
                  ((Some v) v) ((None _u) -1))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 42 Int64)))
