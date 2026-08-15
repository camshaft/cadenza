(case "trn1 a TURNSTILE coin/push machine — coin unlocks or wastes (wasted coins answer a deepening negative count), push passes and relocks or bounces (bounces answer their own negative count), the seed decides whether the gate STARTS unlocked, and the first push diverges the runs from the very first row"
  (input  (do
            (effect T
              (op coin (-> Int64))
              (op push (-> Int64)))
            (def (main (: n Int64))
              (handle T (tuple (if (< 5 n) 1 0) (: 0 Int64) (: 0 Int64) (: 0 Int64))
                ((coin () st
                  (match st
                    ((tuple u waste bounce passed)
                      (if (= u 1)
                          (resume (- 0 (+ waste 1)) (tuple 1 (+ waste 1) bounce passed))
                          (resume 1 (tuple 1 waste bounce passed))))))
                 (push () st
                  (match st
                    ((tuple u waste bounce passed)
                      (if (= u 1)
                          (resume (+ passed 1) (tuple 0 waste bounce (+ passed 1)))
                          (resume (- 0 (+ bounce 1)) (tuple 0 waste (+ bounce 1) passed)))))))
                (let ((a (T.push)))
                  (let ((b (T.coin)))
                    (let ((c (T.coin)))
                      (let ((d (T.push)))
                        (let ((e (T.push)))
                          (let ((f (T.coin)))
                            (let ((g (T.push)))
                              (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1009901990103 Int64))
  (call   main (: 0 Int64)) (output (: -990099019898 Int64)))
