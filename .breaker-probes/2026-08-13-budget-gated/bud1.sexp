(case "bud1 a BUDGET-GATED accumulator — two spends consume the budget, the third answers the negated total from exhaustion, a refill re-arms exactly one more spend, the final read exposes the accumulated total"
  (input  (do
            (effect S
              (op spend (-> Int64 Int64))
              (op refill (-> Int64 Int64))
              (op total (-> Int64)))
            (def (main (: n Int64))
              (handle S (tuple 2 n)
                ((spend (v) st
                  (match st
                    ((tuple b t)
                      (if (> b 0)
                          (resume (- b 1) (tuple (- b 1) (+ t v)))
                          (resume (- 0 t) st)))))
                 (refill (k) st
                  (match st
                    ((tuple b t) (resume (+ b k) (tuple (+ b k) t)))))
                 (total () st
                  (match st ((tuple _b t) (resume t st)))))
                (let ((a (S.spend 5)))
                  (let ((b (S.spend 7)))
                    (let ((c (S.spend 9)))
                      (let ((d (S.refill 1)))
                        (let ((e (S.spend 9)))
                          (let ((f (S.total)))
                            (+ (* 100 (+ (* 10 (+ (* 10 (+ (* 100 (+ (* 10 a) b)) (+ c 50))) d)) e)) f)))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 10351024 Int64))
  (call   main (: 30 Int64)) (output (: 10081051 Int64)))
