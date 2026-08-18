(case "tmb3 DOUBLE REPLAY THEN TOMBSTONE — the arm replays the tail twice discards BOTH outcomes and answers a state-keyed tombstone, neither replay's value survives yet both replays still run the body, and a lowering that returns either replay's outcome instead of the tombstone shifts the whole answer"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s
                  (do (resume s (+ s 1))
                      (resume (+ s 10) (+ s 2))
                      (+ (* s 100) 7))))
                (E.tick)))
            (export main)))
  (call   main (: 10 Int64)) (output (: 107 Int64))
  (call   main (: 0 Int64)) (output (: 7 Int64)))
