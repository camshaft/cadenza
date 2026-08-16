(case "sk12 abort value CONTAINS the heap state itself (state ESCAPES via the abort return, a List result)"
  (input  (do
            (effect St (op put (-> Int64 Int64)) (op halt (-> Unit (List Int64))))
            (def (main (: a Int64))
              (do
                (def r (handle St (list)
                  ((put (v) s (resume 0 (List.push s v)))
                   (halt (u) s s))
                  (do
                    (def x (St.put a))
                    (def y (St.put (+ a 1)))
                    (St.halt))))
                (+ (* 10 (List.len r))
                   (match (List.at r 0) ((Option.Some v) v) ((Option.None) -1)))))
            (export main)))
  (call   main (: 7 Int64))
  (output (: 27 Int64)))
