(case "pyth2 discriminator: NON-dispatching nested handle in toll position (inner body performs nothing, = 7)"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((tick () s
        (+ (resume (+ s 1) (* 10 s))
           (handle E (: 40 Int64)
             ((tick () t (resume t (+ t 1))))
             (: 7 Int64)))))
      (+ (E.tick) (* 10 (E.tick)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 126 Int64))
  (call   main (: 0 Int64)) (output (: 25 Int64)))
