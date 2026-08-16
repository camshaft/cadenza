(case "cs2 the closure state is REPLACED per dispatch by one capturing the arm's OWN binder"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St (fn ((: x Int64)) (+ x n))
                ((next (u) f
                  (let ((r (f 100)))
                    (resume r (fn ((: x Int64)) (+ x r))))))
                (+ (* 1000 (St.next)) (St.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 105205 Int64)))
