(case "ssm1 a SEISMOGRAPH with drift and aftershock tracking — a reading deviating over four from the baseline logs an EVENT (seven-hundred row packing deviation and count, peak kept by an inline max, baseline nudged up one), a quiet reading drifts the baseline one signed step toward the magnitude answering the offset deviation, the read packs peak baseline and events, and the seed's initial baseline shifts every deviation so the peaks differ while the event count agrees"
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
                    (let ((c (Q.sense (: 9 Int64))))
                      (let ((d (Q.sense (: 16 Int64))))
                        (let ((f (Q.read)))
                          (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 751060020752622 Int64))
  (call   main (: 0 Int64)) (output (: 771080040772802 Int64)))
