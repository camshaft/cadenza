(case "esc1 an ESCROW protocol — hold moves funds from balance to escrow only when covered, rollback returns the whole escrow to balance, and the over-balance hold bounces without touching either slot"
  (input  (do
            (effect S
              (op hold (-> Int64 Int64))
              (op commit (-> Int64))
              (op rollback (-> Int64)))
            (def (main (: n Int64))
              (handle S (tuple n 0)
                ((hold (v) st
                  (match st
                    ((tuple bal esc)
                      (if (<= v bal)
                          (resume (+ (* 10 (- bal v)) (+ esc v)) (tuple (- bal v) (+ esc v)))
                          (resume -1 st)))))
                 (commit () st
                  (match st
                    ((tuple bal esc) (resume esc (tuple bal 0)))))
                 (rollback () st
                  (match st
                    ((tuple bal esc)
                      (resume (+ (* 10 (+ bal esc)) esc) (tuple (+ bal esc) 0))))))
                (let ((a (S.hold 4)))
                  (let ((b (S.hold 3)))
                    (let ((c (S.rollback)))
                      (let ((d (S.hold 20)))
                        (let ((e (S.commit)))
                          (+ (* 100 (+ (* 100 (+ (* 1000 (+ (* 100 (+ a 2)) (+ b 2))) c)) (+ d 2))) e))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 66391070100 Int64))
  (call   main (: 5 Int64)) (output (: 16010540100 Int64)))
