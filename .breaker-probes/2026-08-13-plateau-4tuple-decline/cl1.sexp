(case "cl1 simple 4-tuple rotate arm, five dispatches"
  (input  (do
            (effect S (op t (-> Int64)))
            (def (main (: n Int64))
              (handle S (tuple n 1 2 3)
                ((t () st (match st ((tuple a b c d) (resume a (tuple b c d a))))))
                (let ((a (S.t)))
                  (let ((b (S.t)))
                    (let ((c (S.t)))
                      (let ((d (S.t)))
                        (let ((e (S.t)))
                          (+ a (+ b (+ c (+ d e)))))))))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 14 Int64)))
