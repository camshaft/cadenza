(case "od3 odd-width division by zero traps at runtime"
  (input  (do
            (def (main (: k Int64))
              (Int64.of (/ ((. (Int 24) wrap) 100) ((. (Int 24) wrap) k))))
            (export main)))
  (call   main (: 0 Int64)) (trap "divide by zero")
  (call   main (: 3 Int64)) (output (: 33 Int64)))
