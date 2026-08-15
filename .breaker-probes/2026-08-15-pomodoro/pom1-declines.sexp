(case "pom1 a POMODORO timer — work ticks the session answering the count until the seed-shaped length rolls into a two-tick BREAK (answering a hundred plus completed sessions), work during a break answers the negated remaining and counts it down, rest reads sessions, and the shorter session completes TWO pomodoros where the longer completes one"
  (input  (do
            (effect P
              (op work (-> Int64))
              (op rest (-> Int64)))
            (def (main (: n Int64))
              (handle P (tuple (: 0 Int64) (: 0 Int64) (: 0 Int64))
                ((work () st
                  (match st
                    ((tuple wcount brk sessions)
                      (if (< 0 brk)
                          (resume (- 0 brk) (tuple wcount (- brk 1) sessions))
                          (if (= (+ wcount 1) (+ (% n 3) 2))
                              (resume (+ 101 sessions) (tuple 0 2 (+ sessions 1)))
                              (resume (+ wcount 1) (tuple (+ wcount 1) brk sessions)))))))
                 (rest () st
                  (match st ((tuple wcount brk sessions) (resume sessions st)))))
                (let ((a (P.work)))
                  (let ((b (P.work)))
                    (let ((c (P.work)))
                      (let ((d (P.work)))
                        (let ((e (P.work)))
                          (let ((f (P.work)))
                            (let ((g (P.work)))
                              (let ((h (P.rest)))
                                (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g)) h)))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 103009799010201 Int64))
  (call   main (: 0 Int64)) (output (: 200979902019802 Int64)))
