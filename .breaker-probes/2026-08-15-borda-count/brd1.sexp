(case "brd1 a BORDA count over three candidates — each ballot awards two points to its first choice and one to its second answering the first choice's new total, lead answers the current leader with ties to the lowest id, and the seed steers ONE ballot's first choice producing a three-way tie on one run and a runaway winner on the other"
  (input  (do
            (effect B
              (op ballot (-> Int64 Int64 Int64 Int64))
              (op lead (-> Int64)))
            (def (pts-of (: st (Tuple Int64 Int64 Int64)) (: i Int64))
              (match st
                ((tuple x y z) (if (= i 0) x (if (= i 1) y z)))))
            (def (bump (: st (Tuple Int64 Int64 Int64)) (: i Int64) (: by Int64))
              (match st
                ((tuple x y z)
                  (if (= i 0)
                      (tuple (+ x by) y z)
                      (if (= i 1)
                          (tuple x (+ y by) z)
                          (tuple x y (+ z by)))))))
            (def (main (: n Int64))
              (handle B (tuple (: 0 Int64) (: 0 Int64) (: 0 Int64))
                ((ballot (fst snd trd) st
                  (match (bump (bump st fst 2) snd 1)
                    (st2 (resume (pts-of st2 fst) st2))))
                 (lead () st
                  (match st
                    ((tuple x y z)
                      (if (< x y)
                          (if (< y z) (resume 2 st) (resume 1 st))
                          (if (< x z) (resume 2 st) (resume 0 st)))))))
                (let ((a (B.ballot 0 1 2)))
                  (let ((b (B.ballot (% n 3) 2 (if (= (% n 3) 1) 0 1))))
                    (let ((c (B.lead)))
                      (let ((d (B.ballot 2 0 1)))
                        (let ((e (B.lead)))
                          (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 203010300 Int64))
  (call   main (: 0 Int64)) (output (: 204000300 Int64)))
