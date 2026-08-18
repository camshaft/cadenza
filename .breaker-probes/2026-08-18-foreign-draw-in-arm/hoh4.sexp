(case "hoh4 a FOREIGN DRAW INSIDE THE INNER ARM'S ANSWER — the inner handler's arm draws the OUTER effect while building its resume answer so the outer state thread advances once for the body's opening draw and once inside the inner dispatch, the arm-side draw sees the state the body-side draw left behind, and the seed shifts only the inner tenfold addend"
  (input  (do
            (effect B (op step (-> Int64)))
            (effect F (op draw (-> Int64)))
            (def (main (: n Int64))
              (handle F (: 1 Int64)
                ((draw () st (resume st (+ st 7))))
                (+ (F.draw)
                   (* 100 (handle B (% n 3)
                            ((step () s (resume (+ (* s 10) (F.draw)) (+ s 1))))
                            (B.step))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1801 Int64))
  (call   main (: 0 Int64)) (output (: 801 Int64)))
