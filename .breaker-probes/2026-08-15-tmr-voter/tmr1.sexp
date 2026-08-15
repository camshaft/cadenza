(case "tmr1 a TRIPLE-MODULAR-REDUNDANCY voter — set writes one of three channel slots, vote answers the majority value or -1 counting the all-differ disagreements, and the seed corrupts channel zero so one run stays unanimous while the other walks through a majority flip, a full disagreement, and a healed two-vote majority"
  (input  (do
            (effect T
              (op set (-> Int64 Int64 Int64))
              (op vote (-> Int64))
              (op bad (-> Int64)))
            (def (main (: n Int64))
              (handle T (tuple (: 0 Int64) (: 0 Int64) (: 0 Int64) (: 0 Int64))
                ((set (i v) st
                  (match st
                    ((tuple a b c k)
                      (if (= i 0)
                          (resume v (tuple v b c k))
                          (if (= i 1)
                              (resume v (tuple a v c k))
                              (resume v (tuple a b v k)))))))
                 (vote () st
                  (match st
                    ((tuple a b c k)
                      (if (= a b)
                          (resume a st)
                          (if (= a c)
                              (resume a st)
                              (if (= b c)
                                  (resume b st)
                                  (resume -1 (tuple a b c (+ k 1)))))))))
                 (bad () st
                  (match st ((tuple a b c k) (resume k st)))))
                (let ((p (T.set 0 (+ n 2))))
                  (let ((q (T.set 1 12)))
                    (let ((r (T.set 2 12)))
                      (let ((s (T.vote)))
                        (let ((t (T.set 1 7)))
                          (let ((u (T.vote)))
                            (let ((v (T.set 2 (+ n 2))))
                              (let ((w (T.vote)))
                                (let ((x (T.bad)))
                                  (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 p) q)) r)) s)) t)) u)) v)) w)) x))))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 121212120712121200 Int64))
  (call   main (: 0 Int64)) (output (: 21212120699020201 Int64)))
