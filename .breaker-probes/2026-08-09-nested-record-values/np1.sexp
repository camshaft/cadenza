(case "np1 a NESTED record value built from one draw — projections through two levels read the same drawn base"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (let ((d (E.next)))
                  (let ((r (record (a d) (b (record (x (* 2 d)) (y (+ d 5)))))))
                    (+ (* 100 (. r a))
                       (+ (* 10 (. (. r b) x))
                          (+ (. (. r b) y)
                             (* 1000 (- (E.probe) n)))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 1368 Int64))
  (call   main (: 0 Int64)) (output (: 1005 Int64))
  (call   main (: -4 Int64)) (output (: 521 Int64)))
