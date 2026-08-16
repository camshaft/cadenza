(case "scl1 a BALANCE SCALE — each weight lands on the lighter pan with ties going LEFT and answers the pan code with the new imbalance, swap exchanges the pans counting itself, verdict packs both pans and the swap count, and the seed biases only the FIRST weight yet flips the THIRD placement's pan through the tie rule"
  (input  (do
            (effect B
              (op place (-> Int64 Int64))
              (op swap (-> Int64))
              (op verdict (-> Int64)))
            (def (main (: n Int64))
              (handle B (tuple (: 0 Int64) (: 0 Int64) (: 0 Int64))
                ((place (w) st
                  (match st
                    ((tuple l r s)
                      (if (<= l r)
                          (resume (+ (: 100 Int64) (if (> (+ l w) r) (- (+ l w) r) (- r (+ l w))))
                                  (tuple (+ l w) r s))
                          (resume (+ (: 200 Int64) (if (> l (+ r w)) (- l (+ r w)) (- (+ r w) l)))
                                  (tuple l (+ r w) s))))))
                 (swap () st
                  (match st
                    ((tuple l r s)
                      (resume (+ (* r 10) l) (tuple r l (+ s 1))))))
                 (verdict () st
                  (match st
                    ((tuple l r s)
                      (resume (+ (* l 100) (+ (* r 10) s)) st)))))
                (let ((a (B.place (+ (: 4 Int64) (% n 3)))))
                  (let ((b (B.place (: 4 Int64))))
                    (let ((c (B.place (: 2 Int64))))
                      (let ((d (B.swap)))
                        (let ((f (B.verdict)))
                          (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 105201201065651 Int64))
  (call   main (: 0 Int64)) (output (: 104200102046461 Int64)))
