(case "dbt1 a DEBT AMORTIZATION schedule — pay accrues truncating interest at the seed-shaped rate first and the remainder reduces principal, answering the interest slice; left reads the balance; the higher rate leaks interest on every payment so the principals drift apart payment by payment"
  (input  (do
            (effect D
              (op pay (-> Int64 Int64))
              (op left (-> Int64)))
            (def (main (: n Int64))
              (handle D (: 100 Int64)
                ((pay (v) p
                  (resume (/ (* p (+ (% n 4) 1)) 100)
                          (- p (- v (/ (* p (+ (% n 4) 1)) 100)))))
                 (left () p (resume p p)))
                (let ((a (D.pay 20)))
                  (let ((b (D.pay 20)))
                    (let ((c (D.left)))
                      (let ((d (D.pay 30)))
                        (let ((e (D.left)))
                          (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 3002065001036 Int64))
  (call   main (: 0 Int64)) (output (: 1000061000031 Int64)))
