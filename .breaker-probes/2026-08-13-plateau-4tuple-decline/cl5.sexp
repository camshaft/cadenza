(case "cl5 two-let arm, THREE dispatches"
  (input  (do
            (effect S (op t (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (tuple -999 0 0)
                ((t (v) st
                  (match st
                    ((tuple prev run bl)
                      (let ((r2 (if (= v prev) (+ run 1) 1)))
                        (let ((x (+ r2 bl)))
                          (resume (+ (* x 10) r2) (tuple v r2 x))))))))
                (let ((a (S.t 4))) (let ((b (S.t 4))) (let ((c (S.t n))) (+ a (+ b c)))))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 176 Int64)))
