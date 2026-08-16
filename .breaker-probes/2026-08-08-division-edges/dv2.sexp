(case "dv2 the DIVISOR comes from the arm with a state-dependent sign — quotient and remainder track the alternating divisor exactly"
  (input  (do
            (effect E (op next (-> Int64)) (op getdiv (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (getdiv () s (resume (if (= (% s 2) 0) 3 -3) (+ s 1)))
                 (probe () s (resume s s)))
                (let ((a (E.next)))
                  (let ((d (E.getdiv)))
                    (+ (* 100 (/ a d))
                       (+ (* 10 (% a d)) (- (E.probe) n)))))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 212 Int64))
  (call   main (: -8 Int64)) (output (: 182 Int64))
  (call   main (: 4 Int64)) (output (: -88 Int64)))
