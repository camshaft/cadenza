(case "chr1 a CHAIRLIFT loading two-seat chairs — arrivals join the queue, each passing chair takes the LESSER of the queue and two (an inline min via if) counting itself even when EMPTY, answers pack the take the remaining queue and the chair count, and the seed sizes the first group so one run splits it across a partial chair while the other rides out on the first and sends the second chair up EMPTY"
  (input  (do
            (effect L
              (op arrive (-> Int64 Int64))
              (op chair (-> Int64))
              (op tally (-> Int64)))
            (def (main (: n Int64))
              (handle L (tuple (: 0 Int64) (: 0 Int64) (: 0 Int64))
                ((arrive (k) st
                  (match st
                    ((tuple w l c)
                      (resume (+ (* (+ w k) 10) k) (tuple (+ w k) l c)))))
                 (chair () st
                  (match st
                    ((tuple w l c)
                      (if (>= w 2)
                          (resume (+ (: 200 Int64) (+ (* (- w 2) 10) (% (+ c 1) 10)))
                                  (tuple (- w 2) (+ l 2) (+ c 1)))
                          (resume (+ (* w 100) (+ (* 0 10) (% (+ c 1) 10)))
                                  (tuple (: 0 Int64) (+ l w) (+ c 1)))))))
                 (tally () st
                  (match st
                    ((tuple w l c)
                      (resume (+ (* l 100) (+ (* w 10) c)) st)))))
                (let ((a (L.arrive (+ (: 2 Int64) (% n 3)))))
                  (let ((b (L.chair)))
                    (let ((c (L.chair)))
                      (let ((d (L.arrive (: 1 Int64))))
                        (let ((e (L.chair)))
                          (let ((f (L.tally)))
                            (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 33211102011103403 Int64))
  (call   main (: 0 Int64)) (output (: 22201002011103303 Int64)))
