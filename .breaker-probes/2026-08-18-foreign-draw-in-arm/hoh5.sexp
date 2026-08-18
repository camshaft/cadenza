(case "hoh5 REPEATED FOREIGN DRAWS THREAD THE OUTER LADDER THROUGH BOTH INNER DISPATCHES — each inner step's answer folds in an outer draw so the outer state climbs by sevens across the inner dispatches while the inner state doubles, both ladders' second rungs land in the thousandfold addend, and either thread resetting between dispatches collapses a distinct digit"
  (input  (do
            (effect B (op step (-> Int64)))
            (effect F (op draw (-> Int64)))
            (def (main (: n Int64))
              (handle F (% n 3)
                ((draw () st (resume st (+ st 7))))
                (handle B (: 1 Int64)
                  ((step () s (resume (+ (* s 10) (F.draw)) (* s 2))))
                  (+ (B.step) (* 1000 (B.step))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 28011 Int64))
  (call   main (: 0 Int64)) (output (: 27010 Int64)))
