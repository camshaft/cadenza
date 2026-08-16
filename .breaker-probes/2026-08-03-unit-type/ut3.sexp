(case "ut3 TWO-param generic sum with a unit variant between them keeps exactly [a,b]"
  (input  (do
        (type (Pair a b) (Both a b) (Neither unit))
        (def (main (: k Int64))
          (+ (* 10 (match (Both k "x") ((Both n _s) n) ((Neither _u) -1)))
             (match ((. Set len) (Set.of (list (Neither unit) (Neither unit)))) (1 1) (_other 0))))
        (export main)))
  (call   main (: 4 Int64)) (output (: 41 Int64)))
