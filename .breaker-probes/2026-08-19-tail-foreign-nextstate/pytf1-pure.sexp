(case "pytf1-pure: tail next-state = pure literal 40 (no foreign perform)"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((tick () s (resume (+ s 1) (: 40 Int64))))
      (+ (E.tick) (* 10 (E.tick)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 412 Int64))
  (call   main (: 0 Int64)) (output (: 411 Int64)))
