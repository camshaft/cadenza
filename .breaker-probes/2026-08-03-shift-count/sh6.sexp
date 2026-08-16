(case "sh6 odd-width shift result exceeding the declared width (UInt4: 3<<3)"
  (input  (do
            (def (main (: k Int64)) (Int64.of (<< ((. (UInt 4) wrap) 3) ((. (UInt 4) wrap) k))))
            (export main)))
  (call   main (: 3 Int64)) (trap "overflow")
  (call   main (: 2 Int64)) (output (: 12 Int64)))
