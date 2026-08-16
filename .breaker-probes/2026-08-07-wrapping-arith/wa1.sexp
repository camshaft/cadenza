(case "wa1 wrapping-add as the next-state — a near-MAX seed wraps to a concrete near-MIN value on the second draw"
  (input  (do
            (effect W (op bump (-> Int64)))
            (def (main (: n Int64))
              (handle W 9223372036854775800
                ((bump () s (resume s (Int64.wrapping-add s n))))
                (if (= (W.bump) 9223372036854775800)
                    (if (= (W.bump) -9223372036854775806) 1 2)
                    3)))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1 Int64))
  (call   main (: 3 Int64)) (output (: 2 Int64)))
