(case "qe1 a QUANTITY resume value through a STATEFUL arm (corpus pins the stateless face at 14-eff:3102)"
  (input  (do
            (effect Src (op read (-> Unit (Qty Int64 (Unit.base #"meter")))))
            (def (main (: n Int64))
              (handle Src 0
                ((read (u) s (resume (Qty.of (+ n s) (Unit.base #"meter")) (+ s 1))))
                (+ (Qty.value (Src.read)) (* 10 (Qty.value (Src.read))))))
            (export main)))
  (call   main (: 25 Int64)) (output (: 285 Int64)))
