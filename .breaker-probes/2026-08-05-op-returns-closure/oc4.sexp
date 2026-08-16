(case "oc4 TWO closures minted at different states keep their OWN snapshots (tuple-crossed)"
  (input  (do
            (effect St (op mk (-> Unit (Tuple (-> Int64 Int64) Int64))) (op bump (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((mk (u) s (resume (tuple (fn ((: x Int64)) (+ x s)) 0) s))
                 (bump (u) s (resume s (+ s 10))))
                (match (St.mk)
                  ((tuple f _z)
                    (do (St.bump)
                        (match (St.mk)
                          ((tuple g _w) (+ (* 100 (f 0)) (g 0)))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 515 Int64)))
