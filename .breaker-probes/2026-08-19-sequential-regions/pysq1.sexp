(case "pysq1 SEQUENTIAL SAME-EFFECT REGIONS WITH DIFFERENT TOLL RATES — two top-level handles over one effect run one after the other with independent seeds arms and rates, the first region's hundredfold toll and the second's two-hundredfold never cross, and a stale frame or shared arm table from the first region misprices the second"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (+ (handle E (% n 3)
                   ((tick () s (+ (resume (* s 10) (+ s 1)) (* 100 s))))
                   (E.tick))
                 (* 1000 (handle E (: 5 Int64)
                           ((tick () s (+ (resume (* s 10) (+ s 1)) (* 200 s))))
                           (E.tick)))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1050110 Int64))
  (call   main (: 0 Int64)) (output (: 1050000 Int64)))
