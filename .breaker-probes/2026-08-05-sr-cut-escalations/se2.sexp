(case "se2 sr-cut escalation: MUTUAL recursion advancing, then same-effect abort"
  (input  (do
            (effect Acc (op put (-> Unit Int64)) (op fin (-> Unit Int64)))
            (def (even-w (: n Int64))
              (if (= n 0) 0 (+ (Acc.put) (odd-w (- n 1)))))
            (def (odd-w (: n Int64))
              (if (= n 0) 0 (+ (Acc.put) (even-w (- n 1)))))
            (def (main (: k Int64))
              (handle Acc 0
                ((put (u) s (resume 0 (+ s 1)))
                 (fin (u) s s))
                (do (def _g (even-w k)) (Acc.fin))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 2 Int64)))
