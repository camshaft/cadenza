(case "pyts2 probe: a TUPLE-STATE FIELD SWAP — the state is a pair, each tick answers a digit-packed function of both fields (a*10+b) and threads the SWAPPED pair (b, a) as the next-state, so the two dispatches read the fields in opposite roles and an order-insensitive swap or a wrong thread flips the packed value"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (tuple (% n 3) (: 5 Int64))
      ((tick () s (match s ((tuple a b) (resume (+ (* a 10) b) (tuple b a))))))
      (+ (* 1000 (E.tick)) (E.tick))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 15051 Int64))
  (call   main (: 0 Int64)) (output (: 5050 Int64)))
