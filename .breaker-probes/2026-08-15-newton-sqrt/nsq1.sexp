(case "nsq1 NEWTON integer square root — improve does one Babylonian step (x + t/x)/2 from the high start answering the shrinking iterate, done checks the bracketing invariant x*x <= t < (x+1)^2, and the seeds CONVERGE AT DIFFERENT SPEEDS so the same done probe answers 0 on one run and 1 on the other"
  (input  (do
            (effect N
              (op improve (-> Int64))
              (op done (-> Int64)))
            (def (main (: n Int64))
              (handle N (tuple (+ 60 (* n 7)) (+ 60 (* n 7)))
                ((improve () st
                  (match st
                    ((tuple x t)
                      (resume (/ (+ x (/ t x)) 2) (tuple (/ (+ x (/ t x)) 2) t)))))
                 (done () st
                  (match st
                    ((tuple x t)
                      (if (< t (* (+ x 1) (+ x 1)))
                          (if (< t (* x x))
                              (resume 0 st)
                              (resume 1 st))
                          (resume 0 st))))))
                (let ((a (N.improve)))
                  (let ((b (N.improve)))
                    (let ((c (N.done)))
                      (let ((d (N.improve)))
                        (let ((e (N.improve)))
                          (let ((f (N.done)))
                            (let ((g (N.improve)))
                              (let ((h (N.done)))
                                (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g)) h)))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 6533001812001101 Int64))
  (call   main (: 0 Int64)) (output (: 3016000907010701 Int64)))
