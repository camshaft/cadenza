(case "qs1 a RUNNING-MAX arm — each feed returns the max so far, four feeds with a mid-run dip and a doubling"
  (input  (do
            (effect E (op feed (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E -1000000
                ((feed (x) s (resume (if (> x s) x s) (if (> x s) x s))))
                (let ((a (E.feed n)))
                  (let ((b (E.feed 5)))
                    (let ((c (E.feed (* 2 n))))
                      (let ((d (E.feed -1)))
                        (+ a (+ (* 10 b) (+ (* 100 c) (* 1000 d))))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 6653 Int64))
  (call   main (: 8 Int64)) (output (: 17688 Int64))
  (call   main (: -6 Int64)) (output (: 5544 Int64)))
