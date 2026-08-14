(case "cl2 complex 3-let 2-if arm on a THREE-tuple, five dispatches"
  (input  (do
            (effect S (op t (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (tuple -999 0 0)
                ((t (v) st
                  (match st
                    ((tuple prev run bl)
                      (let ((r2 (if (= v prev) (+ run 1) 1)))
                        (let ((bl2 (if (> r2 bl) r2 bl)))
                          (let ((x (+ r2 bl2)))
                            (resume (+ (* bl2 10) (% x 10)) (tuple v r2 bl2)))))))))
                (let ((a (S.t 4)))
                  (let ((b (S.t 4)))
                    (let ((c (S.t n)))
                      (let ((d (S.t n)))
                        (let ((e (S.t n)))
                          (+ a (+ b (+ c (+ d e)))))))))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 170 Int64)))
