(case "qt1 a TWO-site arm over a Qty state (threshold gates on the unwrapped magnitude)"
  (input  (do
            (effect Acc (op feed (-> Int64 Int64)))
            (def (main (: a Int64))
              (handle Acc (Qty.of a (Unit.base #"meter"))
                ((feed (v) s (if (> v 10) (resume (+ v (Qty.value s)) (Qty.of (+ (Qty.value s) 1) (Unit.base #"meter"))) (resume 0 s))))
                (+ (* 100 (Acc.feed 20)) (+ (* 10 (Acc.feed 3)) (Acc.feed 30)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2536 Int64)))
