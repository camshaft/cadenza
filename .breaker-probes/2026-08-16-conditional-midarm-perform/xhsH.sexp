(case "xhsH the foreign perform AS the selector — the note's answer is let-bound then ROUTES the branch (over nine takes a 7 tag, under packs the answer's low digit), both branches silent, the perform running exactly once as condition input; the inverse of the G family where the perform hides inside a branch"
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
                        (if (> nv 9)
                            (resume (+ (* c2 10) 7) c2)
                            (resume (+ (* c2 10) (% nv 10)) c2))))))
                  (let ((p (I.step (: 3 Int64))))
                    (let ((q (I.step (: 5 Int64))))
                      (let ((r (O.note (: 100 Int64))))
                        (+ (* 1000 (+ (* 1000 p) q)) r)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 44107114 Int64))
  (call   main (: 0 Int64)) (output (: 33087111 Int64)))
