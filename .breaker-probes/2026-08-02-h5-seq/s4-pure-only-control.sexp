(case "s4 pure-only control: no host call in the do, trap elided"
  (input  (do
            (def (main (: d Int64))
              (do (/ 100 d)
                  42))
            (export main)))
  (call   main (: 0 Int64)) (output (: 42 Int64)))
