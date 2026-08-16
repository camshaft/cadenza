(case "ha2 the arm's nested handler is instantiated FRESH per dispatch — two dispatches, independent inner state"
  (input  (do
            (effect Out (op big (-> Int64 Int64)))
            (effect In (op small (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Out 0
                ((big (v) s
                  (resume (handle In v
                            ((small (u) t (resume t (+ t 1))))
                            (+ (In.small) (In.small)))
                          s)))
                (+ (* 100 (Out.big n)) (Out.big 20))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1141 Int64)))
