(case "dic1 a DETERMINISTIC DIE with a hot-streak multiplier — each roll advances a linear-congruential state whose face five-or-higher DEEPENS the streak and scores the face TIMES the streak depth (a reset face scores plain), answers pack face streak and the score's low digit, tally packs score and the live streak, and the seeds' starting states weave streaks of different depths at different rolls"
  (input  (do
            (effect D
              (op roll (-> Int64))
              (op tally (-> Int64)))
            (def (main (: n Int64))
              (handle D (tuple (* (% n 3) 7) (: 0 Int64) (: 0 Int64))
                ((roll () st
                  (match st
                    ((tuple s streak score)
                      (if (>= (+ (% (% (+ (* s 7) 5) 31) 6) 1) 5)
                          (resume (+ (* (+ (% (% (+ (* s 7) 5) 31) 6) 1) 100)
                                     (+ (* (+ streak 1) 10)
                                        (% (+ score (* (+ (% (% (+ (* s 7) 5) 31) 6) 1) (+ streak 1))) 10)))
                                  (tuple (% (+ (* s 7) 5) 31)
                                         (+ streak 1)
                                         (+ score (* (+ (% (% (+ (* s 7) 5) 31) 6) 1) (+ streak 1)))))
                          (resume (+ (* (+ (% (% (+ (* s 7) 5) 31) 6) 1) 100)
                                     (% (+ score (+ (% (% (+ (* s 7) 5) 31) 6) 1)) 10))
                                  (tuple (% (+ (* s 7) 5) 31)
                                         (: 0 Int64)
                                         (+ score (+ (% (% (+ (* s 7) 5) 31) 6) 1))))))))
                 (tally () st
                  (match st
                    ((tuple s streak score) (resume (+ (* score 10) streak) st)))))
                (let ((a (D.roll)))
                  (let ((b (D.roll)))
                    (let ((c (D.roll)))
                      (let ((d (D.roll)))
                        (let ((e (D.roll)))
                          (let ((f (D.tally)))
                            (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 616628301405510301 Int64))
  (call   main (: 0 Int64)) (output (: 616400101516107170 Int64)))
