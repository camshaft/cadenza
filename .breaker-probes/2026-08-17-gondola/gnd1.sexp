(case "gnd1 a GONDOLA on a reflecting cable — move advances the car by the signed direction held in state binding the new position ONCE in a let consumed by the terminus test both answers and both next-states, REFLECTS at either terminus by negating the direction and counting the trip, answers pack the let-bound position an arithmetic direction bit and the trip count, span reads position and trips without advancing, and the seed places the start so one run reflects mid-sequence while the other reaches the far terminus one move later"
  (input  (do
            (effect L
              (op move (-> Int64))
              (op span (-> Int64)))
            (def (main (: n Int64))
              (handle L (tuple (% n 3) (: 1 Int64) (: 0 Int64))
                ((move () st
                  (match st
                    ((tuple pos dir trips)
                      (let ((np (+ pos dir)))
                        (if (if (= np 4) true (= np 0))
                            (resume (+ (* np 100) (+ (* (/ (+ dir 1) 2) 10) (% (+ trips 1) 10)))
                                    (tuple np (- 0 dir) (+ trips 1)))
                            (resume (+ (* np 100) (+ (* (/ (+ dir 1) 2) 10) (% trips 10)))
                                    (tuple np dir trips)))))))
                 (span () st
                  (match st
                    ((tuple pos dir trips)
                      (resume (+ (* pos 10) trips) st)))))
                (let ((a (L.move)))
                  (let ((b (L.move)))
                    (let ((c (L.span)))
                      (let ((d (L.move)))
                        (let ((e (L.move)))
                          (let ((f (L.move)))
                            (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 210310030411301201 Int64))
  (call   main (: 0 Int64)) (output (: 110210020310411301 Int64)))
