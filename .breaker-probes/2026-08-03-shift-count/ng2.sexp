(case "ng2 UNARY negation of odd-width MIN"
  (input  (do
            (def (main (: k Int64)) (Int64.of (- ((. (Int 24) wrap) k))))
            (export main)))
  (call   main (: -8388608 Int64)) (trap "overflow"))
