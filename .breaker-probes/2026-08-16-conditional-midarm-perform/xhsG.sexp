(case "xhsG CONDITIONAL mid-arm foreign perform — the note rides only the THEN branch"
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
                      (if (> c2 5)
                          (let ((nv (O.note c2)))
                            (resume (+ (* c2 10) (% nv 10)) c2))
                          (resume (* c2 10) c2)))))
                  (let ((p (I.step (: 3 Int64))))
                    (let ((q (I.step (: 5 Int64))))
                      (let ((r (O.note (: 100 Int64))))
                        (+ (* 1000 (+ (* 1000 p) q)) r)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 40100110 Int64))
  (call   main (: 0 Int64)) (output (: 30088108 Int64)))
