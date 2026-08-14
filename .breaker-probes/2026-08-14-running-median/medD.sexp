(case "medD control — a SCALAR-arg def with an internal let called from the arm"
  (input  (do
            (effect M (op add (-> Int64 Int64)))
            (def (halfish (: v Int64))
              (let ((h (/ v 2)))
                (if (= (% v 2) 1) (+ h 1) h)))
            (def (main (: n Int64))
              (handle M (: 0 Int64)
                ((add (v) st
                  (resume (halfish (+ st v)) (+ st v))))
                (let ((a (M.add (+ n 4))))
                  (let ((b (M.add 2)))
                    (let ((c (M.add 9)))
                      (+ (* 100 (+ (* 100 a) b)) c))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 70813 Int64))
  (call   main (: 0 Int64)) (output (: 20308 Int64)))
