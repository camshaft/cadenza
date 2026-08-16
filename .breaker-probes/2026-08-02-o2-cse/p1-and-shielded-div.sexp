(case "p1 CSE at O2 must not hoist a repeated division out of a short-circuit and's rhs"
  (input  (do
            (def (main (: x Int64))
              (if (and (not (= x 0)) (= (+ (/ 100 x) (/ 100 x)) 2))
                  1
                  0))
            (export main)))
  (call   main (: 0 Int64)) (output (: 0 Int64)))
