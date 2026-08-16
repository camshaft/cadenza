(case "brw1 a BREW fermenter with a stuck-fermentation rescue — each day the gravity drops by the yeast health floored at ten (an inline min), a drop under three counts as STUCK earning a four-point yeast boost and a seven-hundred alarm with the standing gravity, a healthy day answers gravity and the yeast's low digit, pitching adds yeast, the read packs all three, and the weak seed sticks on DAY ONE (rescuing early) while the strong one ferments clean until the floor"
  (input  (do
            (effect F
              (op day (-> Int64))
              (op pitch (-> Int64 Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle F (tuple (: 60 Int64) (+ (: 2 Int64) (* (% n 3) 3)) (: 0 Int64))
                ((day () st
                  (match st
                    ((tuple g y r)
                      (if (< (if (< y (- g 10)) y (- g 10)) 3)
                          (resume (+ (: 700 Int64) (- g (if (< y (- g 10)) y (- g 10))))
                                  (tuple (- g (if (< y (- g 10)) y (- g 10))) (+ y 4) (+ r 1)))
                          (resume (+ (* (- g (if (< y (- g 10)) y (- g 10))) 10) (% y 10))
                                  (tuple (- g (if (< y (- g 10)) y (- g 10))) y r))))))
                 (pitch (v) st
                  (match st
                    ((tuple g y r)
                      (resume (* (+ y v) 10) (tuple g (+ y v) r)))))
                 (read () st
                  (match st
                    ((tuple g y r)
                      (resume (+ (* g 100) (+ (* y 10) r)) st)))))
                (let ((a (F.day)))
                  (let ((b (F.day)))
                    (let ((c (F.pitch (: 1 Int64))))
                      (let ((d (F.day)))
                        (let ((f (F.read)))
                          (+ (* 10000 (+ (* 10000 (+ (* 10000 (+ (* 10000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 5550505006004464460 Int64))
  (call   main (: 0 Int64)) (output (: 7580526007004574571 Int64)))
