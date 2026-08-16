(case "esc2 Int32 MIN / -1 still traps (aliased-width control — the fix's other arm)"
  (input  (do
            (def (main (: k Int64))
              (Int64.of (/ ((. (Int 32) wrap) -2147483648) ((. (Int 32) wrap) k))))
            (export main)))
  (call   main (: -1 Int64)) (trap "overflow")
  (call   main (: 2 Int64)) (output (: -1073741824 Int64)))
