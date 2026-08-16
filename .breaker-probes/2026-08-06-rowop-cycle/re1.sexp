(case "re1 extend-without ROUND-TRIP restores key identity with the ORIGINAL record"
  (input  (do
            (def (main (: n Int64))
              (do
                (def r (record (a n) (b 2)))
                (def cycled (Record.without (Record.extend r #"c" 99) (c)))
                (+ (* 10 (match (Map.lookup (Map.insert Map.empty r 42) cycled) ((Some v) v) ((None _u) -1)))
                   (if (= cycled r) 1 0))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 421 Int64)))
