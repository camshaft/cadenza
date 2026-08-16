(case "oc1 the arm resumes a CLOSURE capturing the perform-time state; body applies it AFTER a later advance"
  (input  (do
            (effect St (op mk (-> Unit (-> Int64 Int64))) (op bump (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((mk (u) s (resume (fn ((: x Int64)) (+ x s)) s))
                 (bump (u) s (resume s (+ s 10))))
                (let ((f (St.mk)))
                  (do (St.bump)
                      (+ (* 100 (f 1)) (St.bump))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 615 Int64)))
