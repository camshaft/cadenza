(case "odx10 UInt24 division has no MIN/-1 face (control stays green)"
  (input  (do
            (def (main (: k Int64))
              (Int64.of (/ ((. (UInt 24) wrap) 16777215) ((. (UInt 24) wrap) k))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 5592405 Int64)))
