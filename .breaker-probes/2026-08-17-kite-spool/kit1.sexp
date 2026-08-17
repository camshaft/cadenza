(case "kit1 a KITE on a spool — paying out line lifts the kite at double rate CEILINGED by the line itself, a gust past the line's end with strength over four SNAPS it (line halved, kite falling to the stub, counted) while a gentle overrun just pulls TAUT (eight-hundred row at the ceiling), the read packs altitude line and snaps, and the seed's spool means the same gusts snap BOTH kites but the final gentle gust pulls taut against DIFFERENT halved stubs"
  (input  (do
            (effect K
              (op payout (-> Int64 Int64))
              (op wind (-> Int64 Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle K (tuple (: 0 Int64) (+ (: 4 Int64) (* (% n 3) 4)) (: 0 Int64))
                ((payout (k) st
                  (match st
                    ((tuple alt line snaps)
                      (if (> (+ alt (* k 2)) (+ line k))
                          (resume (+ (* (+ line k) 10) (% (+ line k) 10))
                                  (tuple (+ line k) (+ line k) snaps))
                          (resume (+ (* (+ alt (* k 2)) 10) (% (+ line k) 10))
                                  (tuple (+ alt (* k 2)) (+ line k) snaps))))))
                 (wind (g) st
                  (match st
                    ((tuple alt line snaps)
                      (if (> (+ alt g) line)
                          (if (> g 4)
                              (resume (+ (: 900 Int64) (+ snaps 1))
                                      (tuple (/ line 2) (/ line 2) (+ snaps 1)))
                              (resume (+ (: 800 Int64) (% line 100))
                                      (tuple line line snaps)))
                          (resume (+ (* (+ alt g) 10) (% g 10))
                                  (tuple (+ alt g) line snaps))))))
                 (read () st
                  (match st
                    ((tuple alt line snaps)
                      (resume (+ (* alt 100) (+ (* line 10) snaps)) st)))))
                (let ((a (K.wind (: 3 Int64))))
                  (let ((b (K.payout (: 3 Int64))))
                    (let ((c (K.wind (: 6 Int64))))
                      (let ((d (K.wind (: 2 Int64))))
                        (let ((f (K.read)))
                          (+ (* 10000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 330919018050551 Int64))
  (call   main (: 0 Int64)) (output (: 330779018030331 Int64)))
