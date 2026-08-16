(case "wp1d control: linear recursion performing in a STRICT-OPERAND (rw1 shape, known green)"
  (input  (do
            (effect Cnt (op bump (-> Unit Int64)))
            (def (loop (: n Int64))
              (if (= n 0) 0 (+ (Cnt.bump) (loop (- n 1)))))
            (def (main (: n Int64))
              (handle Cnt 0
                ((bump (u) s (resume s (+ s 1))))
                (loop n)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 10 Int64)))
