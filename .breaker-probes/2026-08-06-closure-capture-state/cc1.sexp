(case "cc1 a closure's captured perform result survives a LATER state advance (capture-time, not re-read)"
  (input  (do
            (effect St (op pull (-> Unit Int64)) (op bump (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St 40
                ((pull (u) s (resume s (+ s 1)))
                 (bump (u) s (resume s (+ s 10))))
                (let ((v (St.pull)))
                  (let ((f (fn ((: x Int64)) (+ x v))))
                    (do (St.bump)
                        (f 1))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 41 Int64)))
