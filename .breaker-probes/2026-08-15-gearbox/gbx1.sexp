(case "gbx1 a THREE-SPEED gearbox odometer — shift moves the gear clamped to the one-to-three range answering the landed gear, drive accrues time-times-the-gear's-speed into the odometer answering the total, and the seed's starting gear compounds through every drive so the odometers pull apart while the shift answers converge"
  (input  (do
            (effect G
              (op shift (-> Int64 Int64))
              (op drive (-> Int64 Int64)))
            (def (speed (: g Int64))
              (if (= g 1) 2 (if (= g 2) 5 9)))
            (def (main (: n Int64))
              (handle G (tuple (+ (% n 3) 1) (: 0 Int64))
                ((shift (d) st
                  (match st
                    ((tuple gear odo)
                      (if (< (+ gear d) 1)
                          (resume 1 (tuple 1 odo))
                          (if (< 3 (+ gear d))
                              (resume 3 (tuple 3 odo))
                              (resume (+ gear d) (tuple (+ gear d) odo)))))))
                 (drive (t) st
                  (match st
                    ((tuple gear odo)
                      (resume (+ odo (* t (speed gear))) (tuple gear (+ odo (* t (speed gear)))))))))
                (let ((a (G.drive 2)))
                  (let ((b (G.shift 1)))
                    (let ((c (G.drive 3)))
                      (let ((d (G.shift 1)))
                        (let ((e (G.drive 1)))
                          (let ((f (G.shift -2)))
                            (let ((g (G.drive 4)))
                              (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 10033703460154 Int64))
  (call   main (: 0 Int64)) (output (: 4021903280136 Int64)))
