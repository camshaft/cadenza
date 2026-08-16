(case "bi3 a BIGINT handler state crosses the Int64 boundary MID-THREAD — the greater-than verdict flips exactly at the crossing dispatch"
  (input  (do
            (effect A (op bump (-> Int64)))
            (def (main (: n Int64))
              (handle A (BigInt.of 9223372036854775806)
                ((bump () s
                  (resume (if (> s (BigInt.of 9223372036854775807)) 1 0)
                          (+ s (BigInt.of 1)))))
                (+ (A.bump) (+ (* 10 (A.bump)) (* 100 (A.bump))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 100 Int64)))
