(case "er1 the arm uses try over its own MATCH on the op-arg (early-return inside an arm expression)"
  (input  (do
            (effect St (op find (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St n
                ((find (k) s
                  (resume (match (if (> k s) (Option.Some (* k 10)) (Option.None))
                            ((Option.Some v) v)
                            ((Option.None) -1))
                          s)))
                (+ (* 10 (St.find 10)) (St.find 1))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 999 Int64)))
