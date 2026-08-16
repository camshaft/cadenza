(case "hv3 a TUPLE of two closures escaping the handle (pair not list)"
  (input  (do
            (effect Ctr (op tick (-> Unit Int64)))
            (def (main (: k Int64))
              (do
                (def pair
                  (handle Ctr k ((tick (u) s (resume s (+ s 1))))
                    (let ((a (Ctr.tick)))
                      (let ((b (Ctr.tick)))
                        (tuple (fn ((: x Int64)) (+ x a))
                               (fn ((: x Int64)) (* x b)))))))
                (+ (* 100 ((. pair 0) 1)) ((. pair 1) 2))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 612 Int64)))
