(case "si2 the SEED handle ABORTS — the outer state is the abort value, the seed body's tail never runs"
  (input  (do
            (effect A (op tick (-> Unit Int64)))
            (effect B (op get (-> Unit Int64)))
            (def (main (: n Int64))
              (handle B
                (handle A n ((tick (u) s (+ s 100))) (do (A.tick) 999))
                ((get (u) t (resume t (+ t 1))))
                (+ (B.get) (* 100 (B.get)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 10705 Int64))
  (call   main (: 0 Int64)) (output (: 10200 Int64))
  (call   main (: -4 Int64)) (output (: 9796 Int64)))
