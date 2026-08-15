(case "bnfC control — phase check made CONSTANT (never bumps) — join answers the queue length, every THIRD serve pulls from the BACK of the range (drain guards flattened ahead of the phase check) while others advance the front, a drained queue answers -1, and the seed's pre-queued depth decides whether the last serve finds anyone"
  (input  (do
            (effect Q
              (op join (-> Int64))
              (op serve (-> Int64)))
            (def (main (: n Int64))
              (handle Q (tuple (: 1 Int64) (+ (% n 4) 3) (: 0 Int64))
                ((join () st
                  (match st
                    ((tuple front back k)
                      (resume (- (+ back 1) front) (tuple front (+ back 1) k)))))
                 (serve () st
                  (match st
                    ((tuple front back k)
                      (if (= front back)
                          (resume -1 st)
                          (if (< back front)
                              (resume -1 st)
                              (if (= 0 1)
                                  (resume (- back 1) (tuple front (- back 1) (+ k 1)))
                                  (resume front (tuple (+ front 1) back (+ k 1))))))))))
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
