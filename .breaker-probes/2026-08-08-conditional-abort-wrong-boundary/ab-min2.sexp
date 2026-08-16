(case "abmin2 outer-abort under a LET+IF inside the unrelated inner handle"
  (input  (do
            (effect E (op next (-> Int64)))
            (effect A (op out (-> Int64 Int64)))
            (effect B (op bout (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (handle A 0
                  ((out (v) t (+ 9000 v)))
                  (+ (* 100 (handle B 0
                              ((bout (v) t (+ 500 v)))
                              (let ((d (E.next)))
                                (if (= (% d 3) 0) (A.out d) d))))
                     (- (E.next) n)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 9003 Int64)))
