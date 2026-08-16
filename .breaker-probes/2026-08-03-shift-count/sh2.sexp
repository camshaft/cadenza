(case "sh2 runtime shift count above the Int64 width (65)"
  (input  (do
            (def (main (: k Int64)) (<< 1 k))
            (export main)))
  (call   main (: 65 Int64)) (trap "unreachable"))
