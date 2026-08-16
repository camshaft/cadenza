(case "ss1 a String handler state grown by 100 rope concats across performs, scalar-len observed"
  (input  (do
            (effect Acc (op put (-> Unit Int64)))
            (def (loop (: n Int64) (: acc Int64))
              (if (= n 0) acc (loop (- n 1) (+ acc (Acc.put)))))
            (def (main (: k Int64))
              (handle Acc ""
                ((put (u) s (resume (String.scalar-len s) (String.concat s "xy"))))
                (loop k 0)))
            (export main)))
  (call   main (: 100 Int64)) (output (: 9900 Int64)))
