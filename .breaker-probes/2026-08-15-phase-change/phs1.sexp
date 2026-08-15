(case "phs1 a PHASE-CHANGE heater — each tick delivers one quantum which either RAISES the temperature by a degree or, exactly at the melting point with latent heat unpaid, pays two units of latent answering a tagged fifty-plus row, and the warm seed climbs straight through while the cold seed hits the plateau and stalls there"
  (input  (do
            (effect P
              (op tick (-> Int64))
              (op stat (-> Int64)))
            (def (main (: n Int64))
              (handle P (tuple (- n 4) (: 0 Int64))
                ((tick () st
                  (match st
                    ((tuple temp latent)
                      (if (= temp 0)
                          (if (< latent 6)
                              (resume (+ 50 (+ latent 2)) (tuple temp (+ latent 2)))
                              (resume (+ temp 1) (tuple (+ temp 1) latent)))
                          (resume (+ temp 1) (tuple (+ temp 1) latent))))))
                 (stat () st
                  (match st
                    ((tuple temp latent) (resume (+ (* temp 10) latent) st)))))
                (let ((a (P.tick)))
                  (let ((b (P.tick)))
                    (let ((c (P.tick)))
                      (let ((d (P.tick)))
                        (let ((e (P.tick)))
                          (let ((f (P.tick)))
                            (let ((g (P.stat)))
                              (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 7080910111320 Int64))
  (call   main (: 0 Int64)) (output (: -3020099474596 Int64)))
