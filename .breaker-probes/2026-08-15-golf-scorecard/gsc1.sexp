(case "gsc1 a GOLF scorecard with a birdie-streak multiplier — each hole answers its par delta where consecutive under-par holes MULTIPLY the negative contribution by the deepening streak, an over-par hole resets the streak, card totals, and the seed shifts one hole's strokes so the streak bonus doubles differently"
  (input  (do
            (effect G
              (op hole (-> Int64 Int64 Int64))
              (op card (-> Int64)))
            (def (main (: n Int64))
              (handle G (tuple (: 0 Int64) (: 0 Int64))
                ((hole (strokes par) st
                  (match st
                    ((tuple total streak)
                      (if (< strokes par)
                          (resume (* (- strokes par) (+ streak 1))
                                  (tuple (+ total (* (- strokes par) (+ streak 1))) (+ streak 1)))
                          (resume (- strokes par)
                                  (tuple (+ total (- strokes par)) 0))))))
                 (card () st
                  (match st ((tuple total streak) (resume total st)))))
                (let ((a (G.hole 4 4)))
                  (let ((b (G.hole 3 4)))
                    (let ((c (G.hole (+ (% n 3) 2) 4)))
                      (let ((d (G.hole 5 4)))
                        (let ((e (G.hole 3 4)))
                          (let ((f (G.card)))
                            (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: -101990103 Int64))
  (call   main (: 0 Int64)) (output (: -103990105 Int64)))
