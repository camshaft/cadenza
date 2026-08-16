(case "xhsA the binder feeds the outer perform and the answer but the state threads the OLD column — boundary variant"
  (input  (do
            (effect O (op note (-> Int64 Int64)))
            (effect I (op step (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle O (: 0 Int64)
                ((note (v) acc
                  (resume (+ acc v) (+ acc v))))
                (handle I (: 0 Int64)
                  ((step (x) col
                    (let ((c2 (+ col (+ x (% n 3)))))
                      (let ((nv (O.note c2)))
                        (resume (+ (* c2 10) (% nv 10)) col)))))
                  (let ((a (I.step (: 3 Int64))))
                    (let ((b (I.step (: 5 Int64))))
                      (let ((c (O.note (: 100 Int64))))
                        (+ (* 1000 (+ (* 1000 a) b)) c)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 44060110 Int64))
  (call   main (: 0 Int64)) (output (: 33058108 Int64)))
