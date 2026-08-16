(case "esc3 odd-width MIN / -1 inside a MATCH arm selected at runtime (guard survives lowering context)"
  (input  (do
            (def (main (: k Int64) (: mode Int64))
              (match mode
                (1 (Int64.of (/ ((. (Int 24) wrap) -8388608) ((. (Int 24) wrap) k))))
                (_other 0)))
            (export main)))
  (call   main (: -1 Int64) (: 1 Int64)) (trap "overflow")
  (call   main (: -1 Int64) (: 2 Int64)) (output (: 0 Int64)))
