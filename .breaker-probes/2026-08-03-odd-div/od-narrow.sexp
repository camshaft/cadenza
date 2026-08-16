(case "odx1 Int16 min / -1 (standard narrow width)"
  (input  (do
            (def (main (: k Int64))
              (Int64.of (/ ((. (Int 16) wrap) -32768) ((. (Int 16) wrap) k))))
            (export main)))
  (call   main (: -1 Int64)) (trap "integer overflow"))
(case "odx2 Int24 MULTIPLY overflow at declared width"
  (input  (do
            (def (main (: k Int64))
              (Int64.of (* ((. (Int 24) wrap) 4194304) ((. (Int 24) wrap) k))))
            (export main)))
  (call   main (: 2 Int64)) (trap "integer overflow"))
(case "odx3 Int24 division normal case sanity"
  (input  (do
            (def (main (: k Int64))
              (Int64.of (/ ((. (Int 24) wrap) -8388608) ((. (Int 24) wrap) k))))
            (export main)))
  (call   main (: -2 Int64)) (output (: 4194304 Int64)))
