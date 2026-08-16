(case "rot1 odd-width wrapping MUL stays in declared range (UInt4 wrap-mul via wrap of product)"
  (input  (do
            (def (main (: k Int64))
              (Int64.of ((. (UInt 4) wrap) (* 5 k))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 9 Int64)))
