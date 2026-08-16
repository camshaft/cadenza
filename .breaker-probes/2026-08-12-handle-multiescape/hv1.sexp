(case "hv1 a handle whose value is a LIST OF CLOSURES each capturing a different perform result"
  (input  (do
            (effect Ctr (op tick (-> Unit Int64)))
            (def (main (: k Int64))
              (do
                (def fns
                  (handle Ctr k ((tick (u) s (resume s (+ s 1))))
                    (let ((a (Ctr.tick)))
                      (let ((b (Ctr.tick)))
                        (list (fn ((: x Int64)) (+ x a))
                              (fn ((: x Int64)) (* x b)))))))
                (+ (* 100 (match (List.at fns 0) ((Some f) (f 1)) ((None _u) -1)))
                   (match (List.at fns 1) ((Some g) (g 2)) ((None _u) -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 612 Int64)))
