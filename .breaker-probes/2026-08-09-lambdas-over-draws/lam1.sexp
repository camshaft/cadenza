(case "lam1 INLINE and LET-BOUND lambdas applied to one draw — both closure forms read the same drawn value"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (let ((d (E.next)))
                  (let ((f (fn ((: y Int64)) (* 2 (+ y 10)))))
                    (+ (* 100 ((fn ((: x Int64)) (- (* 3 x) 1)) d))
                       (+ (* 10 (f d)) (- (E.probe) n)))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 741 Int64))
  (call   main (: 0 Int64)) (output (: 101 Int64))
  (call   main (: -3 Int64)) (output (: -859 Int64)))
