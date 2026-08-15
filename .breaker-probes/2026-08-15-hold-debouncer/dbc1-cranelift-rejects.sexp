(case "dbc1 a SIGNAL debouncer with a hold time — the clean output flips only after the raw level disagrees with it for the seed-shaped number of CONSECUTIVE feeds (agreement resets the pending count), each feed answers clean-times-ten plus pending, and the hold-one seed tracks the raw signal instantly while hold-two lags a beat behind every edge"
  (input  (do
            (effect D (op raw (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle D (tuple (: 0 Int64) (: 0 Int64))
                ((raw (v) st
                  (match st
                    ((tuple clean pend)
                      (if (= v clean)
                          (resume (* clean 10) (tuple clean 0))
                          (if (< (+ pend 1) (+ (% n 3) 1))
                              (resume (+ (* clean 10) (+ pend 1)) (tuple clean (+ pend 1)))
                              (resume (* v 10) (tuple v 0))))))))
                (let ((a (D.raw 1)))
                  (let ((b (D.raw 1)))
                    (let ((c (D.raw 0)))
                      (let ((d (D.raw 1)))
                        (let ((e (D.raw 1)))
                          (let ((f (D.raw 0)))
                            (let ((g (D.raw 0)))
                              (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1101110101100 Int64))
  (call   main (: 0 Int64)) (output (: 10100010100000 Int64)))
