(case "ss2e dissect: fin after RECURSIVE grow (n=2) — scalar-len of the escaped state"
  (input  (do
            (effect Acc (op put (-> Unit Int64)) (op fin (-> Unit String)))
            (def (grow (: n Int64))
              (if (= n 0) 0 (+ (Acc.put) (grow (- n 1)))))
            (def (main (: k Int64))
              (String.scalar-len
                (handle Acc ""
                  ((put (u) s (resume 0 (String.concat s "xy")))
                   (fin (u) s s))
                  (do (def _g (grow k)) (Acc.fin)))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 4 Int64)))
