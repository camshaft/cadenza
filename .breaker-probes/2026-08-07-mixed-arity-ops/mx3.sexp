(case "mx3 a TUPLE-arg op chained through a tuple RESULT — the second dispatch consumes the first's destructured output"
  (input  (do
            (effect E (op quo (-> (Tuple Int64 Int64) (Tuple Int64 Int64))))
            (def (main (: n Int64))
              (handle E n
                ((quo (p) s (match p ((tuple q r) (resume (tuple (+ q s) (* r 2)) (+ s 10))))))
                (match (E.quo (tuple 3 4))
                  ((tuple x y) (match (E.quo (tuple x y))
                                 ((tuple u v) (+ (* 100 u) v)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2316 Int64))
  (call   main (: 0 Int64)) (output (: 1316 Int64)))
