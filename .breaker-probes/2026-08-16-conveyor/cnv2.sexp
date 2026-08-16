(case "cnv2 the CONVEYOR at three items — quality plus seed bias ships at seven, the four-to-six band gets one rework boost that ships or scraps on the boosted score, below four scraps, tally packs the counters; each seed takes a DIFFERENT branch at every position"
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
                    (let ((c (C.item (: 2 Int64))))
                      (let ((f (C.tally)))
                        (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) f)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 71702030312 Int64))
  (call   main (: 0 Int64)) (output (: 902030020311 Int64)))
