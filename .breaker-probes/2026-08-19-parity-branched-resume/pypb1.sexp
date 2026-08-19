(case "pypb1 probe: the arm BRANCHES the whole resume (answer AND next-state) on the captured state's PARITY — even states scale-by-10 advancing +3, odd states add 100 doubling; three dispatches walk a parity-alternating thread so both branches fire and the next-state choice steers the following dispatch's parity"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((tick () s
        (if (= (% s 2) 0)
            (resume (* s 10) (+ s 3))
            (resume (+ s 100) (* s 2)))))
      (+ (E.tick) (+ (* 1000 (E.tick)) (* 1000000 (E.tick))))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 105020101 Int64))
  (call   main (: 0 Int64)) (output (: 60103000 Int64)))
