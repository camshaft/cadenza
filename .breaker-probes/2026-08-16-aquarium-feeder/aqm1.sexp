(case "aqm1 an AQUARIUM feeder with hunger decay and an overfeed penalty — feeding satisfies the LESSER of hunger and portion with the leftover sinking to waste, ticks rebuild hunger by two CAPPED at nine, answers pack eaten hunger and the waste's low digit, the read totals fed hunger and waste, and the seed's starting hunger decides whether either portion overfeeds so the waste trails differ at every row"
  (input  (do
            (effect A
              (op feed (-> Int64 Int64))
              (op tick (-> Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle A (tuple (+ (: 3 Int64) (* (% n 3) 3)) (: 0 Int64) (: 0 Int64))
                ((feed (a) st
                  (match st
                    ((tuple h f w)
                      (if (>= h a)
                          (resume (+ (* a 100) (+ (* (- h a) 10) (% w 10)))
                                  (tuple (- h a) (+ f a) w))
                          (resume (+ (* h 100) (% (+ w (- a h)) 10))
                                  (tuple (: 0 Int64) (+ f h) (+ w (- a h))))))))
                 (tick () st
                  (match st
                    ((tuple h f w)
                      (if (> (+ h 2) 9)
                          (resume (+ (: 90 Int64) w) (tuple (: 9 Int64) f w))
                          (resume (+ (* (+ h 2) 10) w) (tuple (+ h 2) f w))))))
                 (read () st
                  (match st
                    ((tuple h f w)
                      (resume (+ (* f 100) (+ (* h 10) w)) st)))))
                (let ((a (A.feed (: 4 Int64))))
                  (let ((b (A.tick)))
                    (let ((c (A.feed (: 8 Int64))))
                      (let ((d (A.tick)))
                        (let ((f (A.read)))
                          (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 420040404024824 Int64))
  (call   main (: 0 Int64)) (output (: 301021207027527 Int64)))
