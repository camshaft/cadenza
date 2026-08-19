(case "pylh2 discriminator: NON-dispatching nested handle let-hoisted before resume (inner body constant = 7)"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((tick () s
        (let ((k (handle E (: 40 Int64)
                   ((tick () t (resume t (+ t 1))))
                   (: 7 Int64))))
          (+ (resume (+ s 1) (* 10 s)) k))))
      (+ (E.tick) (* 10 (E.tick)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 126 Int64))
  (call   main (: 0 Int64)) (output (: 25 Int64)))
