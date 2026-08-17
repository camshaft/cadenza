(case "dvb1 a DIVING BOARD judge panel — each dive scores triple its difficulty MINUS half the standing mark (the judges grade against the last score), beating the mark extends the improving streak else it resets, the read packs total mark and streak, and the seed's opening mark drags every following score through the feedback so the streaks never align"
  (input  (do
            (effect D
              (op dive (-> Int64 Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle D (tuple (+ (: 4 Int64) (* (% n 3) 4)) (: 0 Int64) (: 0 Int64))
                ((dive (d) st
                  (match st
                    ((tuple last streak total)
                      (if (> (- (* d 3) (/ last 2)) last)
                          (resume (+ (* (- (* d 3) (/ last 2)) 10) (+ streak 1))
                                  (tuple (- (* d 3) (/ last 2)) (+ streak 1) (+ total (- (* d 3) (/ last 2)))))
                          (resume (* (- (* d 3) (/ last 2)) 10)
                                  (tuple (- (* d 3) (/ last 2)) (: 0 Int64) (+ total (- (* d 3) (/ last 2)))))))))
                 (read () st
                  (match st
                    ((tuple last streak total)
                      (resume (+ (* total 100) (+ (* last 10) streak)) st)))))
                (let ((a (D.dive (: 3 Int64))))
                  (let ((b (D.dive (: 4 Int64))))
                    (let ((c (D.dive (: 2 Int64))))
                      (let ((f (D.read)))
                        (+ (* 10000 (+ (* 1000 (+ (* 1000 a) b)) c)) f)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 501010101610 Int64))
  (call   main (: 0 Int64)) (output (: 710920201820 Int64)))
