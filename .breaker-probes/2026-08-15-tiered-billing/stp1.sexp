(case "stp1 TIERED flat-fee billing — use accrues usage answering it, bill answers the current tier's flat fee (five under ten, twelve under twenty-five, twenty above) resetting usage, total reads the collected fees, and the seed pre-loads usage so the FIRST bill lands in a different tier while the later cycles converge"
  (input  (do
            (effect B
              (op use (-> Int64 Int64))
              (op bill (-> Int64))
              (op total (-> Int64)))
            (def (main (: n Int64))
              (handle B (tuple n (: 0 Int64))
                ((use (u) st
                  (match st
                    ((tuple usage bills) (resume (+ usage u) (tuple (+ usage u) bills)))))
                 (bill () st
                  (match st
                    ((tuple usage bills)
                      (if (< usage 10)
                          (resume 5 (tuple 0 (+ bills 5)))
                          (if (< usage 25)
                              (resume 12 (tuple 0 (+ bills 12)))
                              (resume 20 (tuple 0 (+ bills 20))))))))
                 (total () st
                  (match st ((tuple usage bills) (resume bills st)))))
                (let ((a (B.use 8)))
                  (let ((b (B.bill)))
                    (let ((c (B.use 20)))
                      (let ((d (B.bill)))
                        (let ((e (B.use 4)))
                          (let ((f (B.bill)))
                            (let ((g (B.total)))
                              (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 18122012040529 Int64))
  (call   main (: 0 Int64)) (output (: 8052012040522 Int64)))
