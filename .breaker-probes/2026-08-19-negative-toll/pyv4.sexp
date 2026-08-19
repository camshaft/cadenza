(case "pyv4 SUBTRACTED TOLLS DRIVE THE WHOLE ANSWER NEGATIVE — each frame SUBTRACTS its hundredfold toll from the resumed value so the unwind runs the fold below zero and keeps going, both seeds land negative with the seeded frame subtracting deeper, and any unsigned slot or clamped intermediate in the unwind path shows immediately"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s
                  (- (resume (* s 10) (+ s 1)) (* 100 (+ s 1)))))
                (+ (E.tick) (* 10 (E.tick)))))
            (export main)))
  (call   main (: 10 Int64)) (output (: -290 Int64))
  (call   main (: 0 Int64)) (output (: -200 Int64)))
