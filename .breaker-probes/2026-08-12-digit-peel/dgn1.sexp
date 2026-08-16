(case "dgn1 digit-peel of a NEGATIVE state — truncated division and dividend-sign remainder agree through the thread, three negative digits"
  (input  (do
            (effect S (op digit (-> Int64)))
            (def (main (: n Int64))
              (handle S n
                ((digit () s (resume (% s 10) (/ s 10))))
                (let ((d1 (S.digit)))
                  (let ((d2 (S.digit)))
                    (let ((d3 (S.digit)))
                      (+ (* 100 d1) (+ (* 10 d2) d3)))))))
            (export main)))
  (call   main (: -251 Int64)) (output (: -152 Int64))
  (call   main (: -8 Int64)) (output (: -800 Int64)))
