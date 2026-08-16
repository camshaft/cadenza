(case "rw2 draw PARITY picks WHICH field Record.with updates — projections of both fields show exactly one write landed"
  (input  (do
            (effect E (op next (-> Int64)) (op span (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 3)))
                 (span () s (resume s s)))
                (let ((d (E.next)))
                  (let ((r0 (record (x 1) (y 2))))
                    (let ((r (if (= (% d 2) 0)
                                 (Record.with r0 #"x" d)
                                 (Record.with r0 #"y" d))))
                      (+ (* 100 (. r x)) (+ (* 10 (. r y)) (- (E.span) n))))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 223 Int64))
  (call   main (: 5 Int64)) (output (: 153 Int64))
  (call   main (: -4 Int64)) (output (: -377 Int64)))
