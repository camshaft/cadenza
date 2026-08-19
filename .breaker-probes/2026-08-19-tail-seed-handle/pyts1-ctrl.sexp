(case "pyts1-ctrl: seed inner handle replaced by literal 42"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (+ (: 42 Int64) (% n 3))
      ((tick () s (resume (+ s 1) (+ s 10))))
      (+ (E.tick) (* 10 (E.tick)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 584 Int64))
  (call   main (: 0 Int64)) (output (: 573 Int64)))
