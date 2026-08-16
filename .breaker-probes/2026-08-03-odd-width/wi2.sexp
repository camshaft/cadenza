(case "wi2 odd-width wrapping at the boundary: UInt4 wraps mod 16, Int24 wraps at ±2^23"
  (input  (do
            (def (main (: k Int64))
              (+ (Int64.of ((. (UInt 4) wrap) (+ 15 k)))
                 (* 100 (Int64.of ((. (Int 24) wrap) (+ 8388607 k))))))
            (export main)))
  (call   main (: 1 Int64)) (output (: -838860800 Int64))
  (call   main (: 0 Int64)) (output (: 838860715 Int64)))
