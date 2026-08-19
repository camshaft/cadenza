(case "pydm1 probe: resume answer packs INTEGER DIVISION and MODULO of the captured state (div*10 + mod) while the next-state advances by 7, so across three dispatches the quotient and remainder both shift and a div/mod confusion or wrong thread would scramble the packed digits"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (+ (% n 3) (: 5 Int64))
      ((tick () s (resume (+ (* (/ s 3) 10) (% s 3)) (+ s 7))))
      (+ (E.tick) (+ (* 1000 (E.tick)) (* 1000000 (E.tick))))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 62041020 Int64))
  (call   main (: 0 Int64)) (output (: 61040012 Int64)))
