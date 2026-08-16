(case "rch1 a RATCHET with phase-keyed SLIP — each click advances the phase mod 4 and the position by phase-plus-two, except at the seed's slip-phase where the position drops back to its last multiple of three and answers the dropped position plus fifty; the zero seed slips twice and its second slip is invisible in position but visible in the answer offset and the slip counter"
  (input  (do
            (effect R
              (op click (-> Int64))
              (op slips (-> Int64)))
            (def (main (: n Int64))
              (handle R (tuple (: 0 Int64) (: 0 Int64) (: 0 Int64))
                ((click () st
                  (match st
                    ((tuple pos k s)
                      (if (= k (% n 4))
                          (resume (+ (- pos (% pos 3)) 50)
                                  (tuple (- pos (% pos 3)) (% (+ k 1) 4) (+ s 1)))
                          (resume (+ pos (+ k 2))
                                  (tuple (+ pos (+ k 2)) (% (+ k 1) 4) s))))))
                 (slips () st
                  (match st ((tuple pos k s) (resume s st)))))
                (let ((a (R.click)))
                  (let ((b (R.click)))
                    (let ((c (R.click)))
                      (let ((d (R.click)))
                        (let ((e (R.click)))
                          (let ((f (R.slips)))
                            (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 20553081001 Int64))
  (call   main (: 0 Int64)) (output (: 500307126202 Int64)))
