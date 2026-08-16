(case "gld1 a GOLD-PANNING yield decay — pan answers the current yield then decays it by a third truncating plus one unguarded (the stream never hits the floor), move relocates resetting the yield to the seed base minus a wear cost that grows two per move, total accumulates, and the richer claim decays through DIFFERENT residues while both wear down in lockstep"
  (input  (do
            (effect G
              (op pan (-> Int64))
              (op move (-> Int64))
              (op total (-> Int64)))
            (def (main (: n Int64))
              (handle G (tuple (+ 20 n) (: 0 Int64) (: 0 Int64))
                ((pan () st
                  (match st
                    ((tuple y wear total)
                      (resume y (tuple (- (- y (/ y 3)) 1) wear (+ total y))))))
                 (move () st
                  (match st
                    ((tuple y wear total)
                      (resume (- (+ 20 n) (+ wear 2))
                              (tuple (- (+ 20 n) (+ wear 2)) (+ wear 2) total)))))
                 (total () st
                  (match st ((tuple y wear total) (resume total st)))))
                (let ((a (G.pan)))
                  (let ((b (G.pan)))
                    (let ((c (G.pan)))
                      (let ((d (G.move)))
                        (let ((e (G.pan)))
                          (let ((f (G.pan)))
                            (let ((g (G.move)))
                              (let ((h (G.total)))
                                (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g)) h)))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 3019122828182707 Int64))
  (call   main (: 0 Int64)) (output (: 2013081818111670 Int64)))
