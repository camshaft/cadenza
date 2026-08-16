(case "w7 the wrapped narrow result compares correctly at its own width"
  (input  (do
            (def (main (: x Int8))
              (if (= (Int8.wrapping-add x (Int8.wrap -1)) (Int8.wrap 127)) 1 0))
            (export main)))
  (call   main (: -128 Int8)) (output (: 1 Int64)))
