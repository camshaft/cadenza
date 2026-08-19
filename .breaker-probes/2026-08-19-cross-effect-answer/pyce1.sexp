(case "pyce1 probe: E's tick arm answers with a value that DISPATCHES a distinct effect F (outer handler answers 100), resume answer = state + foreign draw"
  (input (do
  (effect E (op tick (-> Int64)))
  (effect F (op aux (-> Int64)))
  (def (main (: n Int64))
    (handle F (: 0 Int64)
      ((aux () fs (resume (: 100 Int64) fs)))
      (handle E (% n 3)
        ((tick () s (resume (+ s (F.aux)) (+ s 1))))
        (+ (E.tick) (* 10 (E.tick))))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 1121 Int64))
  (call   main (: 0 Int64)) (output (: 1110 Int64)))
