(case "pyat1 probe: answer-hole dispatching nested handle with a TRANSFORMING inner handler (doubles state, two inner dispatches) = 22, distinct fold value from pyre6"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((tick () s
        (+ (resume (handle E (: 5 Int64)
                     ((tick () t (resume (* t 2) (+ t 1))))
                     (+ (E.tick) (E.tick)))
                   (* 10 s))
           (* 1000 s))))
      (+ (E.tick) (* 10 (E.tick)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 11242 Int64))
  (call   main (: 0 Int64)) (output (: 242 Int64)))
