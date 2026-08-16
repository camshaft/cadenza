(case "bk3 NEGATIVE limb-boundary keys: -(2^64) computed two ways is one Set element"
  (input  (do
            (def (main (: n Int64))
              (do
                (def a (* (BigInt.of -9223372036854775807) (BigInt.of (* n 2))))
                (def b (* (BigInt.of 9223372036854775807) (BigInt.of (- 0 (* n 2)))))
                (Set.len (Set.of (list a b)))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 1 Int64)))
