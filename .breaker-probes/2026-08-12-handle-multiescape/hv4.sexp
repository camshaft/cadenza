(case "hv4 pinned-style tuple (lets inside positions) but DOT-projected instead of match"
  (input  (do
            (effect Ctr (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def pair
                  (handle Ctr n ((next (u) s (resume s (+ s 1))))
                    (tuple (let ((a (Ctr.next unit))) (fn ((: x Int64)) (+ x a)))
                           (let ((b (Ctr.next unit))) (fn ((: x Int64)) (* x b))))))
                (+ ((. pair 0) 100) ((. pair 1) 10))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 143 Int64)))
