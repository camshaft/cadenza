(case "pom5 the merged pomodoro with the session length carried IN STATE (3-tuple, no n in arm)"
  (input  (do
            (effect P
              (op work (-> Int64))
              (op rest (-> Int64)))
            (def (main (: n Int64))
              (handle P (tuple (: 0 Int64) (: 0 Int64) (+ (% n 3) 2))
                ((work (
                  ) st
                  (match st
                    ((tuple phase sessions slen)
                      (if (< 101 phase)
                          (resume (- 100 phase) (tuple (- phase 1) sessions slen))
                          (if (< 99 phase)
                              (resume (- 100 phase) (tuple 0 sessions slen))
                              (if (= (+ phase 1) slen)
                                  (resume (+ 101 sessions) (tuple 102 (+ sessions 1) slen))
                                  (resume (+ phase 1) (tuple (+ phase 1) sessions slen))))))))
                 (rest () st
                  (match st ((tuple phase sessions slen) (resume sessions st)))))
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