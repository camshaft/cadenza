(case "ss2g control: NON-recursive grow-equivalent (2 puts inline in the do) then fin"
  (input  (do
            (effect Acc (op put (-> Unit Int64)) (op fin (-> Unit Int64)))
            (def (main (: k Int64))
              (handle Acc 0
                ((put (u) s (resume 0 (+ s 1)))
                 (fin (u) s s))
                (do (def _a (+ (Acc.put) (Acc.put))) (Acc.fin))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 2 Int64)))
