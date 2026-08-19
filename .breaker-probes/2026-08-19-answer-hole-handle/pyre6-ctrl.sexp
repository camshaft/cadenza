(case "pyre6-ctrl referential-transparency control: inner closed handle replaced by its value 42"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((tick () s
        (+ (resume (: 42 Int64) (* 10 s))
           (* 1000 s))))
      (+ (E.tick) (* 10 (E.tick)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 11462 Int64))
  (call   main (: 0 Int64)) (output (: 462 Int64)))
