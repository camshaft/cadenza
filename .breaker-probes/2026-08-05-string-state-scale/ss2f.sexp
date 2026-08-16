(case "ss2f control: SCALAR state, recursive grow, then a same-effect observer op"
  (input  (do
            (effect Acc (op put (-> Unit Int64)) (op fin (-> Unit Int64)))
            (def (grow (: n Int64))
              (if (= n 0) 0 (+ (Acc.put) (grow (- n 1)))))
            (def (main (: k Int64))
              (handle Acc 0
                ((put (u) s (resume 0 (+ s 1)))
                 (fin (u) s s))
                (do (def _g (grow k)) (Acc.fin))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 2 Int64)))
