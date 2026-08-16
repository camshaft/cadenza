(case "pop1 a POPCORN kettle — each heat raises the temperature five degrees answering how many NEW kernels popped (thresholds every three degrees from the seed base, the count derived by differencing the running total), bowl reads the total, and the cool kettle pops a single late kernel while the hot one accelerates through five"
  (input  (do
            (effect P
              (op heat (-> Int64))
              (op bowl (-> Int64)))
            (def (totpop (: temp Int64) (: base Int64))
              (if (< temp base) 0 (+ (/ (- temp base) 3) 1)))
            (def (main (: n Int64))
              (handle P (tuple (: 0 Int64) (: 0 Int64))
                ((heat () st
                  (match st
                    ((tuple temp popped)
                      (match (totpop (+ temp 5) (+ 18 n))
                        (tot
                          (resume (- tot popped) (tuple (+ temp 5) tot)))))))
                 (bowl () st
                  (match st ((tuple temp popped) (resume popped st)))))
                (let ((a (P.heat)))
                  (let ((b (P.heat)))
                    (let ((c (P.heat)))
                      (let ((d (P.heat)))
                        (let ((e (P.heat)))
                          (let ((f (P.heat)))
                            (let ((g (P.bowl)))
                              (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 101 Int64))
  (call   main (: 0 Int64)) (output (: 1020205 Int64)))
