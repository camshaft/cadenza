(case "tpc1 TWO-PHASE COMMIT across two handlers — the coordinator prepares both sides, commits both only when both prepared, otherwise aborts both restoring the balances; the pass and fail paths both verified"
  (input  (do
            (effect A
              (op prep (-> Int64 Int64))
              (op fin (-> Int64 Int64)))
            (effect B
              (op prep (-> Int64 Int64))
              (op fin (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle A (tuple 10 0)
                ((prep (v) st
                  (match st
                    ((tuple bal _h)
                      (if (<= v bal)
                          (resume 1 (tuple (- bal v) v))
                          (resume 0 st)))))
                 (fin (ok) st
                  (match st
                    ((tuple bal h)
                      (if (= ok 1)
                          (resume bal (tuple bal 0))
                          (resume (+ bal h) (tuple (+ bal h) 0)))))))
                (handle B (tuple n 0)
                  ((prep (v) st
                    (match st
                      ((tuple bal _h)
                        (if (<= v bal)
                            (resume 1 (tuple (- bal v) v))
                            (resume 0 st)))))
                   (fin (ok) st
                    (match st
                      ((tuple bal h)
                        (if (= ok 1)
                            (resume bal (tuple bal 0))
                            (resume (+ bal h) (tuple (+ bal h) 0)))))))
                  (let ((pa (A.prep 4)))
                    (let ((pb (B.prep 6)))
                      (let ((ok (if (= (+ pa pb) 2) 1 0)))
                        (let ((ra (A.fin ok)))
                          (let ((rb (B.fin ok)))
                            (+ (* 10000 (+ (* 10 (+ (* 10 pa) pb)) ok))
                               (+ (* 100 ra) rb))))))))))
            (export main)))
  (call   main (: 8 Int64)) (output (: 1110602 Int64))
  (call   main (: 3 Int64)) (output (: 1001003 Int64)))
