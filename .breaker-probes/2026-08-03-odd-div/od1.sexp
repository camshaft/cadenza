(case "od1 odd-width division overflow traps at the declared width (Int24 min / -1)"
  (input  (do
            (def (main (: k Int64))
              (Int64.of (/ ((. (Int 24) wrap) -8388608) ((. (Int 24) wrap) k))))
            (export main)))
  (call   main (: -1 Int64)) (trap "integer overflow")
  (call   main (: 2 Int64)) (output (: -4194304 Int64)))
