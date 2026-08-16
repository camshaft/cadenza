(case "s5 a discarded pure trapping item in a do with a HANDLED perform is elided"
  (input  (do
            (effect A (op bump (-> Unit Int64)))
            (def (main (: d Int64))
              (handle A 0
                ((bump (u) s (resume s (+ s 1))))
                (do (/ 100 d)
                    (A.bump unit)
                    42)))
            (export main)))
  (call   main (: 0 Int64)) (output (: 42 Int64)))
