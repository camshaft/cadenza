(case "pybf1 probe: op flag(b) carries a BOOL argument and the arm branches the whole resume on it — true scales the state advancing +1, false adds a hundred doubling; two dispatches pass true then false so both branches fire and the next-state choice differs per branch"
  (input (do
  (effect E (op flag (-> Bool Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((flag (b) s (if b (resume (* s 10) (+ s 1)) (resume (+ s 100) (* s 2)))))
      (+ (* 100 (E.flag true)) (E.flag false))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 1102 Int64))
  (call   main (: 0 Int64)) (output (: 101 Int64)))
