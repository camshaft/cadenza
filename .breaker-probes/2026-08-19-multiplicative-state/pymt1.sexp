(case "pymt1 probe: a GEOMETRIC (tripling) state thread — each tick answers the current state and threads (* s 3), so three dispatches read s, 3s, 9s; packing them into separate digit ranges makes a wrong multiplier or a mis-threaded state visibly scramble the powers-of-three progression"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (+ (% n 3) (: 2 Int64))
      ((tick () s (resume s (* s 3))))
      (+ (E.tick) (+ (* 100 (E.tick)) (* 10000 (E.tick))))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 270903 Int64))
  (call   main (: 0 Int64)) (output (: 180602 Int64)))
