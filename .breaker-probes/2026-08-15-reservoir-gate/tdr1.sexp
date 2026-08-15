(case "tdr1 a RESERVOIR gate with a ratcheting threshold — inflow raises the level, gate releases HALF truncating only above the threshold (which ratchets up by one after every release), and the LOWER starting threshold releases on the very first gate while the higher one holds, the runs re-converging on the final held gate"
  (input  (do
            (effect R
              (op inflow (-> Int64 Int64))
              (op gate (-> Int64)))
            (def (main (: n Int64))
              (handle R (tuple (: 0 Int64) (+ (% n 4) 4))
                ((inflow (v) st
                  (match st
                    ((tuple level thresh) (resume (+ level v) (tuple (+ level v) thresh)))))
                 (gate () st
                  (match st
                    ((tuple level thresh)
                      (if (< thresh level)
                          (resume (/ level 2)
                                  (tuple (- level (/ level 2)) (+ thresh 1)))
                          (resume 0 st))))))
                (let ((a (R.inflow 5)))
                  (let ((b (R.gate)))
                    (let ((c (R.inflow 4)))
                      (let ((d (R.gate)))
                        (let ((e (R.inflow 6)))
                          (let ((f (R.gate)))
                            (let ((g (R.gate)))
                              (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 5000904110500 Int64))
  (call   main (: 0 Int64)) (output (: 5020703100500 Int64)))
