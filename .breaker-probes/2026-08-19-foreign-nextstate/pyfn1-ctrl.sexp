(case "pyfn1-ctrl: F.aux replaced by literal 40 in the next-state"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((tick () s (+ (resume (+ s 1) (: 40 Int64)) (* 1000 s))))
      (+ (E.tick) (* 10 (E.tick)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 41412 Int64))
  (call   main (: 0 Int64)) (output (: 40411 Int64)))
