(case "ss2d dissect: fin AFTER two DIRECT puts (no recursion) — does state survive to fin?"
  (input  (do
            (effect Acc (op put (-> Unit Int64)) (op fin (-> Unit String)))
            (def (main)
              (String.scalar-len
                (handle Acc ""
                  ((put (u) s (resume 0 (String.concat s "xy")))
                   (fin (u) s s))
                  (do (def _a (Acc.put)) (def _b (Acc.put)) (Acc.fin)))))
            (export main)))
  (output (: 4 Int64)))
