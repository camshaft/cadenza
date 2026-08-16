(case "ha1 a FULL handle expression in the resume-value slot — the arm runs a nested handler per dispatch"
  (input  (do
            (effect Out (op big (-> Int64 Int64)))
            (effect In (op small (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Out 0
                ((big (v) s
                  (resume (handle In 100
                            ((small (w) t (resume (+ w t) t)))
                            (In.small (* v 2)))
                          s)))
                (Out.big n)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 110 Int64)))
