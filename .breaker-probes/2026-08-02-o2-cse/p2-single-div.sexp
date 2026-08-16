(case "p2 isolate: single division in and-rhs, computed lhs"
  (input  (do
            (def (main (: x Int64))
              (if (and (not (= x 0)) (< (/ 100 x) 5))
                  1
                  0))
            (export main)))
  (call   main (: 0 Int64)) (output (: 0 Int64)))
