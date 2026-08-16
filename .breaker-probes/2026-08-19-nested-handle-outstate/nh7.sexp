(case "nh7 depth-3 do-statement variant: the c3-nested shape TWO handles deep"
  (input  (do
            (effect A (op ga (-> Unit Int64)) (op pa (-> Int64 Unit)))
            (effect B (op gb (-> Unit Int64)))
            (effect C (op gc (-> Unit Int64)))
            (def (main (: x Int64))
              (handle A x
                ((ga (u) s (resume s (+ s 1)))
                 (pa (v) _s (resume unit v)))
                (handle B 100 ((gb (u) t (resume t t)))
                  (handle C 200 ((gc (u) w (resume w w)))
                    (do
                      (let ((k true)) (if k (A.pa 7) unit))
                      (+ (* 10 (A.ga)) x))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 73 Int64)))
