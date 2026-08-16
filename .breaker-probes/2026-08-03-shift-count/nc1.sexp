(case "nc1 odd-width CHECKED narrowing traps above the declared max ((. (Int 24) of) 8388608)"
  (input  (do
            (def (main (: k Int64)) (Int64.of ((. (Int 24) of) k)))
            (export main)))
  (call   main (: 8388608 Int64)) (output (: 99999 Int64))
  (call   main (: 8388607 Int64)) (output (: 8388607 Int64)))
