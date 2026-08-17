(case "bkp1 a BEEKEEPER'S smoker and frame pulls — pulling a frame needs four calm (success banks the frame and the agitation costs three calm, a rattled hive STINGS instead counting it), puffing adds calm capped at nine (a ninety-nine cap row), the read packs frames calm and stings, and the seed's starting calm pulls-first-clean on one hive against stung-first on the other with the shared puff landing on different calm floors"
  (input  (do
            (effect B
              (op puff (-> Int64 Int64))
              (op pull (-> Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle B (tuple (+ (: 2 Int64) (* (% n 3) 3)) (: 0 Int64) (: 0 Int64))
                ((puff (k) st
                  (match st
                    ((tuple calm fr sg)
                      (if (> (+ calm k) 9)
                          (resume (: 99 Int64) (tuple (: 9 Int64) fr sg))
                          (resume (* (+ calm k) 10) (tuple (+ calm k) fr sg))))))
                 (pull () st
                  (match st
                    ((tuple calm fr sg)
                      (if (>= calm 4)
                          (resume (+ (* (+ fr 1) 10) (% (- calm 3) 10))
                                  (tuple (- calm 3) (+ fr 1) sg))
                          (resume (+ (: 900 Int64) (+ sg 1))
                                  (tuple calm fr (+ sg 1)))))))
                 (read () st
                  (match st
                    ((tuple calm fr sg)
                      (resume (+ (* fr 100) (+ (* calm 10) sg)) st)))))
                (let ((a (B.pull)))
                  (let ((b (B.puff (: 4 Int64))))
                    (let ((c (B.pull)))
                      (let ((d (B.pull)))
                        (let ((f (B.read)))
                          (+ (* 10000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 120600239010231 Int64))
  (call   main (: 0 Int64)) (output (: 9010600139020132 Int64)))
