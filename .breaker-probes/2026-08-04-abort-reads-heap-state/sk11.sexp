(case "sk11 the abort arm reads heap state THROUGH A HELPER FN call (threading across a call boundary)"
  (input  (do
            (effect St (op put (-> Int64 Int64)) (op halt (-> Unit Int64)))
            (def (score (: s (List Int64))) (* 1000 (List.len s)))
            (def (main (: a Int64))
              (handle St (list)
                ((put (v) s (resume 0 (List.push s v)))
                 (halt (u) s (score s)))
                (do
                  (def x (St.put a))
                  (def y (St.put (+ a 1)))
                  (St.halt))))
            (export main)))
  (call   main (: 7 Int64))
  (output (: 2000 Int64)))
