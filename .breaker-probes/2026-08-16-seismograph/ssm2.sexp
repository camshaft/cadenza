(case "ssm2 the SEISMOGRAPH at three readings — deviation over four logs an event (peak by inline max, baseline nudged up), quiet drifts one signed step, and the seeds disagree on whether the THIRD reading is an aftershock event or a quiet drift so even the event counts split"
  (input  (do
            (effect Q
              (op sense (-> Int64 Int64))
              (op read (-> Int64)))
            (def (sgn (: d Int64))
              (if (> d 0) 1 (if (< d 0) (: -1 Int64) 0)))
            (def (main (: n Int64))
              (handle Q (tuple (+ (: 8 Int64) (* (% n 3) 2)) (: 0 Int64) (: 0 Int64))
                ((sense (mag) st
                  (match st
                    ((tuple base peak ev)
                      (if (> (- mag base) 4)
                          (resume (+ (: 700 Int64) (+ (* (- mag base) 10) (+ ev 1)))
                                  (tuple (+ base 1)
                                         (if (> (- mag base) peak) (- mag base) peak)
                                         (+ ev 1)))
                          (resume (* (+ (- mag base) 5) 10)
                                  (tuple (+ base (sgn (- mag base))) peak ev))))))
                 (read () st
                  (match st
                    ((tuple base peak ev)
                      (resume (+ (* peak 100) (+ (* base 10) ev)) st)))))
                (let ((a (Q.sense (: 15 Int64))))
                  (let ((b (Q.sense (: 12 Int64))))
                    (let ((c (Q.sense (: 16 Int64))))
                      (let ((f (Q.read)))
                        (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) f)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 751060090631 Int64))
  (call   main (: 0 Int64)) (output (: 771080762812 Int64)))
