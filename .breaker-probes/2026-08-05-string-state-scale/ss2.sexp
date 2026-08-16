(case "ss2 the 100-concat rope state ESCAPES via a final abort and compares equal to its flat twin"
  (input  (do
            (def (rep (: n Int64) (: acc String))
              (if (= n 0) acc (rep (- n 1) (String.concat acc "xy"))))
            (effect Acc (op put (-> Unit Int64)) (op fin (-> Unit String)))
            (def (grow (: n Int64))
              (if (= n 0) 0 (do (def _i (Acc.put)) (grow (- n 1)))))
            (def (main (: k Int64))
              (if (= (handle Acc ""
                       ((put (u) s (resume 0 (String.concat s "xy")))
                        (fin (u) s s))
                       (do (def _g (grow k)) (Acc.fin)))
                     (rep k ""))
                1 0))
            (export main)))
  (call   main (: 100 Int64)) (output (: 1 Int64)))
