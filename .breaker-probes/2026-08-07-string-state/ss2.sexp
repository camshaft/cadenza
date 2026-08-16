(case "ss2 string state grows across a DO sequence — discarded draws still advance the rope"
  (input  (do
            (effect Log (op emit (-> Int64)))
            (def (main (: n Int64))
              (handle Log "s"
                ((emit () s (resume (String.byte-len s) (String.concat s "yz"))))
                (do
                  (Log.emit)
                  (Log.emit)
                  (+ (* 10 (Log.emit)) n))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 55 Int64))
  (call   main (: 0 Int64)) (output (: 50 Int64)))
