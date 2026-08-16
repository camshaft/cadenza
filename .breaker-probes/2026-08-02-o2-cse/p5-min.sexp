(case "p5 shrink: two identical divisions as = operands in and-rhs"
  (input  (do
            (def (main (: x Int64))
              (if (and (not (= x 0)) (= (/ 100 x) (/ 100 x)))
                  1
                  0))
            (export main)))
  (call   main (: 0 Int64)) (output (: 0 Int64)))
