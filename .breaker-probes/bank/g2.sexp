(case "g2 bare-binder guard over a string scrutinee in MAIN directly"
  (input  (do
            (def (main (: k Int64))
              (match "apple" ((guard t (< t "m")) 1) (_ 3)))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1 Int64)))
