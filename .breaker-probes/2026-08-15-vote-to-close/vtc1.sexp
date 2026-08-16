(case "vtc1 a VOTE-TO-CLOSE quorum — second registers answering the count until the seed-shaped threshold CLOSES the motion (answering a hundred plus the count; later ops answer minus a hundred), withdraw decrements while open clamping at zero, and the lower threshold closes one second earlier turning the tail into closed sentinels"
  (input  (do
            (effect V
              (op second (-> Int64))
              (op withdraw (-> Int64)))
            (def (main (: n Int64))
              (handle V (tuple (: 0 Int64) (: 0 Int64))
                ((second () st
                  (match st
                    ((tuple count closed)
                      (if (= closed 1)
                          (resume -100 st)
                          (if (< (+ count 1) (+ (% n 3) 2))
                              (resume (+ count 1) (tuple (+ count 1) 0))
                              (resume (+ 100 (+ count 1)) (tuple (+ count 1) 1)))))))
                 (withdraw () st
                  (match st
                    ((tuple count closed)
                      (if (= closed 1)
                          (resume -100 st)
                          (if (< 0 count)
                              (resume (- count 1) (tuple (- count 1) 0))
                              (resume 0 st)))))))
                (let ((a (V.second)))
                  (let ((b (V.withdraw)))
                    (let ((c (V.second)))
                      (let ((d (V.second)))
                        (let ((e (V.second)))
                          (let ((f (V.withdraw)))
                            (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 10001030200 Int64))
  (call   main (: 0 Int64)) (output (: 10002009900 Int64)))
