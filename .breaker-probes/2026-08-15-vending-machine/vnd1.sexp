(case "vnd1 a VENDING machine with a change float — insert accumulates credit, buy vends only when the credit covers the seven-cent price AND the float covers the change (answering the change, growing the float by price minus change, zeroing credit), refusing otherwise with distinct negated codes, and the seed's starting float decides whether ANY sale completes"
  (input  (do
            (effect V
              (op insert (-> Int64 Int64))
              (op buy (-> Int64)))
            (def (main (: n Int64))
              (handle V (tuple (: 0 Int64) (% n 4))
                ((insert (c) st
                  (match st
                    ((tuple credit fl) (resume (+ credit c) (tuple (+ credit c) fl)))))
                 (buy () st
                  (match st
                    ((tuple credit fl)
                      (if (< credit 7)
                          (resume (- credit 7) st)
                          (if (< fl (- credit 7))
                              (resume (- (- fl (- credit 7)) 50) st)
                              (resume (- credit 7)
                                      (tuple 0 (+ fl (- 7 (- credit 7)))))))))))
                (let ((a (V.insert 5)))
                  (let ((b (V.buy)))
                    (let ((c (V.insert 4)))
                      (let ((d (V.buy)))
                        (let ((e (V.insert 10)))
                          (let ((f (V.buy)))
                            (let ((g (V.buy)))
                              (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 4980902100293 Int64))
  (call   main (: 0 Int64)) (output (: 4980848183738 Int64)))
