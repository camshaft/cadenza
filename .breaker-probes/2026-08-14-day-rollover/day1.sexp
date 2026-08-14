(case "day1 a DAY-ROLLOVER ledger — transactions accumulate in the day buffer, endday posts the net to the balance answers day and net and clears, and the second day starts from a clean buffer"
  (input  (do
            (effect S
              (op txn (-> Int64 Int64))
              (op endday (-> Int64)))
            (def (main (: n Int64))
              (handle S (tuple 100 0 0)
                ((txn (v) st
                  (match st
                    ((tuple bal buf day)
                      (resume (+ (+ buf v) 50) (tuple bal (+ buf v) day)))))
                 (endday () st
                  (match st
                    ((tuple bal buf day)
                      (resume (+ (* (+ day 1) 1000) (+ buf 100))
                              (tuple (+ bal buf) 0 (+ day 1)))))))
                (let ((a (S.txn n)))
                  (let ((b (S.txn -30)))
                    (let ((c (S.endday)))
                      (let ((d (S.txn 5)))
                        (let ((e (S.endday)))
                          (+ (* 10000 (+ (* 1000 (+ (* 10000 (+ (* 1000 a) b)) c)) d)) e))))))))
            (export main)))
  (call   main (: 20 Int64)) (output (: 7004010900552105 Int64))
  (call   main (: 50 Int64)) (output (: 10007011200552105 Int64)))
