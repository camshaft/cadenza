(case "fe1 Float64 handler state advanced per perform (float arithmetic in arm)"
  (input  (do
            (effect Acc (op add (-> Float64 Float64)))
            (def (main)
              (handle Acc 0.5
                ((add (v) s (resume s (+ s v))))
                (do
                  (def a (Acc.add 1.25))
                  (def b (Acc.add 2.0))
                  (+ a (+ b (Acc.add 0.0))))))
            (export main)))
  (output (: 6.0 Float64)))
