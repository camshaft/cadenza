(case "sm3 sr-adjacent: TWO grows with the abort observer between them (partial advance observed)"
  (input  (do
            (effect Acc (op put (-> Unit Int64)) (op fin (-> Unit Int64)))
            (def (grow (: n Int64))
              (if (= n 0) 0 (+ (Acc.put) (grow (- n 1)))))
            (def (main (: k Int64))
              (+ (handle Acc 0
                   ((put (u) s (resume 0 (+ s 1)))
                    (fin (u) s s))
                   (do (def _g1 (grow k)) (def _g2 (grow k)) (Acc.fin)))
                 0))
            (export main)))
  (call   main (: 2 Int64)) (output (: 4 Int64)))
