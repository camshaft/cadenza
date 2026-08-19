(case "pyth1 probe: nested closed handle (=42) in the TOLL position beside resume"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((tick () s
        (+ (resume (+ s 1) (* 10 s))
           (handle E (: 40 Int64)
             ((tick () t (resume t (+ t 1))))
             (+ (E.tick) 2)))))
      (+ (E.tick) (* 10 (E.tick)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 196 Int64))
  (call   main (: 0 Int64)) (output (: 95 Int64)))
