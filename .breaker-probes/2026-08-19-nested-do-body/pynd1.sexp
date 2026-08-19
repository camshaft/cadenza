(case "pynd1 probe: a NESTED do-block as the handler BODY (do A (do B C)) sequences four dispatches, discarding all but the final expression's value — this is the shape that broke the ML round-trip before the printer fix (6345bd197 keeps the inner block boundary); pins that the fixed printer preserves nested-do semantics and the discarded dispatches still advance the state"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((tick () s (resume (* s 10) (+ s 1))))
      (do (E.tick)
          (do (E.tick)
              (+ (* 100 (E.tick)) (E.tick))))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 3040 Int64))
  (call   main (: 0 Int64)) (output (: 2030 Int64)))
