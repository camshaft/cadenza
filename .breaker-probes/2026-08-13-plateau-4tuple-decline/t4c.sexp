(case "t4cmin same arm, TWO dispatches"
  (input  (do
            (effect S (op t (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (tuple -999 0 0 -1)
                ((t (v) st
                  (match st
                    ((tuple prev run bl bv)
                      (let ((r2 (if (= v prev) (+ run 1) 1)))
                        (let ((bl2 (if (> r2 bl) r2 bl)))
                          (let ((bv2 (if (> r2 bl) v bv)))
                            (resume bl2 (tuple v r2 bl2 bv2)))))))))
                (+ (S.t n) (* 10 (S.t n)))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 21 Int64)))
