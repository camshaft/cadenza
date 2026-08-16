(case "lgt1 a TRAFFIC light with a demand sensor — tick counts down green-three yellow-one red-seed cycling on zero (answers pack color-times-ten plus remaining), demand during a LONG red shortens the remainder to one answering a hundred plus the old value (otherwise it just reads), and the seed-shaped red length changes both the red row and the demand's rebate"
  (input  (do
            (effect T
              (op tick (-> Int64))
              (op demand (-> Int64)))
            (def (main (: n Int64))
              (handle T (tuple (: 0 Int64) (: 3 Int64))
                ((tick () st
                  (match st
                    ((tuple color rem)
                      (if (= (- rem 1) 0)
                          (if (= color 0)
                              (resume 11 (tuple 1 1))
                              (if (= color 1)
                                  (resume (+ 20 (+ (% n 3) 2)) (tuple 2 (+ (% n 3) 2)))
                                  (resume 3 (tuple 0 3))))
                          (resume (+ (* color 10) (- rem 1)) (tuple color (- rem 1)))))))
                 (demand () st
                  (match st
                    ((tuple color rem)
                      (if (= color 2)
                          (if (< 1 rem)
                              (resume (+ 100 rem) (tuple 2 1))
                              (resume (+ 20 rem) st))
                          (resume (+ (* color 10) rem) st))))))
                (let ((a (T.tick)))
                  (let ((b (T.tick)))
                    (let ((c (T.tick)))
                      (let ((d (T.tick)))
                        (let ((e (T.demand)))
                          (let ((f (T.tick)))
                            (let ((g (T.tick)))
                              (let ((h (T.demand)))
                                (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g)) h)))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 201112403030202 Int64))
  (call   main (: 0 Int64)) (output (: 201112302030202 Int64)))
