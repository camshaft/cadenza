(case "fac2 the BODY manufactures snapshot closures from op-answered values — two factories at different drawn states, both applied after the state moved on"
  (input  (do
            (effect S
              (op snap (-> Int64))
              (op bump (-> Int64)))
            (def (main (: n Int64))
              (handle S n
                ((snap () s (resume s s))
                 (bump () s (resume (+ s 1) (+ s 1))))
                (let ((c1 (S.snap)))
                  (let ((f1 (fn ((: x Int64)) (+ (* x 10) c1))))
                    (let ((_a (S.bump)))
                      (let ((_b (S.bump)))
                        (let ((c2 (S.snap)))
                          (let ((f2 (fn ((: x Int64)) (+ (* x 10) c2))))
                            (+ (* 1000 (f1 5)) (f2 5))))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 53055 Int64))
  (call   main (: 40 Int64)) (output (: 90092 Int64)))
