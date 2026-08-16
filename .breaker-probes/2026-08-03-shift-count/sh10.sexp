(case "sh10 odd-width SIGNED right shift preserves sign (Int24 -8 >> 1 = -4)"
  (input  (do
            (def (main (: k Int64)) (Int64.of (>> ((. (Int 24) wrap) -8) ((. (Int 24) wrap) k))))
            (export main)))
  (call   main (: 1 Int64)) (output (: -4 Int64)))
