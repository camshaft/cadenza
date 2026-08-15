(case "tow1 a TUG-OF-WAR marker with a latching win line — each pull moves the marker toward its side answering the position until crossing plus-or-minus ten LATCHES the result (every later pull answers the frozen plus-or-minus hundred), where reads the final marker, and the offset seed loses THREE pulls early so its frozen tail overlaps the other run's still-live rows"
  (input  (do
            (effect T
              (op pull (-> Int64 Int64 Int64))
              (op where (-> Int64)))
            (def (main (: n Int64))
              (handle T (tuple (- (% n 4) 2) (: 0 Int64))
                ((pull (side s) st
                  (match st
                    ((tuple pos won)
                      (if (< 0 won)
                          (resume 100 st)
                          (if (< won 0)
                              (resume -100 st)
                              (if (= side 1)
                                  (if (< 9 (+ pos s))
                                      (resume 100 (tuple (+ pos s) 1))
                                      (resume (+ pos s) (tuple (+ pos s) 0)))
                                  (if (< (- pos s) -9)
                                      (resume -100 (tuple (- pos s) -1))
                                      (resume (- pos s) (tuple (- pos s) 0)))))))))
                 (where () st
                  (match st ((tuple pos won) (resume pos st)))))
                (let ((a (T.pull 1 4)))
                  (let ((b (T.pull -1 7)))
                    (let ((c (T.pull -1 6)))
                      (let ((d (T.pull 1 3)))
                        (let ((e (T.pull -1 5)))
                          (let ((f (T.where)))
                            (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 39690929989 Int64))
  (call   main (: 0 Int64)) (output (: 19398989989 Int64)))
