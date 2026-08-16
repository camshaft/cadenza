(case "oar1 a FOUR-arg effect op binds positionally (place-value checksum)"
  (input  (do
            (effect Calc (op mix4 (-> Int64 Int64 Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle Calc 0
                ((mix4 (a b c d) s (resume (+ (* 1000 a) (+ (* 100 b) (+ (* 10 c) d))) s)))
                (Calc.mix4 n 2 3 4)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5234 Int64)))
