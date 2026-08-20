(case "pyu8w1 probe: a UInt8 handler state near the 255 boundary threaded by UInt8.wrapping-add — tick answers the widened state (Int64.of) and threads (UInt8.wrapping-add s 5), so across three dispatches the state WRAPS past 255 back through zero; packing the three widened reads makes a wrong wrap or a widened-instead-of-wrapped thread visible"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (UInt8.wrapping-add (: 250 UInt8) (UInt8.wrap (% n 3)))
      ((tick () s (resume (Int64.of s) (UInt8.wrapping-add s (: 5 UInt8)))))
      (+ (* 1000000 (E.tick)) (+ (* 1000 (E.tick)) (E.tick)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 251000005 Int64))
  (call   main (: 0 Int64)) (output (: 250255004 Int64)))
