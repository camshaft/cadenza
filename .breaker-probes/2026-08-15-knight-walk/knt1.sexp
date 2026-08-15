(case "knt1 a KNIGHT walker on an eight-by-eight board — hop attempts the L-move answering the landing square's index or -1 REFUSED off-board with the position held, cnt reads the completed-hop count, and the seed's starting rank makes the SAME move list bounce twice on one run and once on the other"
  (input  (do
            (effect W
              (op hop (-> Int64 Int64 Int64))
              (op cnt (-> Int64)))
            (def (main (: n Int64))
              (handle W (tuple (% n 8) (: 3 Int64) (: 0 Int64))
                ((hop (dr dc) st
                  (match st
                    ((tuple r c hops)
                      (if (< (+ r dr) 0)
                          (resume -1 st)
                          (if (< 7 (+ r dr))
                              (resume -1 st)
                              (if (< (+ c dc) 0)
                                  (resume -1 st)
                                  (if (< 7 (+ c dc))
                                      (resume -1 st)
                                      (resume (+ (* (+ r dr) 8) (+ c dc))
                                              (tuple (+ r dr) (+ c dc) (+ hops 1))))))))))
                 (cnt () st
                  (match st ((tuple r c hops) (resume hops st)))))
                (let ((a (W.hop -2 1)))
                  (let ((b (W.hop -1 -2)))
                    (let ((c (W.hop 2 -1)))
                      (let ((d (W.hop -2 -1)))
                        (let ((e (W.cnt)))
                          (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 399190203 Int64))
  (call   main (: 0 Int64)) (output (: -100819898 Int64)))
