(case "cnv1 a CONVEYOR with a reject gate and one-shot rework — an item's quality plus the seed bias ships at seven, gets ONE rework boost of three in the four-to-six band (shipping or scrapping on the boosted score), or scraps below four, tally packs inspected reworked and shipped; the bias walks the SAME quality feed across all three bands so the branch taken at each position differs per seed"
  (input  (do
            (effect C
              (op item (-> Int64 Int64))
              (op tally (-> Int64)))
            (def (main (: n Int64))
              (handle C (tuple (: 0 Int64) (: 0 Int64) (: 0 Int64))
                ((item (q) st
                  (match st
                    ((tuple ins rew shp)
                      (if (>= (+ q (% n 3)) 7)
                          (resume (+ (* (+ q (% n 3)) 10) 1)
                                  (tuple (+ ins 1) rew (+ shp 1)))
                          (if (>= (+ q (% n 3)) 4)
                              (if (>= (+ (+ q (% n 3)) 3) 7)
                                  (resume (+ (* (+ (+ q (% n 3)) 3) 100) 2)
                                          (tuple (+ ins 1) (+ rew 1) (+ shp 1)))
                                  (resume (* (+ (+ q (% n 3)) 3) 100)
                                          (tuple (+ ins 1) (+ rew 1) shp)))
                              (resume (* (+ q (% n 3)) 10)
                                      (tuple (+ ins 1) rew shp)))))))
                 (tally () st
                  (match st ((tuple ins rew shp) (resume (+ (* ins 100) (+ (* rew 10) shp)) st)))))
                (let ((a (C.item (: 6 Int64))))
                  (let ((b (C.item (: 3 Int64))))
                    (let ((c (C.item (: 7 Int64))))
                      (let ((d (C.item (: 2 Int64))))
                        (let ((e (C.item (: 5 Int64))))
                          (let ((f (C.tally)))
                            (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 71702081030902524 Int64))
  (call   main (: 0 Int64)) (output (: 902030071020802523 Int64)))
