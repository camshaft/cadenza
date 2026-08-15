(case "hnc1 the HANOI smallest-disk cycle — the small disk moves on every ODD move cycling pegs in the seed-picked direction (answering peg-times-ten plus one), even moves answer the small disk's resting peg, count reads the total, and the two directions trace MIRROR peg sequences that re-converge at the mid-cycle zero rows"
  (input  (do
            (effect H
              (op move (-> Int64))
              (op count (-> Int64)))
            (def (main (: n Int64))
              (handle H (tuple (: 0 Int64) (: 0 Int64))
                ((move () st
                  (match st
                    ((tuple peg moves)
                      (if (= (% (+ moves 1) 2) 1)
                          (if (= (% n 3) 1)
                              (resume (+ (* (% (+ peg 1) 3) 10) 1)
                                      (tuple (% (+ peg 1) 3) (+ moves 1)))
                              (resume (+ (* (% (+ peg 2) 3) 10) 1)
                                      (tuple (% (+ peg 2) 3) (+ moves 1))))
                          (resume (* peg 10) (tuple peg (+ moves 1)))))))
                 (count () st
                  (match st ((tuple peg moves) (resume moves st)))))
                (let ((a (H.move)))
                  (let ((b (H.move)))
                    (let ((c (H.move)))
                      (let ((d (H.move)))
                        (let ((e (H.move)))
                          (let ((f (H.move)))
                            (let ((g (H.move)))
                              (let ((h (H.count)))
                                (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g)) h)))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1110212001001107 Int64))
  (call   main (: 0 Int64)) (output (: 2120111001002107 Int64)))
