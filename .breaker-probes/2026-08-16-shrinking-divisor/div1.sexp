(case "div1 a SHRINKING DIVISOR guarded to zero — each work divides its argument by the countdown divisor (quotient shared between the answer and the running total) while the divisor decrements toward the ZERO the guard branch absorbs (answering nine hundred with the state untouched and a save counted), the read packs total divisor and saves, and the seed's starting divisor reaches the guard one dispatch earlier on one run"
  (input  (do
            (effect W
              (op work (-> Int64 Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle W (tuple (+ (: 2 Int64) (% n 3)) (: 0 Int64) (: 0 Int64))
                ((work (x) st
                  (match st
                    ((tuple d total saves)
                      (if (= d 0)
                          (resume (: 900 Int64) (tuple d total (+ saves 1)))
                          (resume (+ (* (/ x d) 10) d)
                                  (tuple (- d 1) (+ total (/ x d)) saves))))))
                 (read () st
                  (match st
                    ((tuple d total saves)
                      (resume (+ (* total 100) (+ (* d 10) saves)) st)))))
                (let ((a (W.work (: 12 Int64))))
                  (let ((b (W.work (: 9 Int64))))
                    (let ((c (W.work (: 6 Int64))))
                      (let ((e (W.work (: 4 Int64))))
                        (let ((f (W.read)))
                          (+ (* 10000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) e)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 430420619001401 Int64))
  (call   main (: 0 Int64)) (output (: 620919009001502 Int64)))
