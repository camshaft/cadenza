(case "pf2 a wrapped narrow result drives a BRANCH condition at the wrapped value"
  (input  (do
            (def (main (: x Int8))
              (if (< (Int8.wrapping-add x (Int8.wrap 1)) (Int8.wrap 0)) 1 0))
            (export main)))
  (call   main (: 127 Int8)) (output (: 1 Int64))
  (call   main (: 5 Int8)) (output (: 0 Int64)))
