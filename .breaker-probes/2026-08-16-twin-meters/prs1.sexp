(case "prs1 TWIN parking meters — feed adds coins times the seed rate to one meter answering its minutes, tick decrements BOTH clamped at zero answering how many now read expired, and the slower rate expires meter one two ticks earlier walking the expired count up to two while the faster rate holds at one"
  (input  (do
            (effect P
              (op feed (-> Int64 Int64 Int64))
              (op tick (-> Int64)))
            (def (dec1 (: v Int64)) (if (< 0 v) (- v 1) 0))
            (def (exp1 (: v Int64)) (if (= v 0) 1 0))
            (def (main (: n Int64))
              (handle P (tuple (: 0 Int64) (: 0 Int64))
                ((feed (i c) st
                  (match st
                    ((tuple m0 m1)
                      (if (= i 0)
                          (resume (+ m0 (* c (+ (% n 3) 2))) (tuple (+ m0 (* c (+ (% n 3) 2))) m1))
                          (resume (+ m1 (* c (+ (% n 3) 2))) (tuple m0 (+ m1 (* c (+ (% n 3) 2)))))))))
                 (tick () st
                  (match st
                    ((tuple m0 m1)
                      (resume (+ (exp1 (dec1 m0)) (exp1 (dec1 m1)))
                              (tuple (dec1 m0) (dec1 m1)))))))
                (let ((a (P.feed 0 2)))
                  (let ((b (P.feed 1 1)))
                    (let ((c (P.tick)))
                      (let ((d (P.tick)))
                        (let ((e (P.tick)))
                          (let ((f (P.tick)))
                            (let ((g (P.tick)))
                              (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 6030000010101 Int64))
  (call   main (: 0 Int64)) (output (: 4020001010202 Int64)))
