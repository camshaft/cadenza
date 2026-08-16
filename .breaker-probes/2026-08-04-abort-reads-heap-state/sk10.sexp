(case "sk10 QTY state read by an abort arm (unit-wrapped scalar seed face)"
  (input  (do
            (effect St (op halt (-> Unit Int64)))
            (def (main (: a Int64))
              (handle St (Qty.of 5 (Unit.base #"meter"))
                ((halt (u) s (* 100 (+ (Qty.value s) a))))
                (St.halt)))
            (export main)))
  (call   main (: 2 Int64))
  (output (: 700 Int64)))
