(case "n2 runtime Int8 division and remainder at the MIN corner"
  (input  (do
            (def (main (: x Int8))
              (+ (Int64.of (/ x (Int8.wrap 3)))
                 (* 1000 (Int64.of (% x (Int8.wrap 3))))))
            (export main)))
  (call   main (: -128 Int8)) (output (: -2042 Int64))
  (call   main (: 100 Int8)) (output (: 1033 Int64)))
