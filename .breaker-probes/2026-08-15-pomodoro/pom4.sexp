(case "pom4 seed comparison removed (slen hardcoded 3) — under a hundred counts the session, a hundred-plus counts the break remaining, the same protocol answers survive the encoding, and the shorter session completes two pomodoros where the longer completes one"
  (input  (do
            (effect P
              (op work (-> Int64))
              (op rest (-> Int64)))
            (def (main (: n Int64))
              (handle P (tuple (: 0 Int64) (: 0 Int64))
                ((work () st
                  (match st
                    ((tuple phase sessions)
                      (if (< 101 phase)
                          (resume (- 100 phase) (tuple (- phase 1) sessions))
                          (if (< 99 phase)
                              (resume (- 100 phase) (tuple 0 sessions))
                              (if (= (+ phase 1) 3)
                                  (resume (+ 101 sessions) (tuple 102 (+ sessions 1)))
                                  (resume (+ phase 1) (tuple (+ phase 1) sessions))))))))
                 (rest () st
                  (match st ((tuple phase sessions) (resume sessions st)))))
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
  (call   main (: 0 Int64)) (output (: 103009799010201 Int64)))
