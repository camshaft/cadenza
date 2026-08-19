(case "pytn1-ctrl: inner handle replaced by literal 42 in the tail next-state"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((tick () s (resume (+ s 1) (: 42 Int64))))
      (+ (E.tick) (* 10 (E.tick)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 432 Int64))
  (call   main (: 0 Int64)) (output (: 431 Int64)))
