(case "ss2c dissect: LENGTHS of escaped-rope vs flat twin (are they even the same size?)"
  (input  (do
            (def (rep (: n Int64) (: acc String))
              (if (= n 0) acc (rep (- n 1) (String.concat acc "xy"))))
            (effect Acc (op put (-> Unit Int64)) (op fin (-> Unit String)))
            (def (grow (: n Int64))
              (if (= n 0) 0 (+ (Acc.put) (grow (- n 1)))))
            (def (main (: k Int64))
              (do
                (def esc (handle Acc ""
                           ((put (u) s (resume 0 (String.concat s "xy")))
                            (fin (u) s s))
                           (do (def _g (grow k)) (Acc.fin))))
                (+ (* 1000 (String.scalar-len esc)) (String.scalar-len (rep k "")))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 6006 Int64)))
