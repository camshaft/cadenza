(case "pyre7-bothholes probe: nested closed handle in BOTH resume holes (answer AND next-state) — must DECLINE (next-state hole triggers pyre3 guard)"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((tick () s
        (+ (resume (handle E (: 40 Int64)
                     ((tick () t (resume t (+ t 1))))
                     (+ (E.tick) 2))
                   (handle E (: 50 Int64)
                     ((tick () u (resume u (+ u 1))))
                     (+ (E.tick) 3)))
           (* 1000 s))))
      (+ (E.tick) (* 10 (E.tick)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 99999 Int64))
  (call   main (: 0 Int64)) (output (: 99999 Int64)))
