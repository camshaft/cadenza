(case "im2 the abort SELECTS which of two pre-advanced states escapes (abort-as-selector over inner handles)"
  (input  (do
            (effect A (op adv (-> Unit Int64)) (op halt (-> Unit Int64)))
            (def (round (: seed Int64) (: k Int64))
              (handle A seed
                ((adv (u) s (resume 0 (+ s k)))
                 (halt (u) s s))
                (do (A.adv) (A.adv) (A.halt))))
            (def (main (: n Int64))
              (+ (* 10 (round n 1)) (round n 100)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 275 Int64)))
