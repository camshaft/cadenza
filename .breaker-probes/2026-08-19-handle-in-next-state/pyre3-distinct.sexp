(case "pyre3d probe" (input (do
  (effect F (op ftick (-> Int64)))
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((tick () s
        (+ (resume (* s 10) (handle F (: 40 Int64)
                              ((ftick () t (resume t (+ t 1))))
                              (+ (F.ftick) 2)))
           (* 1000 s))))
      (+ (E.tick) (* 10 (E.tick)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 47210 Int64))
  (call   main (: 0 Int64)) (output (: 46200 Int64)))
