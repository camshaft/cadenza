(case "q6 an erased Qty add is still CHECKED at the inner width (overflow traps)"
  (input  (do
            (def (main (: v Int64))
              (let ((q (Qty.of v (Unit.base #"meter"))))
                (Qty.value (+ q q))))
            (export main)))
  (call   main (: 4611686018427387904 Int64)) (trap "integer overflow")
  (call   main (: 21 Int64)) (output (: 42 Int64)))
