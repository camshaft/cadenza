(case "ae2 ao10 escalation: performing-cond selecting BETWEEN two aborts (both branches bail differently)"
  (input  (do
            (effect A (op tick (-> Unit Int64)))
            (effect B (op lo (-> Unit Int64)) (op hi (-> Unit Int64)))
            (def (main (: n Int64))
              (+ (handle B 0
                   ((lo (u) t 100)
                    (hi (u) t 200))
                   (handle A n
                     ((tick (u) s (resume s (+ s 1))))
                     (if (> (A.tick) 3) (B.hi) (B.lo))))
                 1))
            (export main)))
  (call   main (: 5 Int64)) (output (: 201 Int64)))
