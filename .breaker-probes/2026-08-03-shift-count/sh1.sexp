(case "sh1 runtime shift count equal to the Int64 width"
  (input  (do
            (def (main (: k Int64)) (<< 1 k))
            (export main)))
  (call   main (: 64 Int64)) (trap "unreachable"))
