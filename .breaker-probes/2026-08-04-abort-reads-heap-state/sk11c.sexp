(case "sk11c sk11 boundary: heap state through a helper in a RESUME arm"
  (input  (do
            (effect St (op put (-> Int64 Int64)))
            (def (score (: s (List Int64))) (List.len s))
            (def (main (: a Int64))
              (handle St (list)
                ((put (v) s (resume (score s) (List.push s v))))
                (do
                  (def x (St.put a))
                  (St.put (+ a 1)))))
            (export main)))
  (call   main (: 7 Int64))
  (output (: 1 Int64)))
