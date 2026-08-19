(case "pytn1 probe: TAIL-resumptive arm with a dispatching nested handle in the NEXT-STATE hole (tail analogue of pyre3's two-hole next-state)"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((tick () s
        (resume (+ s 1)
                (handle E (: 40 Int64)
                  ((tick () t (resume t (+ t 1))))
                  (+ (E.tick) 2)))))
      (+ (E.tick) (* 10 (E.tick)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 432 Int64))
  (call   main (: 0 Int64)) (output (: 431 Int64)))
