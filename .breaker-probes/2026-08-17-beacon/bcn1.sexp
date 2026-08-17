(case "bcn1 a BEACON whose sweep answer is a LET CHAIN INSIDE the resume's argument position — the arm binds the turn and the beam octant in nested lets scoped ONLY within the answer expression while the next-state tuple recomputes the same turn and the flash test inline outside their scope, log reads angle parity and flashes, and the seed aims the lamp so the two runs flash from different octants and drift apart by the accumulated turns"
  (input  (do
            (effect L
              (op sweep (-> Int64))
              (op log (-> Int64)))
            (def (main (: n Int64))
              (handle L (tuple (% n 3) (: 0 Int64))
                ((sweep () st
                  (match st
                    ((tuple ang flash)
                      (resume (let ((turn (+ (* ang 3) 1)))
                                (let ((beam (% turn 8)))
                                  (+ (* beam 100)
                                     (+ (* (% turn 10) 10)
                                        (% (if (>= beam 4) (+ flash 1) flash) 10)))))
                              (tuple (+ ang (+ (* ang 3) 1))
                                     (if (>= (% (+ (* ang 3) 1) 8) 4) (+ flash 1) flash))))))
                 (log () st
                  (match st
                    ((tuple ang flash)
                      (resume (+ (* (% ang 10) 10) flash) st)))))
                (let ((a (L.sweep)))
                  (let ((b (L.sweep)))
                    (let ((c (L.log)))
                      (let ((d (L.sweep)))
                        (let ((e (L.sweep)))
                          (let ((f (L.sweep)))
                            (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 441061011041061041 Int64))
  (call   main (: 0 Int64)) (output (: 110441051061041061 Int64)))
