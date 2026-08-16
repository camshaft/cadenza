(case "tri1 sign TRICHOTOMY of a draw — negative, zero-literal, and positive rows each route distinctly"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (+ (* 10 (match (E.next)
                           ((guard d (< d 0)) (- 100 d))
                           (0 555)
                           (d (+ 200 d))))
                   (- (E.probe) n))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 2041 Int64))
  (call   main (: 0 Int64)) (output (: 5551 Int64))
  (call   main (: -6 Int64)) (output (: 1061 Int64)))
