(case "xhsC the outer perform takes a CONSTANT while the binder feeds answer and state — boundary variant"
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
                      (let ((nv (O.note (: 5 Int64))))
                        (resume (+ (* c2 10) (% nv 10)) c2)))))
                  (let ((a (I.step (: 3 Int64))))
                    (let ((b (I.step (: 5 Int64))))
                      (let ((c (O.note (: 100 Int64))))
                        (+ (* 1000 (+ (* 1000 a) b)) c)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 45100110 Int64))
  (call   main (: 0 Int64)) (output (: 35080110 Int64)))
