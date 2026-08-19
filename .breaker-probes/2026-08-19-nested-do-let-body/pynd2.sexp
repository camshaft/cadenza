(case "pynd2 probe: a NESTED do-block as a LET body — (let ((k 5)) (do A (do B (+ C k)))) exercises the fn/let greedy-body position of the printer round-trip fix (6345bd197 covered handle/let/fn bodies); the let binding survives into the innermost expression while the discarded dispatches advance the handler state"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((tick () s (resume (* s 10) (+ s 1))))
      (let ((k (: 5 Int64)))
        (do (E.tick)
            (do (E.tick)
                (+ (* 100 (E.tick)) k))))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 3005 Int64))
  (call   main (: 0 Int64)) (output (: 2005 Int64)))
