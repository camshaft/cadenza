(do (def (main (: n Int64)) (let ((q (Qty.of n (Unit.base #"meter"))) (r (Qty.of 4 (Unit.base #"meter")))) (Qty.value (+ q r)))) (export main))
