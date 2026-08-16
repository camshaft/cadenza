(case "snk1 a CHUTE WALK — each move advances the position by the input plus a seed bias, but a landing on a multiple of five answers the landing packed with a 5 tag and SLIDES BACK four squares (counting the slide), the closing read packs the final position with the slide count, and the bias decides WHETHER the chute fires mid-run or on the very last move"
  (input  (do
            (effect W
              (op move (-> Int64 Int64))
              (op fin (-> Int64)))
            (def (main (: n Int64))
              (handle W (tuple (: 0 Int64) (: 0 Int64))
                ((move (x) st
                  (match st
                    ((tuple pos s)
                      (if (= (% (+ pos (+ x (% n 3))) 5) 0)
                          (resume (+ (* (+ pos (+ x (% n 3))) 10) 5)
                                  (tuple (- (+ pos (+ x (% n 3))) 4) (+ s 1)))
                          (resume (+ pos (+ x (% n 3)))
                                  (tuple (+ pos (+ x (% n 3))) s))))))
                 (fin () st
                  (match st ((tuple pos s) (resume (+ (* pos 10) s) st)))))
                (let ((a (W.move (: 3 Int64))))
                  (let ((b (W.move (: 4 Int64))))
                    (let ((c (W.move (: 2 Int64))))
                      (let ((d (W.move (: 6 Int64))))
                        (let ((e (W.move (: 5 Int64))))
                          (let ((f (W.fin)))
                            (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 4009012019255211 Int64))
  (call   main (: 0 Int64)) (output (: 3007009155016161 Int64)))
