(case "cc5 composed closures over three draws — g(f(draw)) where f and g each captured an earlier read"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (let ((a (St.next)))
                  (let ((f (fn ((: x Int64)) (+ x a))))
                    (let ((b (St.next)))
                      (let ((g (fn ((: x Int64)) (* x b))))
                        (g (f (St.next)))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 72 Int64))
  (call   main (: 0 Int64)) (output (: 2 Int64)))
