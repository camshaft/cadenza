(case "cr1 catch-and-reseed — a conditionally-aborting region's value (abort payload OR normal result) seeds a SECOND handle"
  (input  (do
            (effect R (op raise (-> Int64 Int64)))
            (effect T (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle T
                (handle R 0
                  ((raise (v) u v))
                  (if (= (% n 2) 0)
                      (+ n 1)
                      (do (R.raise (* n 10)) 999)))
                ((tick () t (resume t (+ t 3))))
                (+ (T.tick) (T.tick))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 63 Int64))
  (call   main (: 4 Int64)) (output (: 13 Int64))
  (call   main (: -5 Int64)) (output (: -97 Int64)))
