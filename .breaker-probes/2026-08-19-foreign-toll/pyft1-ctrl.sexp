(case "pyft1-ctrl: F.aux replaced by literal 100 in the toll"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((tick () s (+ (resume (+ s 1) (* 10 s)) (: 100 Int64))))
      (+ (E.tick) (* 10 (E.tick)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 312 Int64))
  (call   main (: 0 Int64)) (output (: 211 Int64)))
