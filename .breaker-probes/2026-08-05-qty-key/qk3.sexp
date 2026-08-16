(case "qk3 the SAME magnitude under DIFFERENT dimensions are distinct Set elements"
  (input  (do
            (def (main (: v Int64))
              (+ (Set.len (Set.of (list
                   (Qty.of (Int64.of v) (Unit.base #"meter"))
                   (Qty.of (Int64.of v) (Unit.base #"meter")))))
                 (* 10 (Set.len (Set.of (list
                   (Qty.of (Int64.of v) (Unit.base #"second"))
                   (Qty.of (+ (Int64.of v) 1) (Unit.base #"second"))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 21 Int64)))
