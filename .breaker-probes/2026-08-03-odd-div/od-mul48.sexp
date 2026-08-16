(case "odx9 Int48 multiply overflow control"
  (input  (do
            (def (main (: k Int64))
              (Int64.of (* ((. (Int 48) wrap) 70368744177664) ((. (Int 48) wrap) k))))
            (export main)))
  (call   main (: 2 Int64)) (trap "integer overflow"))
