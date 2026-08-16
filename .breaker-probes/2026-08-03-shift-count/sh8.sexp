(case "sh8 SIGNED odd-width shift overflow (Int24: 4194304<<1)"
  (input  (do
            (def (main (: k Int64)) (Int64.of (<< ((. (Int 24) wrap) 4194304) ((. (Int 24) wrap) k))))
            (export main)))
  (call   main (: 1 Int64)) (trap "overflow")
  (call   main (: 0 Int64)) (output (: 4194304 Int64)))
