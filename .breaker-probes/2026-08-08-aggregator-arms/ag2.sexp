(case "ag2 TWO aggregator effects interleaved — each keeps its own running sum, weighted reads pin the four-feed order"
  (input  (do
            (effect P (op feed (-> Int64 Int64)))
            (effect Q (op feed (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle P 0
                ((feed (x) s (resume (+ s x) (+ s x))))
                (handle Q 0
                  ((feed (x) t (resume (+ t x) (+ t x))))
                  (let ((r1 (P.feed n)))
                    (let ((r2 (Q.feed 10)))
                      (let ((r3 (P.feed n)))
                        (let ((r4 (Q.feed 10)))
                          (+ r1 (+ (* 2 r2) (+ (* 3 r3) (* 4 r4)))))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 121 Int64))
  (call   main (: 0 Int64)) (output (: 100 Int64))
  (call   main (: -5 Int64)) (output (: 65 Int64)))
