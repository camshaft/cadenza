(case "ftn1 a WISHING FOUNTAIN with city skimming — each toss adds coins and a wish but a pool over twelve is SKIMMED down to eight (the seven-hundred row carrying the skim amount and the wish's low digit), a scoop takes the lesser of three and the pool, the read packs coins skimmed and wishes, and the seed's starting pool crosses the skim line on a DIFFERENT toss so the skim amounts and every later pool level diverge"
  (input  (do
            (effect F
              (op toss (-> Int64 Int64))
              (op scoop (-> Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle F (tuple (+ (: 5 Int64) (* (% n 3) 4)) (: 0 Int64) (: 0 Int64))
                ((toss (v) st
                  (match st
                    ((tuple coins sk w)
                      (if (> (+ coins v) 12)
                          (resume (+ (: 700 Int64) (+ (* (- (+ coins v) 8) 10) (% (+ w 1) 10)))
                                  (tuple (: 8 Int64) (+ sk (- (+ coins v) 8)) (+ w 1)))
                          (resume (+ (* (+ coins v) 10) (% (+ w 1) 10))
                                  (tuple (+ coins v) sk (+ w 1)))))))
                 (scoop () st
                  (match st
                    ((tuple coins sk w)
                      (if (< coins 3)
                          (resume (+ (* coins 10) 0) (tuple (: 0 Int64) sk w))
                          (resume (+ (: 30 Int64) (% (- coins 3) 10)) (tuple (- coins 3) sk w))))))
                 (read () st
                  (match st
                    ((tuple coins sk w)
                      (resume (+ (* coins 100) (+ (* sk 10) w)) st)))))
                (let ((a (F.toss (: 3 Int64))))
                  (let ((b (F.scoop)))
                    (let ((c (F.toss (: 4 Int64))))
                      (let ((d (F.toss (: 2 Int64))))
                        (let ((f (F.read)))
                          (+ (* 10000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1210397521031053 Int64))
  (call   main (: 0 Int64)) (output (: 810350921131103 Int64)))
