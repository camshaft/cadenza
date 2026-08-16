(case "rc1 a 20-cycle extend-without churn on one record restores key identity"
  (input  (do
            (def (churn (: i Int64) (: r (Record (a Int64) (b Int64))))
              (if (= i 0) r (churn (- i 1) (Record.without (Record.extend r #"t" i) (t)))))
            (def (main (: n Int64))
              (do
                (def r (record (a n) (b 2)))
                (def cycled (churn 20 r))
                (+ (* 10 (match (Map.lookup (Map.insert Map.empty r 42) cycled) ((Some v) v) ((None _u) -1)))
                   (if (= cycled r) 1 0))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 421 Int64)))
