(case "irg3 the IRRIGATION controller at three requests — zone-scaled costs, counted skips with the rotation still advancing, the seed-set budget decides whether the third request waters or skips"
  (input  (do
            (effect I
              (op water (-> Int64 Int64))
              (op report (-> Int64)))
            (def (main (: n Int64))
              (handle I (tuple (: 0 Int64) (+ (: 10 Int64) (* (% n 3) 4)) (: 0 Int64))
                ((water (amt) st
                  (match st
                    ((tuple zone budget skipped)
                      (if (>= budget (* amt (+ zone 1)))
                          (resume (+ (* (* amt (+ zone 1)) 10) (% (+ zone 1) 3))
                                  (tuple (% (+ zone 1) 3) (- budget (* amt (+ zone 1))) skipped))
                          (resume (+ (: 900 Int64) (% (+ zone 1) 3))
                                  (tuple (% (+ zone 1) 3) budget (+ skipped 1)))))))
                 (report () st
                  (match st ((tuple zone budget skipped) (resume (+ (* budget 100) (+ (* skipped 10) zone)) st)))))
                (let ((a (I.water (: 3 Int64))))
                  (let ((b (I.water (: 2 Int64))))
                    (let ((c (I.water (: 2 Int64))))
                      (let ((f (I.report)))
                        (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) f)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 31042060100 Int64))
  (call   main (: 0 Int64)) (output (: 31042900310 Int64)))
