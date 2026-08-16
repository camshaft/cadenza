(case "w4 runtime Int8 wrapping-add and wrapping-sub at the corners"
  (input  (do
            (def (main (: x Int8))
              (+ (Int64.of (Int8.wrapping-add x (Int8.wrap -1)))
                 (* 1000 (Int64.of (Int8.wrapping-sub x (Int8.wrap 1))))))
            (export main)))
  (call   main (: -128 Int8)) (output (: 127127 Int64)))
