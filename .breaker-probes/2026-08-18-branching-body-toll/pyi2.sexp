(case "pyi2 BRANCHES WITH UNEQUAL DISPATCH COUNTS under a tolled arm — the drawn parity routes to a one-draw branch or a TWO-draw branch so the seeds stack two or three tolled frames from the same program, the odd path's extra frame adds both an answer factor and a toll, and a lowering assuming a fixed frame count per body misprices the deeper seed"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s (+ (resume s (+ s 1)) (* 1000 (+ s 1)))))
                (match (% (E.tick) 2)
                  (0 (+ 100 (E.tick)))
                  (_ (+ (* 200 (E.tick)) (E.tick))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 9403 Int64))
  (call   main (: 0 Int64)) (output (: 3101 Int64)))
