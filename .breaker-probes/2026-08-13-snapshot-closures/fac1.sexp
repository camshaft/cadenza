(case "fac1 the arm MANUFACTURES a closure over the CURRENT state as the op result — two factories at different states hand back distinct snapshots, both applied after the state moved on"
  (input  (do
            (effect S
              (op mk (-> (-> Int64 Int64)))
              (op bump (-> Int64)))
            (def (main (: n Int64))
              (handle S n
                ((mk () s (resume (fn ((: x Int64)) (+ (* x 10) s)) s))
                 (bump () s (resume (+ s 1) (+ s 1))))
                (let ((f1 (S.mk)))
                  (let ((_a (S.bump)))
                    (let ((_b (S.bump)))
                      (let ((f2 (S.mk)))
                        (+ (* 1000 (f1 5)) (f2 5))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 53055 Int64))
  (call   main (: 40 Int64)) (output (: 90092 Int64)))
