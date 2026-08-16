(case "qk2 derived-DIMENSION keys: meter/second built by division deduces with the same derived unit"
  (input  (do
            (def (main (: v Int64))
              (Set.len (Set.of (list
                (/ (Qty.of (Int64.of v) (Unit.base #"meter")) (Qty.of 2 (Unit.base #"second")))
                (/ (Qty.of (Int64.of v) (Unit.base #"meter")) (Qty.of 2 (Unit.base #"second")))
                (/ (Qty.of (+ (Int64.of v) 2) (Unit.base #"meter")) (Qty.of 2 (Unit.base #"second")))))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 2 Int64)))
