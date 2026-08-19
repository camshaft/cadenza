(case "pylh1 probe: DISPATCHING nested handle let-hoisted before resume, value used post-resume"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((tick () s
        (let ((k (handle E (: 40 Int64)
                   ((tick () t (resume t (+ t 1))))
                   (+ (E.tick) 2))))
          (+ (resume (+ s 1) (* 10 s)) k))))
      (+ (E.tick) (* 10 (E.tick)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 196 Int64))
  (call   main (: 0 Int64)) (output (: 95 Int64)))
