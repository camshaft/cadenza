(case "ae1 ao10 escalation: performing-cond where the condition performs the SAME effect that aborts"
  (input  (do
            (effect A (op tick (-> Unit Int64)))
            (effect B (op bail (-> Unit Int64)))
            (def (main (: n Int64))
              (handle A n
                ((tick (u) s (resume s (+ s 1))))
                (+ (handle B 0
                     ((bail (u) t 99))
                     (if (> (A.tick) 0) (B.bail) -1))
                   (A.tick))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 105 Int64)))
