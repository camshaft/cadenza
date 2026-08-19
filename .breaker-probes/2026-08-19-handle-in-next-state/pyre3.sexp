(case "pyre3 probe" (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((tick () s
        (+ (resume (* s 10) (handle E (: 40 Int64)
                              ((tick () t (resume t (+ t 1))))
                              (+ (E.tick) 2)))
           (* 1000 s))))
      (+ (E.tick) (* 10 (E.tick)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 47210 Int64))
  (call   main (: 0 Int64)) (output (: 46200 Int64)))
