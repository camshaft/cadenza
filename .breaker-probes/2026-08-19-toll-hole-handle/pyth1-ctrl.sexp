(case "pyth1-ctrl: literal 42 toll (ref-transparency control for nested-handle toll position)"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((tick () s
        (+ (resume (+ s 1) (* 10 s))
           (: 42 Int64))))
      (+ (E.tick) (* 10 (E.tick)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 196 Int64))
  (call   main (: 0 Int64)) (output (: 95 Int64)))
