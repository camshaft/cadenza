(case "pyre6-distinct probe: DISTINCT-effect nested handle in the resume-answer hole"
  (input (do
  (effect E (op tick (-> Int64)))
  (effect F (op ping (-> Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((tick () s
        (+ (resume (handle F (: 40 Int64)
                     ((ping () t (resume t (+ t 1))))
                     (+ (F.ping) 2))
                   (* 10 s))
           (* 1000 s))))
      (+ (E.tick) (* 10 (E.tick)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 11462 Int64))
  (call   main (: 0 Int64)) (output (: 462 Int64)))
