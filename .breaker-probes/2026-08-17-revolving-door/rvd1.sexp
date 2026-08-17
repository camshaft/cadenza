(case "rvd1 a REVOLVING DOOR of four segments — a group enters only if it fits the remaining segments (a jam turns the whole group away counting itself), a spin releases every occupant and counts the revolution packing the released headcount with the revolution's low digit, the read packs revolutions jams and occupancy, and the seed sizes the FIRST group so the second entry jams on one run and rides on the other"
  (input  (do
            (effect D
              (op enter (-> Int64 Int64))
              (op spin (-> Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle D (tuple (: 0 Int64) (: 0 Int64) (: 0 Int64))
                ((enter (k) st
                  (match st
                    ((tuple occ revs jams)
                      (if (<= (+ occ k) 4)
                          (resume (+ (* (+ occ k) 10) k) (tuple (+ occ k) revs jams))
                          (resume (+ (: 900 Int64) (+ jams 1)) (tuple occ revs (+ jams 1)))))))
                 (spin () st
                  (match st
                    ((tuple occ revs jams)
                      (resume (+ (: 100 Int64) (+ (* occ 10) (% (+ revs 1) 10)))
                              (tuple (: 0 Int64) (+ revs 1) jams)))))
                 (read () st
                  (match st
                    ((tuple occ revs jams)
                      (resume (+ (* revs 100) (+ (* jams 10) occ)) st)))))
                (let ((a (D.enter (+ (: 2 Int64) (% n 3)))))
                  (let ((b (D.enter (: 2 Int64))))
                    (let ((c (D.spin)))
                      (let ((d (D.enter (: 3 Int64))))
                        (let ((e (D.enter (: 2 Int64))))
                          (let ((f (D.read)))
                            (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 33901131033902123 Int64))
  (call   main (: 0 Int64)) (output (: 22042141033901113 Int64)))
