(case "oamin2 helper-abort under Bail nested inside an OUTER resumptive E frame"
  (input  (do
            (effect E (op probe (-> Int64)))
            (effect Bail (op out (-> Int64 Int64)))
            (def (unwrap (: o (Option Int64)) (: tag Int64))
              (match o
                ((Some v) v)
                ((None) (Bail.out tag))))
            (def (main (: n Int64))
              (handle E n
                ((probe () s (resume s s)))
                (+ (* 10 (handle Bail 0
                           ((out (v) t (+ 500 v)))
                           (unwrap (if (> n 0) (Some n) (None)) 11)))
                   (E.probe))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 44 Int64))
  (call   main (: -1 Int64)) (output (: 5109 Int64)))
