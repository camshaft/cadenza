(case "pid1 an integer P+I CONTROLLER stepping a plant toward the setpoint 20 — each step folds the error into the integral, applies u = (2*err + integral) / 4 with a NEGATIVE dividend on the overshoot face, and the closing draw reads the wound-up integral"
  (input  (do
            (effect P
              (op step (-> Int64))
              (op integ (-> Int64)))
            (def (main (: n Int64))
              (handle P (tuple n (: 0 Int64))
                ((step () st
                  (match st
                    ((tuple pv ig)
                      (match (tuple (- 20 pv) ig)
                        ((tuple err ig2)
                          (match (+ ig2 err)
                            (ig3
                              (match (+ pv (/ (+ (* 2 err) ig3) 4))
                                (pv2 (resume pv2 (tuple pv2 ig3)))))))))))
                 (integ () st
                  (match st ((tuple pv ig) (resume ig st)))))
                (let ((a (P.step)))
                  (let ((b (P.step)))
                    (let ((c (P.step)))
                      (let ((d (P.step)))
                        (let ((e (P.step)))
                          (let ((f (P.integ)))
                            (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 172123232306 Int64))
  (call   main (: 0 Int64)) (output (: 152327272608 Int64)))
