(case "sh12 odd-width RIGHT-shift count at the declared width traps (Int24 >> 24)"
  (input  (do
            (def (main (: k Int64)) (Int64.of (>> ((. (Int 24) wrap) 100) ((. (Int 24) wrap) k))))
            (export main)))
  (call   main (: 24 Int64)) (trap "unreachable")
  (call   main (: 2 Int64)) (output (: 25 Int64)))
