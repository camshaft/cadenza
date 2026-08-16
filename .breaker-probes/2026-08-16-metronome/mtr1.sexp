(case "mtr1 a METRONOME with a seed-keyed time signature — each tick advances the beat which WRAPS at the signature (three or four to the bar) counting the bar and the accent and answering a downbeat row tagged with the signature itself, off-beats answer bar and beat, the report packs accents bars and the live beat, and the same five ticks land the downbeats at DIFFERENT positions per signature"
  (input  (do
            (effect M
              (op tick (-> Int64))
              (op report (-> Int64)))
            (def (main (: n Int64))
              (handle M (tuple (: 0 Int64) (: 0 Int64) (: 0 Int64))
                ((tick () st
                  (match st
                    ((tuple beat bar acc)
                      (if (>= (+ beat 1) (+ 3 (% n 3)))
                          (resume (+ (* (+ bar 1) 100) (+ 90 (+ 3 (% n 3))))
                                  (tuple (: 0 Int64) (+ bar 1) (+ acc 1)))
                          (resume (+ (* bar 100) (* (+ beat 1) 10))
                                  (tuple (+ beat 1) bar acc))))))
                 (report () st
                  (match st
                    ((tuple beat bar acc)
                      (resume (+ (* acc 100) (+ (* bar 10) beat)) st)))))
                (let ((a (M.tick)))
                  (let ((b (M.tick)))
                    (let ((c (M.tick)))
                      (let ((d (M.tick)))
                        (let ((e (M.tick)))
                          (let ((f (M.report)))
                            (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 10020030194110111 Int64))
  (call   main (: 0 Int64)) (output (: 10020193110120112 Int64)))
