(case "dv1 truncated division and dividend-sign modulo over DRAWS — negative dividends exercise the toward-zero rule through dispatch"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 3))))
                (let ((a (E.next)))
                  (let ((b (E.next)))
                    (+ (* 1000 (/ a 4))
                       (+ (* 100 (% a 4))
                          (+ (* 10 (/ b 4)) (% b 4))))))))
            (export main)))
  (call   main (: -7 Int64)) (output (: -1310 Int64))
  (call   main (: 5 Int64)) (output (: 1120 Int64))
  (call   main (: -9 Int64)) (output (: -2112 Int64)))
