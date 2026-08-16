(case "dp2 a perform-seeded inner handle composed with a SECOND same-effect instantiation after it"
  (input  (do
            (effect A (op tick (-> Unit Int64)))
            (effect B (op tock (-> Unit Int64)))
            (def (main (: n Int64))
              (+ (* 100 (handle A n
                          ((tick (u) s (resume s (+ s 1))))
                          (handle B (A.tick)
                            ((tock (u) t (resume t (+ t 10))))
                            (+ (B.tock) (B.tock)))))
                 (handle A 50
                   ((tick (u) s (resume s (+ s 1))))
                   (A.tick))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2050 Int64)))
