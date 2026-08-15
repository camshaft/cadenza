(case "dth1 a DEAD-MAN'S switch — beat resets the miss counter answering the uptime tick, poll counts a miss answering it until the seed-shaped threshold LATCHES the alarm, and once latched EVERY answer is -9 forever (beats cannot clear it); the lower threshold latches six rows early and the long -9 tail pins the latch as absorbing"
  (input  (do
            (effect D
              (op beat (-> Int64))
              (op poll (-> Int64)))
            (def (main (: n Int64))
              (handle D (tuple (: 0 Int64) (: 0 Int64) (: 0 Int64))
                ((beat () st
                  (match st
                    ((tuple misses uptime latched)
                      (if (= latched 1)
                          (resume -9 st)
                          (resume (+ uptime 1) (tuple 0 (+ uptime 1) 0))))))
                 (poll () st
                  (match st
                    ((tuple misses uptime latched)
                      (if (= latched 1)
                          (resume -9 st)
                          (if (< (+ misses 1) (+ (% n 3) 2))
                              (resume (+ misses 1) (tuple (+ misses 1) uptime 0))
                              (resume -9 (tuple (+ misses 1) uptime 1))))))))
                (let ((a (D.beat)))
                  (let ((b (D.poll)))
                    (let ((c (D.poll)))
                      (let ((d (D.beat)))
                        (let ((e (D.poll)))
                          (let ((f (D.poll)))
                            (let ((g (D.poll)))
                              (let ((h (D.beat)))
                                (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g)) h)))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 101020201019091 Int64))
  (call   main (: 0 Int64)) (output (: 100909090909091 Int64)))
