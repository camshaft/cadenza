(case "xhsE the COMPUTED perform-arg variant — the inner arm let-binds the advanced column but performs the outer note with the binder PLUS ONE (a compound of the binder, not the binder itself), resuming and threading as before; the freeze's completeness boundary declines this today and the post-merge follow-up folds it"
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
                      (let ((nv (O.note (+ c2 1))))
                        (resume (+ (* c2 10) (% nv 10)) c2)))))
                  (let ((a (I.step (: 3 Int64))))
                    (let ((b (I.step (: 5 Int64))))
                      (let ((c (O.note (: 100 Int64))))
                        (+ (* 1000 (+ (* 1000 a) b)) c)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 45106116 Int64))
  (call   main (: 0 Int64)) (output (: 34083113 Int64)))
