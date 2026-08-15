(case "rps2 the RPS judge with the opponent move HOISTED through match binders — the LCG advance and the mod-3 move bind once per dispatch instead of recomputing per branch, same protocol and answers"
  (input  (do
            (effect R
              (op play (-> Int64 Int64))
              (op score (-> Int64)))
            (def (main (: n Int64))
              (handle R (tuple (+ n 3) (: 0 Int64) (: 0 Int64))
                ((play (mine) st
                  (match st
                    ((tuple seed wins losses)
                      (match (% (+ (* seed 5) 3) 16)
                        (s2
                          (match (% s2 3)
                            (o
                              (if (= mine o)
                                  (resume 0 (tuple s2 wins losses))
                                  (if (= (% (+ (- mine o) 3) 3) 1)
                                      (resume 1 (tuple s2 (+ wins 1) losses))
                                      (resume -1 (tuple s2 wins (+ losses 1))))))))))))
                 (score () st
                  (match st
                    ((tuple seed wins losses) (resume (+ (* wins 10) losses) st)))))
                (let ((a (R.play 0)))
                  (let ((b (R.play 1)))
                    (let ((c (R.play 2)))
                      (let ((d (R.play 0)))
                        (let ((e (R.score)))
                          (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: -100010097 Int64))
  (call   main (: 0 Int64)) (output (: 100009921 Int64)))
