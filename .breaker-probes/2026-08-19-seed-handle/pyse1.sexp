(case "pyse1 probe: nested closed handle in the outer handle's SEED position (=42), then +n%3"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (+ (handle E (: 40 Int64)
                   ((tick () t (resume t (+ t 1))))
                   (+ (E.tick) 2))
                 (% n 3))
      ((tick () s
        (+ (resume (+ s 1) (* 10 s))
           (* 100 s))))
      (+ (E.tick) (* 10 (E.tick)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 51654 Int64))
  (call   main (: 0 Int64)) (output (: 50453 Int64)))
