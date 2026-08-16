(case "sh4 shift count equal to the DECLARED width (UInt4 << 4)"
  (input  (do
            (def (main (: k Int64)) (Int64.of (<< ((. (UInt 4) wrap) 1) ((. (UInt 4) wrap) k))))
            (export main)))
  (call   main (: 4 Int64)) (trap "unreachable"))
