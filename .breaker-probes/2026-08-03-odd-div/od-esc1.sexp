(case "esc1 Int40 min / -1 traps post-fix (a THIRD odd width, i64 slot)"
  (input  (do
            (def (main (: k Int64))
              (Int64.of (/ ((. (Int 40) wrap) -549755813888) ((. (Int 40) wrap) k))))
            (export main)))
  (call   main (: -1 Int64)) (trap "overflow")
  (call   main (: -2 Int64)) (output (: 274877906944 Int64)))
