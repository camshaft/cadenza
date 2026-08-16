(case "ra8 the recursive walk performs two draws ONLY at the exit leaf (one per non-exit round)"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (walk (: k Int64))
              (let ((a (E.next)))
                (if (< a 20)
                    (walk (+ k 1))
                    (let ((b (E.next)))
                      (+ (* 100 k) b)))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 5))))
                (+ (walk 0) (E.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 355 Int64))
  (call   main (: 1 Int64)) (output (: 457 Int64)))
