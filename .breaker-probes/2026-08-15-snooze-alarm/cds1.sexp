(case "cds1 a COUNTDOWN alarm with snooze — tick decrements answering the remaining time until it FIRES at zero (answering a negated ten-plus-fire-count and auto-reloading the seed interval), snooze adds slack, and the shorter interval fires MID-STREAM where the longer one fires on the very last tick"
  (input  (do
            (effect A
              (op tick (-> Int64))
              (op snooze (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle A (tuple (+ (% n 4) 3) (: 0 Int64))
                ((tick () st
                  (match st
                    ((tuple rem fires)
                      (if (= (- rem 1) 0)
                          (resume (- 0 (+ 11 fires)) (tuple (+ (% n 4) 3) (+ fires 1)))
                          (resume (- rem 1) (tuple (- rem 1) fires))))))
                 (snooze (k) st
                  (match st
                    ((tuple rem fires) (resume (+ rem k) (tuple (+ rem k) fires))))))
                (let ((a (A.tick)))
                  (let ((b (A.tick)))
                    (let ((c (A.snooze 2)))
                      (let ((d (A.tick)))
                        (let ((e (A.tick)))
                          (let ((f (A.tick)))
                            (let ((g (A.tick)))
                              (let ((h (A.tick)))
                                (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g)) h)))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 403050403020089 Int64))
  (call   main (: 0 Int64)) (output (: 201030200890201 Int64)))
