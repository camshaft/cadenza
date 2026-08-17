(case "blc1 a BALANCE SCALE weighing TWO op arguments against each other — weigh's three-way arg-vs-arg comparison answers side and margin counting only LEFT wins, every weigh folds the signed difference into the running tilt, level reports the tilt's magnitude by sign-splitting WITHOUT abs tagging lean direction in the hundreds digit, and the seed loads the left pan of three weighings so one run balances twice level-right while the other tips left twice and reads a levelled beam last"
  (input  (do
            (effect L
              (op weigh (-> Int64 Int64 Int64))
              (op level (-> Int64)))
            (def (main (: n Int64))
              (handle L (tuple (: 0 Int64) (: 0 Int64))
                ((weigh (a b) st
                  (match st
                    ((tuple tilt lw)
                      (if (> a b)
                          (resume (+ (: 100 Int64) (+ (* (- a b) 10) (% (+ lw 1) 10)))
                                  (tuple (+ tilt (- a b)) (+ lw 1)))
                          (if (< a b)
                              (resume (+ (: 200 Int64) (+ (* (- b a) 10) (% lw 10)))
                                      (tuple (- tilt (- b a)) lw))
                              (resume (+ (: 300 Int64) (% lw 10))
                                      (tuple tilt lw)))))))
                 (level () st
                  (match st
                    ((tuple tilt lw)
                      (if (< tilt 0)
                          (resume (+ (: 500 Int64) (+ (* (- 0 tilt) 10) (% lw 10))) st)
                          (resume (+ (: 400 Int64) (+ (* tilt 10) (% lw 10))) st))))))
                (let ((s (% n 3)))
                  (let ((a (L.weigh (+ 2 s) 3)))
                    (let ((b (L.weigh (+ 1 s) 2)))
                      (let ((c (L.level)))
                        (let ((d (L.weigh 4 (+ 2 s))))
                          (let ((e (L.weigh 3 3)))
                            (let ((f (L.level)))
                              (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e)) f))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 300300400111301411 Int64))
  (call   main (: 0 Int64)) (output (: 210210520121301401 Int64)))
