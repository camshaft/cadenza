(case "rw1 two sequential Record.with updates each take a DRAW — the second write sees the advanced state, projections read both back"
  (input  (do
            (effect E (op next (-> Int64)) (op span (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 3)))
                 (span () s (resume (- s 0) s)))
                (let ((r0 (record (x 1) (y 2))))
                  (let ((r1 (Record.with r0 #"x" (E.next))))
                    (let ((r2 (Record.with r1 #"y" (E.next))))
                      (+ (* 100 (. r2 x)) (+ (* 10 (. r2 y)) (- (E.span) n))))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 256 Int64))
  (call   main (: 0 Int64)) (output (: 36 Int64))
  (call   main (: -4 Int64)) (output (: -404 Int64)))
