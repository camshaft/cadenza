(case "prm1 a PRISM bench refracting a beam — aiming adds the angle mod seven where a ZERO exit is absorbed (counted, beam surviving unchanged), a split HALVES the beam and a beam split to darkness RE-LIGHTS at five (seven-hundred row with the split count), the read packs beam splits and absorbed, and the seed's beam is absorbed mid-run on one bench but split to darkness on the other so the same ops thread entirely different light paths"
  (input  (do
            (effect P
              (op aim (-> Int64 Int64))
              (op split (-> Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle P (tuple (+ (: 3 Int64) (* (% n 3) 2)) (: 0 Int64) (: 0 Int64))
                ((aim (a) st
                  (match st
                    ((tuple beam sp ab)
                      (if (= (% (+ beam a) 7) 0)
                          (resume (+ (: 900 Int64) (+ ab 1)) (tuple beam sp (+ ab 1)))
                          (resume (+ (* (% (+ beam a) 7) 10) (% a 10))
                                  (tuple (% (+ beam a) 7) sp ab))))))
                 (split () st
                  (match st
                    ((tuple beam sp ab)
                      (if (= (/ beam 2) 0)
                          (resume (+ (: 700 Int64) (+ sp 1)) (tuple (: 5 Int64) (+ sp 1) ab))
                          (resume (+ (* (/ beam 2) 10) (% (+ sp 1) 10))
                                  (tuple (/ beam 2) (+ sp 1) ab))))))
                 (read () st
                  (match st
                    ((tuple beam sp ab)
                      (resume (+ (* beam 100) (+ (* sp 10) ab)) st)))))
                (let ((a (P.aim (: 3 Int64))))
                  (let ((b (P.split)))
                    (let ((c (P.aim (: 4 Int64))))
                      (let ((d (P.split)))
                        (let ((f (P.read)))
                          (+ (* 10000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 137010240120120 Int64))
  (call   main (: 0 Int64)) (output (: 630319010120121 Int64)))
