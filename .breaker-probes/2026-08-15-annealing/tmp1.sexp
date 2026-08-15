(case "tmp1 a SIMULATED-ANNEALING acceptance schedule — cool decays the temperature by nine-tenths truncating, accept takes any improvement and any worsening still under the heat counting the accepts, and the hot seed accepts EVERYTHING the cold seed rejects while both take the improving move"
  (input  (do
            (effect A
              (op cool (-> Int64))
              (op accept (-> Int64 Int64))
              (op tally (-> Int64)))
            (def (main (: n Int64))
              (handle A (tuple (+ 40 (* n 4)) (: 0 Int64))
                ((cool () st
                  (match st
                    ((tuple temp k)
                      (resume (/ (* temp 9) 10) (tuple (/ (* temp 9) 10) k)))))
                 (accept (d) st
                  (match st
                    ((tuple temp k)
                      (if (< d 1)
                          (resume 1 (tuple temp (+ k 1)))
                          (if (< d temp)
                              (resume 1 (tuple temp (+ k 1)))
                              (resume 0 st))))))
                 (tally () st
                  (match st ((tuple temp k) (resume k st)))))
                (let ((a (A.accept 50)))
                  (let ((b (A.cool)))
                    (let ((c (A.accept 50)))
                      (let ((d (A.cool)))
                        (let ((e (A.accept -3)))
                          (let ((f (A.cool)))
                            (let ((g (A.accept 50)))
                              (let ((h (A.accept 30)))
                                (let ((i (A.tally)))
                                  (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g)) h)) i))))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 17201640157010105 Int64))
  (call   main (: 0 Int64)) (output (: 3600320128000001 Int64)))
