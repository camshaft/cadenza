(case "pyre6 probe" (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((tick () s
        (+ (resume (handle E (: 40 Int64)
                     ((tick () t (resume t (+ t 1))))
                     (+ (E.tick) 2))
                   (* 10 s))
           (* 1000 s))))
      (+ (E.tick) (* 10 (E.tick)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 11462 Int64))
  (call   main (: 0 Int64)) (output (: 462 Int64)))
