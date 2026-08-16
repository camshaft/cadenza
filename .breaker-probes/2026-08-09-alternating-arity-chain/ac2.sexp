(case "ac2 the chain's MIDDLE result routes the branch that picks WHICH op finishes the pipeline"
  (input  (do
            (effect E (op inc (-> Int64 Int64)) (op dbl (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((inc (x) s (resume (+ x s) (+ s 1)))
                 (dbl (x) s (resume (+ (* 2 x) s) (+ s 2))))
                (let ((mid (E.dbl (E.inc 3))))
                  (if (> mid 10) (E.inc mid) (E.dbl mid)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 30 Int64))
  (call   main (: 0 Int64)) (output (: 17 Int64))
  (call   main (: -9 Int64)) (output (: -46 Int64)))
