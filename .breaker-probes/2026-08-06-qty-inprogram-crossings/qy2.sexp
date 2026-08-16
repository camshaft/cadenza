(case "qy2 a Qty STATE threads via the def-workaround arm — the documented accept-shape runs end to end"
  (input  (do
            (effect Acc (op step (-> Unit (Qty Int64 (Unit.base #"meter")))))
            (def (main (: n Int64))
              (handle Acc (Qty.of n (Unit.base #"meter"))
                ((step (u) s (do (def t (+ s s)) (resume s t))))
                (Qty.value (+ (Acc.step) (Acc.step)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 15 Int64)))
