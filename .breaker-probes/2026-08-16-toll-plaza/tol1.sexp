(case "tol1 a TOLL PLAZA with an exact-change lane — a five rides the exact lane counting itself and feeding the till, an overpayment needs the till to COVER the change (the till keeps the toll, the change flows back) else the car is DELAYED with the till untouched, an underpayment bounces with a four-hundred tag, the read packs till exact and delayed, and the seed's float lets one plaza serve every car while the other delays all three overpayers"
  (input  (do
            (effect P
              (op pay (-> Int64 Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle P (tuple (* (% n 3) 3) (: 0 Int64) (: 0 Int64))
                ((pay (amt) st
                  (match st
                    ((tuple till ex dl)
                      (if (= amt 5)
                          (resume (+ (: 100 Int64) (+ ex 1)) (tuple (+ till 5) (+ ex 1) dl))
                          (if (> amt 5)
                              (if (>= till (- amt 5))
                                  (resume (+ (* (- amt 5) 10) 1) (tuple (+ till 5) ex dl))
                                  (resume (+ (: 900 Int64) (+ dl 1)) (tuple till ex (+ dl 1))))
                              (resume (+ (: 400 Int64) amt) st))))))
                 (read () st
                  (match st
                    ((tuple till ex dl)
                      (resume (+ (* till 100) (+ (* ex 10) dl)) st)))))
                (let ((a (P.pay (: 7 Int64))))
                  (let ((b (P.pay (: 5 Int64))))
                    (let ((c (P.pay (: 14 Int64))))
                      (let ((d (P.pay (: 11 Int64))))
                        (let ((f (P.read)))
                          (+ (* 10000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 211010910612310 Int64))
  (call   main (: 0 Int64)) (output (: 9011019029030513 Int64)))
