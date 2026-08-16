(case "g1 bare-binder guard over a STRING PARAM scrutinee"
  (input  (do
            (def (band (: s String))
              (match s ((guard t (< t "m")) 1) (_ 3)))
            (def (main (: k Int64)) (band "apple"))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1 Int64)))
