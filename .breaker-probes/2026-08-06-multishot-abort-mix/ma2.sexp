(case "ma2 an inner ABORT handle inside a MULTI-SHOT region — each branch runs its own abort"
  (input  (do
            (effect Amb (op pick (-> Unit Int64)))
            (effect Bail (op stop (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Amb 0
                ((pick (u) s (+ (resume 1 s) (resume 2 s))))
                (+ (* 10 (Amb.pick))
                   (handle Bail 0
                     ((stop (v) t (* v 3)))
                     (+ 999 (Bail.stop (Amb.pick)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 78 Int64)))
