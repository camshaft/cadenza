(case "xfr1 CLAMPED transfers between two accounts — each transfer moves min(requested, available) and answers the amount actually moved, the final draw reads the signed imbalance, and only one seed clamps the last transfer"
  (input  (do
            (effect X
              (op xfer (-> Int64 Int64))
              (op back (-> Int64 Int64))
              (op imb (-> Int64)))
            (def (main (: n Int64))
              (handle X (tuple (+ n 5) (: 3 Int64))
                ((xfer (v) st
                  (match st
                    ((tuple a b)
                      (if (< a v)
                          (resume a (tuple 0 (+ b a)))
                          (resume v (tuple (- a v) (+ b v)))))))
                 (back (v) st
                  (match st
                    ((tuple a b)
                      (if (< b v)
                          (resume b (tuple (+ a b) 0))
                          (resume v (tuple (+ a v) (- b v)))))))
                 (imb () st
                  (match st ((tuple a b) (resume (- a b) st)))))
                (let ((p (X.xfer 4)))
                  (let ((q (X.back 9)))
                    (let ((r (X.xfer 6)))
                      (let ((s (X.back 2)))
                        (let ((t (X.xfer 11)))
                          (let ((u (X.imb)))
                            (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 p) q)) r)) s)) t)) u)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 40706021088 Int64))
  (call   main (: 0 Int64)) (output (: 40706020392 Int64)))
