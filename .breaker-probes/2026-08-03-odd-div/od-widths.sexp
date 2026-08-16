(case "odx4 Int48 min / -1 (odd width, i64 slot)"
  (input  (do
            (def (main (: k Int64))
              (Int64.of (/ ((. (Int 48) wrap) -140737488355328) ((. (Int 48) wrap) k))))
            (export main)))
  (call   main (: -1 Int64)) (trap "integer overflow"))
(case "odx5 Int8 min / -1 (standard width control)"
  (input  (do
            (def (main (: k Int64))
              (Int64.of (/ ((. (Int 8) wrap) -128) ((. (Int 8) wrap) k))))
            (export main)))
  (call   main (: -1 Int64)) (trap "integer overflow"))
(case "odx6 Int24 SUBTRACT overflow at declared width (checked family control)"
  (input  (do
            (def (main (: k Int64))
              (Int64.of (- ((. (Int 24) wrap) -8388608) ((. (Int 24) wrap) k))))
            (export main)))
  (call   main (: 1 Int64)) (trap "integer overflow"))
