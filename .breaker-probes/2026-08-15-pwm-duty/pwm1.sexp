(case "pwm1 a PWM duty-cycle counter — tick answers on/off by whether the phase within the period-4 window is below the duty, advancing the phase and counting ons; setduty retunes the width mid-stream answering the on-count so far, and the seed-shaped initial duty separates the first window's pattern"
  (input  (do
            (effect P
              (op tick (-> Int64))
              (op setduty (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle P (tuple (: 0 Int64) (+ (% n 3) 1) (: 0 Int64))
                ((tick () st
                  (match st
                    ((tuple phase duty ons)
                      (if (< (% phase 4) duty)
                          (resume 1 (tuple (+ phase 1) duty (+ ons 1)))
                          (resume 0 (tuple (+ phase 1) duty ons))))))
                 (setduty (v) st
                  (match st
                    ((tuple phase duty ons) (resume ons (tuple phase v ons))))))
                (let ((a (P.tick)))
                  (let ((b (P.tick)))
                    (let ((c (P.tick)))
                      (let ((d (P.tick)))
                        (let ((e (P.setduty 3)))
                          (let ((f (P.tick)))
                            (let ((g (P.tick)))
                              (let ((h (P.tick)))
                                (let ((i (P.setduty 0)))
                                  (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g)) h)) i))))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 10100000201010105 Int64))
  (call   main (: 0 Int64)) (output (: 10000000101010104 Int64)))
