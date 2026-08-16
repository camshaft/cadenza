(case "irg1 an IRRIGATION controller over three zones sharing a water budget — each water request costs the amount SCALED by the current zone number, an unaffordable request is SKIPPED (counted) yet the rotation still advances, the seed sets only the starting budget, and the two runs share their first skip but diverge on a SECOND skip when the tighter budget runs dry a request earlier"
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
                    (let ((c (I.water (: 4 Int64))))
                      (let ((d (I.water (: 1 Int64))))
                        (let ((e (I.water (: 2 Int64))))
                          (let ((f (I.report)))
                            (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 31042900011042212 Int64))
  (call   main (: 0 Int64)) (output (: 31042900011902222 Int64)))
