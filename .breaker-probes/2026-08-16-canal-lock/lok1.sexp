(case "lok1 a CANAL lock filling toward a seed pool level — enter answers the full gap, each equalize raises the lock two clamped at the pool answering the new level, exit answers a hundred plus the trip count when the levels MATCH or the remaining gap negated, and the low pool completes passage on the third equalize while the high pool is still twelve short at the same row"
  (input  (do
            (effect L
              (op enter (-> Int64))
              (op equalize (-> Int64))
              (op exit (-> Int64)))
            (def (main (: n Int64))
              (handle L (tuple (: 0 Int64) (: 0 Int64))
                ((enter () st
                  (match st
                    ((tuple lock trips) (resume (- (+ n 6) lock) st))))
                 (equalize () st
                  (match st
                    ((tuple lock trips)
                      (if (< (+ lock 2) (+ n 6))
                          (resume (+ lock 2) (tuple (+ lock 2) trips))
                          (resume (+ n 6) (tuple (+ n 6) trips))))))
                 (exit () st
                  (match st
                    ((tuple lock trips)
                      (if (= lock (+ n 6))
                          (resume (+ 101 trips) (tuple lock (+ trips 1)))
                          (resume (- lock (+ n 6)) st))))))
                (let ((a (L.enter)))
                  (let ((b (L.equalize)))
                    (let ((c (L.equalize)))
                      (let ((d (L.exit)))
                        (let ((e (L.equalize)))
                          (let ((f (L.exit)))
                            (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 160203880590 Int64))
  (call   main (: 0 Int64)) (output (: 60203980701 Int64)))
