(case "g4 bare-binder guard over an INT param scrutinee in a helper (the #46 shape, control)"
  (input  (do
            (def (band (: q Int64))
              (match q ((guard w (> w 10)) 1) (_ 0)))
            (def (main (: x Int64)) (band x))
            (export main)))
  (call   main (: 15 Int64)) (output (: 1 Int64)))
