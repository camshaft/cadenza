(case "w2 runtime Int8 wrapping-mul MIN by -1 wraps to MIN"
  (input  (do
            (def (main (: x Int8))
              (Int64.of (Int8.wrapping-mul x (Int8.wrap -1))))
            (export main)))
  (call   main (: -128 Int8)) (output (: -128 Int64))
  (call   main (: 5 Int8)) (output (: -5 Int64)))
