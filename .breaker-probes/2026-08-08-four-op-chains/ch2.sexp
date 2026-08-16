(case "ch2 the same FOUR-op chain flattened through LETs — one binding per hop must equal the nested form exactly"
  (input  (do
            (effect E (op a (-> Int64)) (op b (-> Int64 Int64)) (op c (-> Int64 Int64)) (op d (-> Int64 Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((a () s (resume s (+ s 2)))
                 (b (x) s (resume (+ x s) (+ s 3)))
                 (c (x) s (resume (* 2 x) (+ s 5)))
                 (d (x) s (resume (+ x s) (+ s 1)))
                 (probe () s (resume s s)))
                (let ((va (E.a)))
                  (let ((vb (E.b va)))
                    (let ((vc (E.c vb)))
                      (let ((vd (E.d vc)))
                        (+ (* 100 vd) (E.probe))))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 2413 Int64))
  (call   main (: 0 Int64)) (output (: 1411 Int64))
  (call   main (: -4 Int64)) (output (: -593 Int64)))
