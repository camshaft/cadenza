(case "ap1 an arm performing the outer effect in BOTH its resume-value and next-state slots — order pinned"
  (input  (do
            (effect Out (op tick (-> Unit Int64)))
            (effect In (op step (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Out n
                ((tick (u) t (resume t (+ t 1))))
                (handle In 0
                  ((step (u) s (resume (Out.tick) (+ s (Out.tick)))))
                  (+ (* 100 (In.step)) (In.step)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 507 Int64)))
