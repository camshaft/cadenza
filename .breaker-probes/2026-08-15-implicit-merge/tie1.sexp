(case "tie1 a TWO-POINTER merge of IMPLICIT arithmetic streams — the heads bind once through match binders, take answers the smaller advancing that stream's index, a TIE answers fifty-plus-the-value advancing the a-side, and the seed offsets stream a so one run opens with a tie and the other buries its tie mid-stream"
  (input  (do
            (effect M (op take (-> Int64)))
            (def (main (: n Int64))
              (handle M (tuple (: 0 Int64) (: 0 Int64))
                ((take () st
                  (match st
                    ((tuple ai bi)
                      (match (+ (* 3 ai) (+ (% n 4) 1))
                        (ah
                          (match (+ (* 4 bi) 1)
                            (bh
                              (if (= ah bh)
                                  (resume (+ 50 ah) (tuple (+ ai 1) bi))
                                  (if (< ah bh)
                                      (resume ah (tuple (+ ai 1) bi))
                                      (resume bh (tuple ai (+ bi 1)))))))))))))
                (let ((a (M.take)))
                  (let ((b (M.take)))
                    (let ((c (M.take)))
                      (let ((d (M.take)))
                        (let ((e (M.take)))
                          (let ((f (M.take)))
                            (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 10305065909 Int64))
  (call   main (: 0 Int64)) (output (: 510104050709 Int64)))
