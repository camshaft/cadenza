(case "t4 pipe-or pure no effects"
  (input  (do
            (def (main (: n Int64)) (| n 8))
            (export main)))
  (call   main (: 3 Int64)) (output (: 11 Int64)))
