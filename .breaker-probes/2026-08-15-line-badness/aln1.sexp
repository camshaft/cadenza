(case "aln1 a LINE-BREAK badness accumulator — feeding a word that would overflow the width-twelve line FLUSHES first answering the flushed line's slack SQUARED, a fitting word answers zero, flush forces the partial line, rdbad totals, and the seed word's width shifts the break point so the same overflow pays a different squared penalty"
  (input  (do
            (effect L
              (op feed (-> Int64 Int64))
              (op flush (-> Int64))
              (op rdbad (-> Int64)))
            (def (main (: n Int64))
              (handle L (tuple (: 0 Int64) (: 0 Int64))
                ((feed (w) st
                  (match st
                    ((tuple line bad)
                      (if (= line 0)
                          (resume 0 (tuple w bad))
                          (if (< 12 (+ line (+ 1 w)))
                              (resume (* (- 12 line) (- 12 line))
                                      (tuple w (+ bad (* (- 12 line) (- 12 line)))))
                              (resume 0 (tuple (+ line (+ 1 w)) bad)))))))
                 (flush () st
                  (match st
                    ((tuple line bad)
                      (if (= line 0)
                          (resume -1 st)
                          (resume (* (- 12 line) (- 12 line))
                                  (tuple 0 (+ bad (* (- 12 line) (- 12 line)))))))))
                 (rdbad () st
                  (match st ((tuple line bad) (resume bad st)))))
                (let ((a (L.feed (+ (% n 4) 2))))
                  (let ((b (L.feed 4)))
                    (let ((c (L.feed 5)))
                      (let ((d (L.feed 2)))
                        (let ((e (L.flush)))
                          (let ((f (L.feed 12)))
                            (let ((g (L.flush)))
                              (let ((h (L.rdbad)))
                                (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e)) f)) g)) h)))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 9000016000000025 Int64))
  (call   main (: 0 Int64)) (output (: 25000016000000041 Int64)))
