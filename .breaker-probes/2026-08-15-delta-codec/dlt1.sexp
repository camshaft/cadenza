(case "dlt1 a DELTA encoder and decoder sharing ONE previous-value slot — enc answers the difference storing the raw value, dec answers the reconstruction storing it, interleaving them CROSS-TALKS through the shared slot by design, and after the seed washes out of the second row the tails converge exactly"
  (input  (do
            (effect D
              (op enc (-> Int64 Int64))
              (op dec (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle D (: 0 Int64)
                ((enc (v) prev (resume (- v prev) v))
                 (dec (d) prev (resume (+ prev d) (+ prev d))))
                (let ((a (D.enc (+ n 4))))
                  (let ((b (D.enc 9)))
                    (let ((c (D.dec 3)))
                      (let ((d (D.enc 20)))
                        (let ((e (D.dec -5)))
                          (let ((f (D.dec 2)))
                            (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 139512081517 Int64))
  (call   main (: 0 Int64)) (output (: 40512081517 Int64)))
