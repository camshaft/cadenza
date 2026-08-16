(case "do1 three DISCARDED performs on a do-spine still advance the state (effect-only evaluation)"
  (input  (do
            (effect St (op bump (-> Unit Int64)) (op peek (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((bump (u) s (resume s (+ s 1)))
                 (peek (u) s (resume s s)))
                (do
                  (St.bump)
                  (St.bump)
                  (St.bump)
                  (St.peek))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 8 Int64)))
