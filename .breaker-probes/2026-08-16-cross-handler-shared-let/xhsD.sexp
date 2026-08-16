(case "xhsD the DROPPED-frozen-arg complement — the outer note arm IGNORES its op param (answers the count, threads count plus one) while the inner step arm still let-binds the advanced column performs the outer note with it and resumes packing both; the frozen argument is dropped not escaped, and the fold is CORRECT"
  (input  (do
            (effect O (op note (-> Int64 Int64)))
            (effect I (op step (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle O (: 0 Int64)
                ((note (v) acc
                  (resume acc (+ acc 1))))
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
  (call   main (: 10 Int64)) (output (: 40101002 Int64))
  (call   main (: 0 Int64)) (output (: 30081002 Int64)))
