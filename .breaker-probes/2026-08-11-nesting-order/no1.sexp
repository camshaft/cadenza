(case "no1 the inner arm performs the OUTER effect mid-transition — both threads advance in lockstep per inner dispatch"
  (input  (do
            (effect A (op ga (-> Int64)))
            (effect B (op gb (-> Int64)))
            (def (main (: n Int64))
              (handle A n
                ((ga () s (resume s (+ s 1))))
                (handle B 100
                  ((gb () t (resume (+ t (A.ga)) (+ t 10))))
                  (+ (B.gb) (* 1000 (B.gb))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 114103 Int64))
  (call   main (: 0 Int64)) (output (: 111100 Int64)))
