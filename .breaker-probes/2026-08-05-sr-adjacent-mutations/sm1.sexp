(case "sm1 sr-adjacent: recursive advances then the abort observer arrives via a BRANCH (if selecting the abort)"
  (input  (do
            (effect Acc (op put (-> Unit Int64)) (op fin (-> Unit Int64)))
            (def (grow (: n Int64))
              (if (= n 0) 0 (+ (Acc.put) (grow (- n 1)))))
            (def (main (: k Int64))
              (handle Acc 0
                ((put (u) s (resume 0 (+ s 1)))
                 (fin (u) s s))
                (do (def _g (grow k))
                    (if (> k 0) (Acc.fin) -1))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 2 Int64)))
