(case "nb1 draws ASSEMBLE a number digit-by-digit — the inverse of the peel arm, low digits of a striding thread"
  (input  (do
            (effect E (op digit (-> Int64)))
            (def (main (: n Int64))
              (handle E (if (< n 0) (- 0 n) n)
                ((digit () s (resume (% s 10) (+ s 3))))
                (let ((d1 (E.digit)))
                  (let ((d2 (E.digit)))
                    (let ((d3 (E.digit)))
                      (+ (* 100 d1) (+ (* 10 d2) d3)))))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 470 Int64))
  (call   main (: 0 Int64)) (output (: 36 Int64))
  (call   main (: -17 Int64)) (output (: 703 Int64)))
