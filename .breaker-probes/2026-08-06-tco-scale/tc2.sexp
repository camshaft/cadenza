(case "tc2 tail recursion threading a HEAP accumulator at depth 50000 (Perceus at scale)"
  (input  (do
            (def (go (: i Int64) (: acc (List Int64)))
              (if (= i 0) acc (go (- i 1) (List.update acc 0 i))))
            (def (main (: n Int64))
              (match (List.at (go n (list 0 99)) 0) ((Some v) v) ((None _u) -1)))
            (export main)))
  (call   main (: 50000 Int64)) (output (: 1 Int64)))
