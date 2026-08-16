(case "qk1 a quantity reached via ARITHMETIC keys a Map like the directly-built quantity"
  (input  (do
            (def (main (: v Int64))
              (let ((computed (* (Qty.of (Int64.of v) (Unit.base #"meter")) (Qty.of 3 (Unit.base #"meter"))))
                    (m (Map.insert Map.empty (* (Qty.of 6 (Unit.base #"meter")) (Qty.of 1 (Unit.base #"meter"))) 42)))
                (match (Map.lookup m computed) ((Some r) r) ((None _u) -1))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 42 Int64))
  (call   main (: 3 Int64)) (output (: -1 Int64)))
