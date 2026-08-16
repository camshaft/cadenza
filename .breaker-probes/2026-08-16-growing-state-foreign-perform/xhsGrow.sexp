(case "xhsGrow GROWING-STATE shared-let with a mid-arm foreign perform — the step arm derives its binder from the list LENGTH, performs the outer note with it, answers the binder packed with the note's low digit, and threads the list GROWN by the binder; the growing next-state excludes the collapse so the distribute path must keep the two binder copies coherent"
  (input  (do
            (effect O (op note (-> Int64 Int64)))
            (effect I (op step (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle O (: 0 Int64)
                ((note (v) acc
                  (resume (+ acc v) (+ acc v))))
                (handle I (list)
                  ((step (x) col
                    (let ((c2 (+ (List.len col) (+ x (% n 3)))))
                      (let ((nv (O.note c2)))
                        (resume (+ (* c2 10) (% nv 10)) (List.push col c2))))))
                  (let ((a (I.step (: 3 Int64))))
                    (let ((b (I.step (: 5 Int64))))
                      (let ((r (O.note (: 100 Int64))))
                        (+ (* 1000 (+ (* 1000 a) b)) r)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 44071111 Int64))
  (call   main (: 0 Int64)) (output (: 33069109 Int64)))
