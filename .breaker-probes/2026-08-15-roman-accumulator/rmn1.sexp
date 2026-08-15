(case "rmn1 a ROMAN-NUMERAL accumulator with subtractive correction — feeding a numeral LARGER than its predecessor retro-subtracts the predecessor twice (the IX rule), the seed picks the second numeral as five or one, and the correction fires at DIFFERENT positions so the totals cross (nineteen versus twenty-three)"
  (input  (do
            (effect R
              (op feed (-> Int64 Int64))
              (op tot (-> Int64)))
            (def (main (: n Int64))
              (handle R (tuple (: 0 Int64) (: 0 Int64))
                ((feed (v) st
                  (match st
                    ((tuple total prev)
                      (if (< 0 prev)
                          (if (< prev v)
                              (resume (+ total (- v (* 2 prev))) (tuple (+ total (- v (* 2 prev))) v))
                              (resume (+ total v) (tuple (+ total v) v)))
                          (resume (+ total v) (tuple (+ total v) v))))))
                 (tot () st
                  (match st ((tuple total prev) (resume total st)))))
                (let ((a (R.feed 10)))
                  (let ((b (R.feed (if (= (% n 3) 1) 5 1))))
                    (let ((c (R.feed 10)))
                      (let ((d (R.feed 1)))
                        (let ((e (R.feed 5)))
                          (let ((f (R.tot)))
                            (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 101515161919 Int64))
  (call   main (: 0 Int64)) (output (: 101119202323 Int64)))
