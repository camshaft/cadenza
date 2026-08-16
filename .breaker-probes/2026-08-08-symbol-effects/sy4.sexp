(case "sy4 sequential draws written into record FIELDS via Record.with symbol selectors — chained functional updates"
  (input  (do
            (effect P (op pick (-> Int64)))
            (def (main (: n Int64))
              (handle P n
                ((pick () s (resume s (+ s 1))))
                (let ((r (record (x 10) (y 20))))
                  (let ((r2 (Record.with r #"x" (P.pick))))
                    (let ((r3 (Record.with r2 #"y" (P.pick))))
                      (+ (* 100 (. r3 x)) (. r3 y)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 506 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64)))
