(case "si1 a WHOLE inner handle expression as an outer handler's SEED"
  (input  (do
            (effect A (op tick (-> Unit Int64)))
            (effect B (op get (-> Unit Int64)))
            (def (main (: n Int64))
              (handle B
                (handle A n ((tick (u) s (resume s (+ s 3))))
                  (+ (A.tick) (* 10 (A.tick))))
                ((get (u) t (resume t (+ t 1))))
                (+ (B.get) (* 100 (B.get)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 8685 Int64))
  (call   main (: 0 Int64)) (output (: 3130 Int64))
  (call   main (: -4 Int64)) (output (: -1314 Int64)))
