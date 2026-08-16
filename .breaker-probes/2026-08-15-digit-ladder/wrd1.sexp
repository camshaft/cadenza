(case "wrd1 a DIGIT-LADDER distance scorer — feed answers the per-digit Hamming distance between the three-digit guess and the seed-picked target computed by a recursive digit-walk callee, closest tracks the minimum, and the two targets rank the SAME guesses in different orders with each run holding a different exact-hit row"
  (input  (do
            (effect W
              (op feed (-> Int64 Int64))
              (op closest (-> Int64)))
            (def (ddist (: a Int64) (: b Int64) (: k Int64) (: acc Int64))
              (if (= k 0)
                  acc
                  (if (= (% a 10) (% b 10))
                      (ddist (/ a 10) (/ b 10) (- k 1) acc)
                      (ddist (/ a 10) (/ b 10) (- k 1) (+ acc 1)))))
            (def (main (: n Int64))
              (handle W (tuple (if (= (% n 3) 1) 345 375) (: 3 Int64))
                ((feed (w) st
                  (match st
                    ((tuple target best)
                      (match (ddist w target 3 0)
                        (d
                          (if (< d best)
                              (resume d (tuple target d))
                              (resume d (tuple target best))))))))
                 (closest () st
                  (match st ((tuple target best) (resume best st)))))
                (let ((a (W.feed 345)))
                  (let ((b (W.feed 325)))
                    (let ((c (W.feed 375)))
                      (let ((d (W.feed 340)))
                        (let ((e (W.closest)))
                          (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1010100 Int64))
  (call   main (: 0 Int64)) (output (: 101000200 Int64)))
