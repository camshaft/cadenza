(case "prk1 a PARKING-LOT fee meter — enter stamps the time, exit charges the seed-shaped first-hour rate plus two per further hour CAPPED at fifteen (a zero-duration stay is free), rev totals the day, and the cap row is identical across seeds while the uncapped rows and the total differ"
  (input  (do
            (effect P
              (op enter (-> Int64 Int64))
              (op exit (-> Int64 Int64))
              (op rev (-> Int64)))
            (def (main (: n Int64))
              (handle P (tuple (: -1 Int64) (: 0 Int64))
                ((enter (t) st
                  (match st
                    ((tuple entry total) (resume t (tuple t total)))))
                 (exit (t) st
                  (match st
                    ((tuple entry total)
                      (if (< (- t entry) 1)
                          (resume 0 (tuple -1 total))
                          (if (< 15 (+ (+ (% n 4) 2) (* 2 (- (- t entry) 1))))
                              (resume 15 (tuple -1 (+ total 15)))
                              (resume (+ (+ (% n 4) 2) (* 2 (- (- t entry) 1)))
                                      (tuple -1 (+ total (+ (+ (% n 4) 2) (* 2 (- (- t entry) 1)))))))))))
                 (rev () st
                  (match st ((tuple entry total) (resume total st)))))
                (let ((a (P.enter 2)))
                  (let ((b (P.exit 5)))
                    (let ((c (P.enter 6)))
                      (let ((d (P.exit 6)))
                        (let ((e (P.enter 7)))
                          (let ((f (P.exit 20)))
                            (let ((g (P.rev)))
                              (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 2080600071523 Int64))
  (call   main (: 0 Int64)) (output (: 2060600071521 Int64)))
