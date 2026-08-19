(case "pyth1-distinct: DISTINCT-effect nested handle (=42) in the toll position"
  (input (do
  (effect E (op tick (-> Int64)))
  (effect F (op ping (-> Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((tick () s
        (+ (resume (+ s 1) (* 10 s))
           (handle F (: 40 Int64)
             ((ping () t (resume t (+ t 1))))
             (+ (F.ping) 2)))))
      (+ (E.tick) (* 10 (E.tick)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 196 Int64))
  (call   main (: 0 Int64)) (output (: 95 Int64)))
