(case "cf1 the units-quantity idiom UNDER a handler: Qty arithmetic on perform results"
  (input  (do
            (effect St (op meters (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((meters (u) s (resume s (+ s 2))))
                (Qty.value (Qty.of (+ (St.meters) (St.meters)) (Unit.base #"meter")))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 12 Int64)))
