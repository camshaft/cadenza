(case "xhs1 CROSS-HANDLER shared-let — the inner step arm let-binds the advanced column, PERFORMS the outer note with it mid-arm (accumulating), then resumes packing the binder with the note's answer while threading the binder as next-state; the seed bias shifts every column so both the inner rows and the outer accumulator diverge"
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
                        (resume (+ (* c2 10) (% nv 10)) c2)))))
                  (let ((a (I.step (: 3 Int64))))
                    (let ((b (I.step (: 5 Int64))))
                      (let ((c (O.note (: 100 Int64))))
                        (+ (* 1000 (+ (* 1000 a) b)) c)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 44104114 Int64))
  (call   main (: 0 Int64)) (output (: 33081111 Int64)))
