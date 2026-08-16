(case "oc2 TWO closures from the same op at different states — each keeps ITS OWN snapshot"
  (input  (do
            (effect St (op mk (-> Unit (-> Int64 Int64))) (op bump (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((mk (u) s (resume (fn ((: x Int64)) (+ x s)) s))
                 (bump (u) s (resume s (+ s 10))))
                (let ((f (St.mk)))
                  (do (St.bump)
                      (let ((g (St.mk)))
                        (+ (* 100 (f 0)) (g 0)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 515 Int64)))
