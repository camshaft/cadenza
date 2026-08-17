(case "ptc1 a PORTCULLIS on a ratcheted windlass — cranking lifts two capped at eight straining harder near the top (height over four), a release DROPS the gate to the floor unless the pawl holds it (eight-hundred hold vs seven-hundred fall echoing the lost height), setting the pawl answers it with the height's low digit, the read packs height strain and pawl, and the seed's initial pawl makes the FIRST release hold and the second fall on one run and exactly the reverse on the other"
  (input  (do
            (effect P
              (op crank (-> Int64))
              (op release (-> Int64))
              (op pawl (-> Int64 Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle P (tuple (: 0 Int64) (if (> (% n 3) 0) 1 0) (: 0 Int64))
                ((crank () st
                  (match st
                    ((tuple h pw sn)
                      (if (> (+ h 2) 8)
                          (resume (+ (: 80 Int64) (% (+ sn 2) 10)) (tuple (: 8 Int64) pw (+ sn 2)))
                          (resume (+ (* (+ h 2) 10) (% (+ sn (/ (+ h 2) 4)) 10))
                                  (tuple (+ h 2) pw (+ sn (/ (+ h 2) 4))))))))
                 (release () st
                  (match st
                    ((tuple h pw sn)
                      (if (= pw 1)
                          (resume (+ (: 800 Int64) h) st)
                          (resume (+ (: 700 Int64) h) (tuple (: 0 Int64) pw sn))))))
                 (pawl (p) st
                  (match st
                    ((tuple h pw sn)
                      (resume (+ (* p 10) (% h 10)) (tuple h p sn)))))
                 (read () st
                  (match st
                    ((tuple h pw sn)
                      (resume (+ (* h 100) (+ (* sn 10) pw)) st)))))
                (let ((a (P.crank)))
                  (let ((b (P.crank)))
                    (let ((c (P.release)))
                      (let ((d (P.pawl (if (> (% n 3) 0) 0 1))))
                        (let ((e (P.release)))
                          (let ((f (P.read)))
                            (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 20041804004704010 Int64))
  (call   main (: 0 Int64)) (output (: 20041704010800011 Int64)))
