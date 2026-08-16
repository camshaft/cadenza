(case "mrt1 a METRONOME with a downbeat accent — tick advances the beat wrapping at the seed-shaped bar length (counting bars), accent answers a hundred plus the bar count ON a downbeat or the beats remaining until the next one, and the four-beat bar catches the second accent ON the downbeat while the three-beat bar misses both"
  (input  (do
            (effect M
              (op tick (-> Int64))
              (op accent (-> Int64)))
            (def (main (: n Int64))
              (handle M (tuple (: 0 Int64) (: 0 Int64))
                ((tick () st
                  (match st
                    ((tuple beat bars)
                      (if (< (+ (% n 3) 3) (+ beat 1))
                          (resume 1 (tuple 1 (+ bars 1)))
                          (resume (+ beat 1) (tuple (+ beat 1) bars))))))
                 (accent () st
                  (match st
                    ((tuple beat bars)
                      (if (= beat 1)
                          (resume (+ 100 bars) st)
                          (resume (- (+ (% n 3) 4) beat) st))))))
                (let ((a (M.tick)))
                  (let ((b (M.tick)))
                    (let ((c (M.tick)))
                      (let ((d (M.accent)))
                        (let ((e (M.tick)))
                          (let ((f (M.tick)))
                            (let ((g (M.accent)))
                              (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1020302040201 Int64))
  (call   main (: 0 Int64)) (output (: 1020301010202 Int64)))
