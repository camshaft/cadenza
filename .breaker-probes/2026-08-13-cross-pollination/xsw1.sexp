(case "xsw1 a CROSS-SWAPPING scalar pair — each dispatch computes both successors then installs them SWAPPED (a gets the b-derived value, b the a-derived), lineage crosses sides every step"
  (input  (do
            (effect S (op step (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (tuple n 2)
                ((step (v) st
                  (match st
                    ((tuple a b)
                      (let ((na (+ a v)))
                        (let ((nb (* b v)))
                          (resume (+ (* 100 na) nb) (tuple nb na))))))))
                (let ((r1 (S.step 2)))
                  (let ((r2 (S.step 3)))
                    (+ (* 100000 r1) r2)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 50400715 Int64))
  (call   main (: 0 Int64)) (output (: 20400706 Int64)))
