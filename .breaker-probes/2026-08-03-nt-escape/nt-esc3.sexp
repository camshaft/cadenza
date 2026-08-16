(case "nt-esc3 nullary control: how does the const path render"
  (input  (do
            (type W (Mk Int64))
            (def (main) (Mk 5))
            (export main)))
  (call   main) (output (: 5 W)))
