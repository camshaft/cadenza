(case "bnfE 2-tuple FIFO control — (front, back) only, no counter"
  (input  (do
            (effect Q
              (op join (-> Int64))
              (op serve (-> Int64)))
            (def (main (: n Int64))
              (handle Q (tuple (: 1 Int64) (+ (% n 4) 3))
                ((join () st
                  (match st
                    ((tuple front back)
                      (resume (- (+ back 1) front) (tuple front (+ back 1))))))
                 (serve () st
                  (match st
                    ((tuple front back)
                      (if (< front back)
                          (resume front (tuple (+ front 1) back))
                          (resume -1 st))))))
                (let ((a (Q.serve)))
                  (let ((b (Q.join)))
                    (let ((c (Q.serve)))
                      (let ((d (Q.serve)))
                        (let ((e (Q.join)))
                          (let ((f (Q.serve)))
                            (let ((g (Q.serve)))
                              (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1040203030405 Int64))
  (call   main (: 0 Int64)) (output (: 1020203010399 Int64)))
