(case "gd3 the guard COMPARES the scrutinee draw to an earlier let-bound draw — two thread values meet in one pure predicate"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (if (= (% s 2) 0) (+ s 1) (- s 2)))))
                (let ((a (E.next)))
                  (match (E.next)
                    ((guard b (> b a)) (+ (* 10 (+ 100 b)) (- b a)))
                    (b (+ (* 10 (+ 300 b)) (- a b)))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1011 Int64))
  (call   main (: 1 Int64)) (output (: 2992 Int64))
  (call   main (: 3 Int64)) (output (: 3012 Int64)))
