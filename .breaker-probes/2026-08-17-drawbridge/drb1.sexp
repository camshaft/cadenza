(case "drb1 a DRAWBRIDGE over a tidal channel — boat LET-BINDS the low-tide boolean ONCE and consumes it in TWO SEPARATE ifs, one selecting the answer's hundreds digit and one selecting the next-state tuple, a low-tide boat slips under advancing the tide by one and counting the crossing while a high-tide boat waits three tide steps crossing nothing, log reads tide and crossings without advancing, and the seed sets the starting tide so the runs disagree on which boats slip under"
  (input  (do
            (effect L
              (op boat (-> Int64))
              (op log (-> Int64)))
            (def (main (: n Int64))
              (handle L (tuple (% n 3) (: 2 Int64) (: 0 Int64))
                ((boat () st
                  (match st
                    ((tuple tide gate cross)
                      (let ((low (< (% tide 4) gate)))
                        (resume (+ (* (if low (: 2 Int64) (: 7 Int64)) 100)
                                   (+ (* (% tide 10) 10) (% cross 10)))
                                (if low
                                    (tuple (+ tide 1) gate (+ cross 1))
                                    (tuple (+ tide 3) gate cross)))))))
                 (log () st
                  (match st
                    ((tuple tide gate cross)
                      (resume (+ (* tide 10) cross) st)))))
                (let ((a (L.boat)))
                  (let ((b (L.boat)))
                    (let ((c (L.boat)))
                      (let ((d (L.log)))
                        (let ((e (L.boat)))
                          (let ((f (L.boat)))
                            (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 210721251062762292 Int64))
  (call   main (: 0 Int64)) (output (: 200211722052252763 Int64)))
