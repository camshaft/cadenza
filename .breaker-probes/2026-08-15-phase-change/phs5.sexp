(case "phs5 inner guard made CONSTANT (latent never checked) — each tick delivers one quantum which either RAISES the temperature by a degree or, exactly at the melting point with latent heat unpaid, pays two units of latent answering a tagged fifty-plus row, and the warm seed climbs straight through while the cold seed hits the plateau and stalls there"
  (input  (do
            (effect P
              (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle P (tuple (+ (% n 6) 1) (: 0 Int64))
                ((tick () st
                  (match st
                    ((tuple temp latent)
                      (if (= temp 3)
                          (if (< 0 6)
                              (resume (+ 50 (+ latent 2)) (tuple temp (+ latent 2)))
                              (resume (+ temp 1) (tuple (+ temp 1) latent)))
                          (resume (+ temp 1) (tuple (+ temp 1) latent))))))
)
                (let ((a (P.tick)))
                  (let ((b (P.tick)))
                    (let ((c (P.tick)))
                      (let ((d (P.tick)))
                        (let ((e (P.tick)))
                          (let ((f (P.tick)))
                            (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 60708091011 Int64))
  (call   main (: 0 Int64)) (output (: 20352545658 Int64)))
