(case "nh6 c3-nested: block-wrapped OUTER perform in a do-statement inside a nested handle"
  (input  (do
            (effect A (op ga (-> Unit Int64)) (op pa (-> Int64 Unit)))
            (effect B (op gb (-> Unit Int64)))
            (def (main (: x Int64))
              (handle A x
                ((ga (u) s (resume s (+ s 1)))
                 (pa (v) _s (resume unit v)))
                (handle B 100 ((gb (u) t (resume t t)))
                  (do
                    (let ((k true)) (if k (A.pa 7) unit))
                    (+ (* 10 (A.ga)) x)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 73 Int64)))
