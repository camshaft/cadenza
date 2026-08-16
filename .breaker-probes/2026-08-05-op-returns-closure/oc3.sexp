(case "oc3 a resumed closure's captured state SNAPSHOT survives a later advance (tuple-crossed)"
  (input  (do
            (effect St (op mk (-> Unit (Tuple (-> Int64 Int64) Int64))) (op bump (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((mk (u) s (resume (tuple (fn ((: x Int64)) (+ x s)) 0) s))
                 (bump (u) s (resume s (+ s 10))))
                (match (St.mk)
                  ((tuple f _z)
                    (let ((b (St.bump)))
                      (+ (* 100 (f 1)) (+ b (St.bump))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 620 Int64)))
