(case "oos1 a NESTED-Option STATE ladder — None to Some None to Some Some v and back, advance climbs one rung per dispatch and classify decodes all three inhabitants between steps"
  (input  (do
            (effect S
              (op adv (-> Int64 Int64))
              (op cls (-> Int64)))
            (def (main (: n Int64))
              (handle S (: (None unit) (Option (Option Int64)))
                ((adv (v) st
                  (match st
                    ((Some inner)
                      (match inner
                        ((Some _x) (resume 0 (: (None unit) (Option (Option Int64)))))
                        ((None _u) (resume 2 (Some (Some v))))))
                    ((None _u) (resume 1 (Some (: (None unit) (Option Int64)))))))
                 (cls () st
                  (resume (match st
                            ((Some inner) (match inner ((Some x) (+ 30 x)) ((None _u) 1)))
                            ((None _u) 0))
                          st)))
                (let ((a (S.cls)))
                  (let ((b (S.adv n)))
                    (let ((c (S.cls)))
                      (let ((d (S.adv n)))
                        (let ((e (S.cls)))
                          (let ((f (S.adv 9)))
                            (let ((g (S.cls)))
                              (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g))))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 10102350000 Int64))
  (call   main (: 7 Int64)) (output (: 10102370000 Int64)))
