(case "sh3 runtime NEGATIVE shift count"
  (input  (do
            (def (main (: k Int64)) (<< 1 k))
            (export main)))
  (call   main (: -1 Int64)) (trap "unreachable"))
