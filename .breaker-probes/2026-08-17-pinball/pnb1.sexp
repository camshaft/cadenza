(case "pnb1 a PINBALL table with a bonus ladder and tilt — a bumper scores its value TIMES the multiplier and crossing a fifty boundary climbs the multiplier capped at three (a seven-hundred row with the new multiplier and the score's low digit), a second nudge would TILT (halving and resetting — dark here, pinned by the tilts-threshold branch), the read packs score multiplier and tilts, and the seed's starting multiplier crosses the fifty boundary on the FIRST bumper for one run and never for the other"
  (input  (do
            (effect P
              (op bumper (-> Int64 Int64))
              (op nudge (-> Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle P (tuple (: 0 Int64) (+ (: 1 Int64) (% n 3)) (: 0 Int64))
                ((bumper (v) st
                  (match st
                    ((tuple score mult tilts)
                      (if (> (/ (+ score (* v mult)) 50) (/ score 50))
                          (if (< mult 3)
                              (resume (+ (: 700 Int64) (+ (* (+ mult 1) 10) (% (+ score (* v mult)) 10)))
                                      (tuple (+ score (* v mult)) (+ mult 1) tilts))
                              (resume (+ (: 700 Int64) (+ (* mult 10) (% (+ score (* v mult)) 10)))
                                      (tuple (+ score (* v mult)) mult tilts)))
                          (resume (+ (* (% (+ score (* v mult)) 100) 10) mult)
                                  (tuple (+ score (* v mult)) mult tilts))))))
                 (nudge () st
                  (match st
                    ((tuple score mult tilts)
                      (if (>= (+ tilts 1) 2)
                          (resume (+ (: 900 Int64) (+ tilts 1))
                                  (tuple (/ score 2) (: 1 Int64) (+ tilts 1)))
                          (resume (+ (: 800 Int64) (+ tilts 1))
                                  (tuple score mult (+ tilts 1)))))))
                 (read () st
                  (match st
                    ((tuple score mult tilts)
                      (resume (+ (* score 100) (+ (* mult 10) tilts)) st)))))
                (let ((a (P.bumper (: 20 Int64))))
                  (let ((b (P.nudge)))
                    (let ((c (P.bumper (: 15 Int64))))
                      (let ((f (P.read)))
                        (+ (* 100000 (+ (* 1000 (+ (* 1000 a) b)) c)) f)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 40280173007031 Int64))
  (call   main (: 0 Int64)) (output (: 20180135103511 Int64)))
