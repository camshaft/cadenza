(case "qrm1 WEIGHTED QUORUM voting with a punitive veto — votes accumulate weight answering whether the threshold is met, veto ZEROES the tally and RAISES the threshold by two, tally packs weight and quorum-bit, and the seed sets the initial threshold so the same votes pass on one seed and never on the other"
  (input  (do
            (effect Q
              (op vote (-> Int64 Int64))
              (op veto (-> Int64))
              (op tally (-> Int64)))
            (def (main (: n Int64))
              (handle Q (tuple (: 0 Int64) (+ n 6))
                ((vote (v) st
                  (match st
                    ((tuple w thr)
                      (if (< (+ w v) thr)
                          (resume 0 (tuple (+ w v) thr))
                          (resume 1 (tuple (+ w v) thr))))))
                 (veto () st
                  (match st
                    ((tuple w thr) (resume (+ thr 2) (tuple 0 (+ thr 2))))))
                 (tally () st
                  (match st
                    ((tuple w thr)
                      (if (< w thr)
                          (resume (* w 10) st)
                          (resume (+ (* w 10) 1) st))))))
                (let ((a (Q.vote 5)))
                  (let ((b (Q.vote 4)))
                    (let ((c (Q.tally)))
                      (let ((d (Q.veto)))
                        (let ((e (Q.vote 9)))
                          (let ((f (Q.vote 8)))
                            (let ((g (Q.tally)))
                              (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 9018000170 Int64))
  (call   main (: 0 Int64)) (output (: 19108010271 Int64)))
