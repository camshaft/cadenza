(case "frg1 a FORGE tempering cycle — quenching a piece at five heat or hotter banks hardness of the heat minus three and drops the fire to two, a cold quench CRACKS the piece (counted, nine-hundred row), stoking adds heat capped at ten with a nine-tag, the read packs hardness heat and cracks, and the seed's opening fire quenches clean on one forge and cracks on the other so the hardness ledgers never meet"
  (input  (do
            (effect F
              (op stoke (-> Int64 Int64))
              (op quench (-> Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle F (tuple (+ (: 3 Int64) (* (% n 3) 3)) (: 0 Int64) (: 0 Int64))
                ((stoke (k) st
                  (match st
                    ((tuple heat hard cr)
                      (if (> (+ heat k) 10)
                          (resume (+ (: 100 Int64) 9) (tuple (: 10 Int64) hard cr))
                          (resume (* (+ heat k) 10) (tuple (+ heat k) hard cr))))))
                 (quench () st
                  (match st
                    ((tuple heat hard cr)
                      (if (>= heat 5)
                          (resume (+ (* (- heat 3) 10) (% (+ hard (- heat 3)) 10))
                                  (tuple (: 2 Int64) (+ hard (- heat 3)) cr))
                          (resume (+ (: 900 Int64) (+ cr 1))
                                  (tuple heat hard (+ cr 1)))))))
                 (read () st
                  (match st
                    ((tuple heat hard cr)
                      (resume (+ (* hard 100) (+ (* heat 10) cr)) st)))))
                (let ((a (F.quench)))
                  (let ((b (F.stoke (: 4 Int64))))
                    (let ((c (F.quench)))
                      (let ((d (F.stoke (: 6 Int64))))
                        (let ((f (F.read)))
                          (+ (* 10000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 330600360800680 Int64))
  (call   main (: 0 Int64)) (output (: 9010700440800481 Int64)))
