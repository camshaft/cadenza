(case "pyfl1 probe: a FLOAT64 handler state threaded by half-step increments — tick answers the current float state and threads (+ s 0.5), so three dispatches read s, s+0.5, s+1.0 and their sum is 3s + 1.5; the seed is selected by n mod 3 among 0.0/1.0/2.0 so the float thread varies and exact half-steps stay representable"
  (input (do
  (effect E (op tick (-> Float64)))
  (def (main (: n Int64))
    (handle E (if (= (% n 3) (: 0 Int64)) (: 0.0 Float64)
                  (if (= (% n 3) (: 1 Int64)) (: 1.0 Float64) (: 2.0 Float64)))
      ((tick () s (resume s (+ s (: 0.5 Float64)))))
      (+ (E.tick) (+ (E.tick) (E.tick)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 4.5 Float64))
  (call   main (: 0 Int64)) (output (: 1.5 Float64)))
