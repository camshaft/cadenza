(case "pyse1-ctrl: literal 42 seed (ref-transparency control for nested-handle seed)"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (+ (: 42 Int64) (% n 3))
      ((tick () s
        (+ (resume (+ s 1) (* 10 s))
           (* 100 s))))
      (+ (E.tick) (* 10 (E.tick)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 51654 Int64))
  (call   main (: 0 Int64)) (output (: 50453 Int64)))
