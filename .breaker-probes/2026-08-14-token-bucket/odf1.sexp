(case "odf1 a TOKEN-BUCKET rate limiter with overdraft penalties — spend succeeds only when the bucket covers it (else a penalty tick and a 0 answer), refill saturates at the cap, and the final draw reads the accumulated penalty count"
  (input  (do
            (effect B
              (op spend (-> Int64 Int64))
              (op refill (-> Int64 Int64))
              (op pens (-> Int64)))
            (def (main (: n Int64))
              (handle B (tuple n (: 0 Int64))
                ((spend (v) st
                  (match st
                    ((tuple tk pen)
                      (if (< tk v)
                          (resume 0 (tuple tk (+ pen 1)))
                          (resume v (tuple (- tk v) pen))))))
                 (refill (v) st
                  (match st
                    ((tuple tk pen)
                      (if (< 10 (+ tk v))
                          (resume 10 (tuple 10 pen))
                          (resume (+ tk v) (tuple (+ tk v) pen))))))
                 (pens () st
                  (match st
                    ((tuple tk pen) (resume pen st)))))
                (let ((a (B.spend 4)))
                  (let ((b (B.spend 8)))
                    (let ((c (B.refill 5)))
                      (let ((d (B.spend 8)))
                        (let ((e (B.spend 3)))
                          (let ((f (B.refill 9)))
                            (let ((g (B.pens)))
                              (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 4001008001002 Int64))
  (call   main (: 0 Int64)) (output (: 500031003 Int64)))
