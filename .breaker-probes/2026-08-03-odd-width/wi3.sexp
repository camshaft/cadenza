(case "wi3 odd-width CHECKED arithmetic traps at the odd boundary (Int24 max + 1)"
  (input  (do
            (def (main (: k Int64))
              (Int64.of (+ ((. (Int 24) wrap) 8388607) ((. (Int 24) wrap) k))))
            (export main)))
  (call   main (: 1 Int64)) (trap "integer overflow")
  (call   main (: 0 Int64)) (output (: 8388607 Int64)))
