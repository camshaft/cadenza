(case "hoh6 the OUTER THREAD SPANS INIT AND ARM DRAWS — the inner handler's starting state comes from an outer draw and its arm folds a SECOND outer draw into the answer, one continuous outer thread runs through the init-time draw into the arm-time draw seven apart, and a thread reset at the frame-install boundary collapses the gap"
  (input  (do
            (effect B (op step (-> Int64)))
            (effect F (op draw (-> Int64)))
            (def (main (: n Int64))
              (handle F (% n 3)
                ((draw () st (resume st (+ st 7))))
                (handle B (F.draw)
                  ((step () s (resume (+ (* s 10) (F.draw)) (+ s 1))))
                  (B.step))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 18 Int64))
  (call   main (: 0 Int64)) (output (: 7 Int64)))
