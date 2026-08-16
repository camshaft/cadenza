(case "prt1 a PRINTER spooler with two priorities and a jam toggle — print drains the high queue before the low one counting pages (a jammed printer answers three nines touching nothing and an empty spool answers zero), submit files into the queue its priority names, the jam toggle reports its new state with the page count, and the seed pre-loads the LOW queue so the first print drains a low job on one run and finds the spool EMPTY on the other"
  (input  (do
            (effect P
              (op submit (-> Int64 Int64))
              (op print (-> Int64))
              (op jam (-> Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle P (tuple (: 0 Int64) (% n 3) (: 0 Int64) (: 0 Int64))
                ((submit (p) st
                  (match st
                    ((tuple hi lo j pr)
                      (if (= p 1)
                          (resume (+ (* (+ hi 1) 10) lo) (tuple (+ hi 1) lo j pr))
                          (resume (+ (* hi 10) (+ lo 1)) (tuple hi (+ lo 1) j pr))))))
                 (print () st
                  (match st
                    ((tuple hi lo j pr)
                      (if (= j 1)
                          (resume (: 999 Int64) st)
                          (if (> hi 0)
                              (resume (+ (: 100 Int64) (+ pr 1)) (tuple (- hi 1) lo j (+ pr 1)))
                              (if (> lo 0)
                                  (resume (+ (: 200 Int64) (+ pr 1)) (tuple hi (- lo 1) j (+ pr 1)))
                                  (resume (: 0 Int64) st)))))))
                 (jam () st
                  (match st
                    ((tuple hi lo j pr)
                      (resume (+ (* (- 1 j) 10) pr) (tuple hi lo (- 1 j) pr)))))
                 (read () st
                  (match st
                    ((tuple hi lo j pr)
                      (resume (+ (* hi 100) (+ (* lo 10) pr)) st)))))
                (let ((a (P.print)))
                  (let ((b (P.submit (: 1 Int64))))
                    (let ((c (P.jam)))
                      (let ((d (P.print)))
                        (let ((e (P.jam)))
                          (let ((f (P.read)))
                            (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 201010011999001101 Int64))
  (call   main (: 0 Int64)) (output (: 10010999000100 Int64)))
