(case "rps1 a ROCK-PAPER-SCISSORS judge against an LCG opponent — play advances the hidden LCG, judges by the mod-3 cyclic-difference rule answering +1/0/-1, and score packs wins and losses; the seed steers the opponent so one run mostly loses (negative packed total) while the other mostly wins"
  (input  (do
            (effect R
              (op play (-> Int64 Int64))
              (op score (-> Int64)))
            (def (main (: n Int64))
              (handle R (tuple (+ n 3) (: 0 Int64) (: 0 Int64))
                ((play (mine) st
                  (match st
                    ((tuple seed wins losses)
                      (if (= mine (% (% (+ (* seed 5) 3) 16) 3))
                          (resume 0 (tuple (% (+ (* seed 5) 3) 16) wins losses))
                          (if (= (% (+ (- mine (% (% (+ (* seed 5) 3) 16) 3)) 3) 3) 1)
                              (resume 1 (tuple (% (+ (* seed 5) 3) 16) (+ wins 1) losses))
                              (resume -1 (tuple (% (+ (* seed 5) 3) 16) wins (+ losses 1))))))))
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
