(case "sh5 in-range odd-width shift wraps at the declared width (UInt4: 1<<3 = 8)"
  (input  (do
            (def (main (: k Int64)) (Int64.of (<< ((. (UInt 4) wrap) 1) ((. (UInt 4) wrap) k))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 8 Int64)))
