(case "pysh4 probe: SAME-EFFECT nested handlers where the inner handle E SHADOWS the outer — outer tick answers (* s 100) threading (+ s 1) seed n%3, inner tick answers (* s 10) threading (+ s 2) seed 50; the body performs an outer tick, then (inside the inner handle) two ticks that must bind to the INNER handler, then a final outer tick — so the two E handlers of the SAME effect coexist and the inner correctly shadows only within its body while the outer state threads across the ticks outside it"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((tick () s (resume (* s 100) (+ s 1))))
      (+ (E.tick)
         (+ (handle E (: 50 Int64)
              ((tick () s (resume (* s 10) (+ s 2))))
              (+ (E.tick) (E.tick)))
            (E.tick)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 1320 Int64))
  (call   main (: 0 Int64)) (output (: 1120 Int64)))
