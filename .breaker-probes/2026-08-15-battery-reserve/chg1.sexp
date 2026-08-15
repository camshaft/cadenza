(case "chg1 a BATTERY controller with a protected reserve — charge clamps at one hundred, draw refuses any request that would dip below the seed-shaped reserve answering the negated shortfall with the level UNTOUCHED, and the same draw sequence trips the refusal at DIFFERENT points per seed with the refused row's level surviving to the next draw"
  (input  (do
            (effect C
              (op charge (-> Int64 Int64))
              (op draw (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle C (: 50 Int64)
                ((charge (v) level
                  (if (< 100 (+ level v))
                      (resume 100 100)
                      (resume (+ level v) (+ level v))))
                 (draw (v) level
                  (if (< (- level v) (+ n 10))
                      (resume (- 0 (- (+ n 10) (- level v))) level)
                      (resume (- level v) (- level v)))))
                (let ((a (C.draw 25)))
                  (let ((b (C.charge 40)))
                    (let ((c (C.draw 52)))
                      (let ((d (C.draw 30)))
                        (let ((e (C.charge 80)))
                          (let ((f (C.draw 52)))
                            (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 256493360048 Int64))
  (call   main (: 0 Int64)) (output (: 256512739341 Int64)))
