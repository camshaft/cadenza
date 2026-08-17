(case "lnd2 the LAUNDROMAT at four ops — the advance arm makes two sequential conditional moves (dryer empties into done, washer hands to dryer) across four leaves, a leftover seed load refuses one run's first wash, and the phase shift survives to the read"
  (input  (do
            (effect L
              (op load (-> Int64 Int64))
              (op advance (-> Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle L (tuple (* (% n 3) 2) (: 0 Int64) (: 0 Int64))
                ((load (k) st
                  (match st
                    ((tuple w d dn)
                      (if (= w 0)
                          (resume (+ (* k 10) 1) (tuple k d dn))
                          (resume (+ (: 900 Int64) w) st)))))
                 (advance () st
                  (match st
                    ((tuple w d dn)
                      (if (> d 0)
                          (if (> w 0)
                              (resume (+ (* w 10) (% (+ dn d) 10)) (tuple (: 0 Int64) w (+ dn d)))
                              (resume (% (+ dn d) 10) (tuple (: 0 Int64) (: 0 Int64) (+ dn d))))
                          (if (> w 0)
                              (resume (+ (* w 10) (% dn 10)) (tuple (: 0 Int64) w dn))
                              (resume (% dn 10) st))))))
                 (read () st
                  (match st
                    ((tuple w d dn)
                      (resume (+ (* dn 100) (+ (* w 10) d)) st)))))
                (let ((a (L.load (: 3 Int64))))
                  (let ((b (L.advance)))
                    (let ((c (L.load (: 4 Int64))))
                      (let ((d (L.advance)))
                        (let ((f (L.read)))
                          (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 902020041042204 Int64))
  (call   main (: 0 Int64)) (output (: 31030041043304 Int64)))
