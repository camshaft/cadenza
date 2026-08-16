(case "wx4 the ARM doubles its argument with wrapping-mul — MAX wraps to -2, MIN to 0, a small value stays exact, count rides along"
  (input  (do
            (effect E (op dbl (-> Int64 Int64)) (op count (-> Int64)))
            (def (main (: u Int64))
              (handle E 0
                ((dbl (x) s (resume (Int64.wrapping-mul x 2) (+ s 1)))
                 (count () s (resume s s)))
                (+ (E.dbl 9223372036854775807)
                   (+ (E.dbl -9223372036854775808)
                      (+ (E.dbl 3) (* 10 (E.count)))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 34 Int64)))
