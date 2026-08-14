(case "rrb1 a ROUND-ROBIN scheduler with a skip mask — turn advances cyclically to the next unskipped worker answering its id (or -1 when all four are out), skip marks a worker answering the popcount, and the SEED sets the starting position so the same skip sequence yields different service orders"
  (input  (do
            (effect R
              (op turn (-> Int64))
              (op skip (-> Int64 Int64)))
            (def (bits (: m Int64) (: acc Int64))
              (if (= m 0) acc (bits (>> m 1) (+ acc (& m 1)))))
            (def (scan (: cur Int64) (: mask Int64) (: step Int64))
              (if (< 4 step)
                  -1
                  (if (= (& (>> mask (% (+ cur step) 4)) 1) 0)
                      (% (+ cur step) 4)
                      (scan cur mask (+ step 1)))))
            (def (main (: n Int64))
              (handle R (tuple (% n 4) (: 0 Int64))
                ((turn () st
                  (match st
                    ((tuple cur mask)
                      (if (= (bits mask 0) 4)
                          (resume -1 st)
                          (match (scan cur mask 1)
                            (nxt (resume nxt (tuple nxt mask))))))))
                 (skip (i) st
                  (match st
                    ((tuple cur mask)
                      (resume (bits (| mask (<< 1 i)) 0) (tuple cur (| mask (<< 1 i))))))))
                (let ((a (R.turn)))
                  (let ((b (R.skip 2)))
                    (let ((c (R.turn)))
                      (let ((d (R.turn)))
                        (let ((e (R.skip 0)))
                          (let ((f (R.skip 1)))
                            (let ((g (R.turn)))
                              (let ((h (R.skip 3)))
                                (let ((i (R.turn)))
                                  (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g)) h)) i))))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 30100010203030399 Int64))
  (call   main (: 0 Int64)) (output (: 10103000203030399 Int64)))
