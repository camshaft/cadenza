(case "pyts1 probe: TAIL-resumptive arm with a dispatching nested handle in the outer handle's SEED (tail sibling of the two-hole pyse1 seed decline)"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (+ (handle E (: 40 Int64)
                   ((tick () t (resume t (+ t 1))))
                   (+ (E.tick) 2))
                 (% n 3))
      ((tick () s (resume (+ s 1) (+ s 10))))
      (+ (E.tick) (* 10 (E.tick)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 584 Int64))
  (call   main (: 0 Int64)) (output (: 573 Int64)))
