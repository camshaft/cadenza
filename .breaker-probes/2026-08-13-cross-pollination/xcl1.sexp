(case "xcl1 a CROSS-POLLINATING closure pair — the tuple state holds two functions, each dispatch rebuilds each one CAPTURING THE OTHER'S fresh result, so lineage crosses sides every step"
  (input  (do
            (effect S (op step (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (tuple (fn ((: x Int64)) (+ x n)) (fn ((: x Int64)) (* x 2)))
                ((step (v) st
                  (match st
                    ((tuple f g)
                      (let ((a (f v)))
                        (let ((b (g v)))
                          (resume (+ (* 100 a) b)
                                  (tuple (fn ((: x Int64)) (+ x b))
                                         (fn ((: x Int64)) (* x a))))))))))
                (let ((r1 (S.step 2)))
                  (let ((r2 (S.step 3)))
                    (+ (* 100000 r1) r2)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 50400715 Int64))
  (call   main (: 0 Int64)) (output (: 20400706 Int64)))
