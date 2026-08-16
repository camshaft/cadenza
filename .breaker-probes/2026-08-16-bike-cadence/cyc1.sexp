(case "cyc1 a BICYCLE cadence over four gear ratios — shift moves the gear clamped to the range answering the landed ratio's tens digit, pedal answers rpm times the ratio over a hundred accumulating distance, the big downshift clamps BOTH runs to the same bottom gear so the tails converge while the opening gears diverge, and the final row reads the odometer"
  (input  (do
            (effect B
              (op shift (-> Int64 Int64))
              (op pedal (-> Int64 Int64))
              (op odo (-> Int64)))
            (def (ratio (: g Int64))
              (if (= g 0) 20 (if (= g 1) 27 (if (= g 2) 34 41))))
            (def (main (: n Int64))
              (handle B (tuple (% n 4) (: 0 Int64))
                ((shift (d) st
                  (match st
                    ((tuple g dist)
                      (if (< (+ g d) 0)
                          (resume 2 (tuple 0 dist))
                          (if (< 3 (+ g d))
                              (resume 4 (tuple 3 dist))
                              (resume (/ (ratio (+ g d)) 10) (tuple (+ g d) dist)))))))
                 (pedal (rpm) st
                  (match st
                    ((tuple g dist)
                      (resume (/ (* rpm (ratio g)) 100)
                              (tuple g (+ dist (/ (* rpm (ratio g)) 100)))))))
                 (odo () st
                  (match st ((tuple g dist) (resume dist st)))))
                (let ((a (B.pedal 60)))
                  (let ((b (B.shift 1)))
                    (let ((c (B.pedal 60)))
                      (let ((d (B.shift -3)))
                        (let ((e (B.pedal 60)))
                          (let ((f (B.shift 1)))
                            (let ((g (B.pedal 90)))
                              (let ((h (B.odo)))
                                (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g)) h)))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 2004240212022480 Int64))
  (call   main (: 0 Int64)) (output (: 1202160212022464 Int64)))
