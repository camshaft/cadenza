(case "or2 the handler ARM builds a record and the body projects it open-row at two widths — arm-built rows cross the dispatch boundary"
  (input  (do
            (effect Mk (op pack (-> Int64 (Record (: x Int64) (: t Int64)))))
            (def (get-x r) (. r x))
            (def (main (: n Int64))
              (handle Mk n
                ((pack (a) s (resume (record (= x (* 10 a)) (= t s)) (+ s 1))))
                (let ((r1 (Mk.pack 2))
                      (r2 (Mk.pack 3)))
                  (+ (get-x r1)
                     (+ (* 100 (get-x r2))
                        (+ (* 10000 (. r1 t)) (* 1000000 (. r2 t))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6053020 Int64))
  (call   main (: 0 Int64)) (output (: 1003020 Int64)))
