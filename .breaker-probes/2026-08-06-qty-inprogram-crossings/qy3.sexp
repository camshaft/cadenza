(case "qy3 a Qty state's next-state slot computes (+ s s) INLINE — the formerly-rejected shape runs"
  (input  (do
            (effect Acc (op step (-> Unit (Qty Int64 (Unit.base #"meter")))))
            (def (main (: n Int64))
              (handle Acc (Qty.of n (Unit.base #"meter"))
                ((step (u) s (resume s (+ s s))))
                (Qty.value (+ (Acc.step) (Acc.step)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 15 Int64)))
