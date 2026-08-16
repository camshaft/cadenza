(case "fe2c isolate: float state read by abort arm, NO overflow (plain (+ s 1.0))"
  (input  (do
            (effect St (op halt (-> Unit Float64)))
            (def (main)
              (handle St 2.5
                ((halt (u) s (+ s 1.0)))
                (St.halt)))
            (export main)))
  (output (: 3.5 Float64)))
