(case "bk2 a BigInt SHRUNK back across the limb boundary dedupes with its small twin"
  (input  (do
            (def (main (: n Int64))
              (do
                (def big (* (BigInt.of 9223372036854775807) (BigInt.of n)))
                (def back (- (+ big (BigInt.of 5)) big))
                (Set.len (Set.of (list back (BigInt.of 5) (BigInt.of 6))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 2 Int64)))
