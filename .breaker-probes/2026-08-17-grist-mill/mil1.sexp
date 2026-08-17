(case "mil1 a GRIST MILL riding the wind — grinding in the ideal four-to-eight band banks the full grist (one-tagged), too slow banks HALF coarse (two-tagged), too fast SPOILS the batch entirely (nine-hundred row, the millstone drag slowing the wheel two), gusts add to the wheel, the read packs flour rpm and spoilage, and the seed's starting wheel speed keeps one mill ideal-then-spoiling while the other grinds coarse-then-ideal so no two rows agree"
  (input  (do
            (effect M
              (op gust (-> Int64 Int64))
              (op grind (-> Int64 Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle M (tuple (+ (: 2 Int64) (* (% n 3) 4)) (: 0 Int64) (: 0 Int64))
                ((gust (w) st
                  (match st
                    ((tuple rpm flour sp)
                      (resume (+ (* (+ rpm w) 10) (% w 10)) (tuple (+ rpm w) flour sp)))))
                 (grind (g) st
                  (match st
                    ((tuple rpm flour sp)
                      (if (< rpm 4)
                          (resume (+ (* (/ g 2) 10) 2) (tuple rpm (+ flour (/ g 2)) sp))
                          (if (> rpm 8)
                              (resume (+ (: 900 Int64) (% (+ sp g) 100))
                                      (tuple (- rpm 2) flour (+ sp g)))
                              (resume (+ (* g 10) 1) (tuple rpm (+ flour g) sp)))))))
                 (read () st
                  (match st
                    ((tuple rpm flour sp)
                      (resume (+ (* flour 100) (+ (* rpm 10) (% sp 10))) st)))))
                (let ((a (M.grind (: 4 Int64))))
                  (let ((b (M.gust (: 3 Int64))))
                    (let ((c (M.grind (: 6 Int64))))
                      (let ((d (M.grind (: 5 Int64))))
                        (let ((f (M.read)))
                          (+ (* 10000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 410939060510976 Int64))
  (call   main (: 0 Int64)) (output (: 220530610511350 Int64)))
