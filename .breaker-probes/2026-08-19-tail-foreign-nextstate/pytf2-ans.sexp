(case "pytf2-ans probe: TAIL arm with a bare foreign perform in the ANSWER hole folds (contrast: same foreign perform in the next-state hole declines)"
  (input (do
  (effect E (op tick (-> Int64)))
  (effect F (op aux (-> Int64)))
  (def (main (: n Int64))
    (handle F (: 0 Int64)
      ((aux () fs (resume (: 40 Int64) fs)))
      (handle E (% n 3)
        ((tick () s (resume (+ s (F.aux)) (+ s 1))))
        (+ (E.tick) (* 10 (E.tick))))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 461 Int64))
  (call   main (: 0 Int64)) (output (: 450 Int64)))
