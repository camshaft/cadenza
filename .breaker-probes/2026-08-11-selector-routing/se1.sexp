(case "se1 a draw SELECTS which op the body performs next — control flow routed by effect results, parity alternates the route"
  (input  (do
            (effect S (op pick (-> Int64)) (op left (-> Int64 Int64)) (op right (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S n
                ((pick () s (resume (% s 2) (+ s 1)))
                 (left (v) s (resume (+ v 100) s))
                 (right (v) s (resume (+ v 200) s)))
                (let ((sel1 (S.pick)))
                  (let ((r1 (if (= sel1 0) (S.left 1) (S.right 1))))
                    (let ((sel2 (S.pick)))
                      (let ((r2 (if (= sel2 0) (S.left 2) (S.right 2))))
                        (+ (* 1000 r1) r2)))))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 101202 Int64))
  (call   main (: 5 Int64)) (output (: 201102 Int64)))
