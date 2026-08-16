(case "qe2 a Qty as the handler STATE itself, advanced per perform via Qty arithmetic"
  (input  (do
            (effect Acc (op add (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Acc (Qty.of n (Unit.base #"meter"))
                ((add (v) s (resume (Qty.value s) (Qty.of (+ (Qty.value s) v) (Unit.base #"meter")))))
                (+ (Acc.add 10) (* 10 (Acc.add 100)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 155 Int64)))
