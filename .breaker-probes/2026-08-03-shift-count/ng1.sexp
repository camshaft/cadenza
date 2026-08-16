(case "ng1 odd-width negation of MIN traps at the declared width (Int24)"
  (input  (do
            (def (main (: k Int64)) (Int64.of (- ((. (Int 24) wrap) 0) ((. (Int 24) wrap) k))))
            (export main)))
  (call   main (: -8388608 Int64)) (trap "overflow"))
